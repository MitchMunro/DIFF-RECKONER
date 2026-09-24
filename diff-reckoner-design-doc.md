# Diff Reckoner — design doc

**Status:** design agreed 2026-09-22. Implemented: §1.1 (the strip), §3 (in-file comments),
§5.3 (the Comments tab), §6 (outstanding-comment indicators).

A standalone terminal diff reviewer. You read a change, drop review comments
into the code, and hand the lot to an agent to address. Forked from
[herdr-reviewr](https://github.com/persiyanov/herdr-reviewr) (MIT,
Dmitry Persiyanov), which this replaces the herdr coupling of.

---

## 1. Why fork

reviewr already is ~90% of this tool: mouse support, draggable pane divider,
syntect highlighting, a scope model, comment capture, clipboard export. Its
herdr coupling is confined to one file. The changes wanted here are four
features, not a new architecture.

Cost accepted: ~29k lines of Rust with three files at 48% of it
(`app.rs` 5,680 lines, `ui.rs` 4,771, `lib.rs` 3,577), and either ongoing
merge work against a fast-moving upstream or deliberate divergence.

Rejected: building fresh. Re-earning mouse drag, pane splitting and diff
rendering to avoid working inside a large file is a bad trade.

### 1.1 First commit — strip

- `src/herdr.rs`, `herdr-plugin.toml`, `herdr/install.sh`
- The PR tab, and with it `src/forge.rs`, `src/gitlab.rs`,
  `src/azure_devops.rs`, `pulldown-cmark`
- The `LastTurn` scope (it polls `herdr agent list`; nothing replaces it)
- The `Agent` export target

Rename the crate and binary to `diff-reckoner`. Keep the MIT LICENSE and
copyright notice, and credit persiyanov in the README.

### 1.2 Inherited unchanged

Mouse handling including divider drag and gutter-click-to-comment, the
resize-coalescing redraw, syntect + two-face highlighting, `similar`-based
content diffing, the navigator/reader split with configurable position, the
2 s poll, the `r` manual refresh, and the scope chip.

---

## 2. Scopes

Three, cycled by the clickable header chip, as inherited:

| Scope | Diff |
|---|---|
| `Uncommitted` | worktree vs HEAD, including untracked |
| `Branch` | vs merge-base with the base branch |
| `Commits` | a picked contiguous run, `A^`..`B` |

Scope selects which files the **Changes** tab lists. It does not filter the
Comments tab (§5.3).

---

## 3. Comments

### 3.1 The model

A comment is written **directly into the source file** as a real code comment,
in whatever syntax the language uses. Every line of a comment carries the tag:

```rust
// [- REVIEW -] this lock is held across an await
// [- REVIEW -] which will deadlock under the retry path
```

Consecutive tagged lines form one comment. One tag on every line means the
parser is a line filter, the format is greppable, and an agent stripping them
needs no block logic.

**The file is the only store.** There is no sidecar database, no JSON, no
in-memory authority. On load and on every poll, the comment list *is* the
result of scanning for the tag. This survives restart, survives an agent
deleting some of them, and can never drift out of sync — because there is
nothing to drift from.

Consequences, all accepted:

- Editing a comment rewrites those lines in place, immediately, on disk.
- Deleting one removes the lines entirely, restoring the file exactly.
- There is no undo beyond git.
- Comments are **not** consumed on export. They persist until something
  removes them from the file — normally the agent you handed them to.

### 3.2 Comments are ordinary changes

A tag line is an inserted line. It appears in the diff as an addition, counts
toward `+/-`, and is a valid jump-to-change target. No special-casing anywhere.

This was contested and settled deliberately. Hiding lines that genuinely exist
in the file is a lie the renderer would have to maintain in every view. The
diff is recomputed from file contents on every refresh anyway, so a comment
insert is the normal path, not a special case. The cursor needs no re-anchoring
either: the comment is inserted directly above the line you were on, so the row
index you already held now holds the comment.

### 3.3 Deleted lines

A removed line has no home in the working tree, and git has no line identity to
reference it by — only `<oldrev>:<path>:N`, which breaks on the next change.

So a comment on a deleted line anchors to the **nearest surviving line** in the
new file, and declares itself on its first line:

```rust
// [- REVIEW -] [DELETED: (let ok = validate_t)] this check was load-bearing
// [- REVIEW -] the retry path depends on it
```

`[DELETED: (...)]` carries the first 16 characters of the removed line,
verbatim, in parentheses. First line only. This keeps one source of truth and
loses nothing — the agent gets the removed code and its location.

**Wholly deleted files** are refused. There is no file to write into.

### 3.4 Where a comment may not go

- **No line-comment syntax** (JSON, CSV) — refuse, and say so in the status bar.
  Writing an invalid line into a file to hold a note breaks the build to store a
  thought. Prose files (Markdown, plain text) are the exception: they are just text, so
  the tag goes on a bare line with no comment marker (`[- REVIEW -] reword this`). A tag line
  already in such a file is read the same way: a line the tag starts is a comment, and can be
  edited and deleted; only writing a new one is refused.
- **Git-ignored paths** — allowed, but behind a confirm prompt. `git check-ignore`
  is a fact, not a heuristic, so this has no false positives. A
  "generated file" heuristic (`// Code generated by`, `.min.js`, `vendor/`)
  was considered and rejected for now: a prompt you learn to dismiss reflexively
  is worse than no prompt.

### 3.5 Comment syntax table

A small built-in table keyed on file extension. Line comments only, and no marker at all
for prose files. `#` as the fallback. Extensions with no line comment are the refusal case
in §3.4.

---

## 4. Discovery

Two passes, both feeding the same list:

| Pass | Interval | Mechanism | Covers |
|---|---|---|---|
| Fast | 2 s (the existing poll) | `git grep -n "\[- REVIEW -\]"` over changed files | the files you're working in |
| Sweep | 60 s | directory walk (`ignore` crate, ignores **disabled**) | the whole repo, including git-ignored paths |

The sweep exists to catch comments left somewhere unexpected — a vendored file,
a build output, somewhere you didn't mean to comment. They surface within a
minute rather than never. It needs a real walk because `git grep` cannot see
ignored files.

---

## 5. Tabs

### 5.1 Changes

Lists **only files with changes** in the navigator. The reader pane shows the
selected file.

**Whole-file view** is on by default. When on, the entire file is rendered and
scrollable, with no folding. When off, only the changed regions show, with
reviewr's existing fold/expand behaviour for unchanged stretches. `w` toggles.

`]` and `[` jump to the next and previous change within the file.

### 5.2 All files

The whole-repo file browser, unchanged from reviewr. Ignored paths dimmed,
change dots on collapsed folders. Kept because browsing the codebase is a
distinct job from reviewing a change — the two tabs differ in what the
navigator lists, not in the reader pane.

### 5.3 Comments

Every comment found anywhere in the repo, regardless of the current scope. A
comment stranded in a file the current branch never touched is exactly the one
you would otherwise forget.

Each entry shows the comment with **5 lines of context either side**, and is
editable and deletable in place (writing straight through to the file, §3.1).
Clicking an entry jumps to it in the Changes tab for full context.

The fixed context window is used rather than the enclosing function name:
reviewr computes diffs from file contents via `similar` rather than parsing
git's hunk headers, so the function name is not free here the way it was in
lazygit-diff-reckoner.

---

## 6. Outstanding-comment indicators

Comments are not consumed on export, so "what is still outstanding" is the
status that matters and it must be visible without opening a tab.

- **Tab bar** — a count on the Comments tab, e.g. `3 Comments (7)`. Primary.
- **Navigator** — dots on files that contain comments. Tells you *where*.
- **Status bar** — not used for this. It is for transient messages
  ("copied 7 comments to clipboard", "no line-comment syntax for .json").

Because discovery re-scans the files (§4), an agent stripping tags makes the
count drop and the dots clear on its own, with no extra code.

---

## 7. Colour

Two layers, handled differently.

**Chrome** — borders, tab bar, gutter, selection, footer — uses
`Color::Reset` (SGR 39/49) and the 16 named ratatui variants. The terminal
resolves these at draw time, so any palette or light/dark switch applies
instantly with no code. This is lazygit's approach and it is never wrong.

**Syntax highlighting** keeps syntect, with the theme chosen by detected
background:

1. Probe at startup, **before `enable_raw_mode()`** — `COLORFGBG` first, then
   an OSC 11 query with a ~200 ms timeout, defaulting to dark on failure.
2. Enable DEC private mode 2031 (`CSI ? 2031 h`). The terminal then pushes an
   unsolicited notification when the palette changes; re-query and swap themes.

The probe ordering is not optional. A probe that reads stdin after the event
loop starts races it — the documented failure (`ratatui-image#202`) is a parked
read stealing the next keypress and flipping the terminal to cooked mode
mid-session while ratatui still believes raw mode is on.

Mode 2031 is supported by Ghostty, kitty, Contour, VTE, tmux and Zellij, and
declined by Windows Terminal. Windows Terminal is **not handled** — leave a
`NOTE:` at the detection site recording that it degrades to startup-only there.

---

## 8. Export

Two keys, two targets, both non-consuming:

- **Clipboard** — `pbcopy` / `wl-copy` / `xclip` / `xsel`, first available.
- **Write to file** — under the repo, then open it. Containers and remote
  shells frequently have no clipboard tool, and a path inside the repo is
  visible to both the TUI and the editor it launches.

Format is reviewr's, unchanged: one block per comment as
`path:start-end`, the verbatim snippet, then the comment text; blocks sorted by
file then line and joined by a blank line. Comment text is normalised (CRs
dropped, trailing space trimmed, blank lines removed) so it cannot forge the
block separator.

**What the export is for.** The agent does not need it — it can grep
`[- REVIEW -]` and get every comment with perfect context. The export is a
paste-to-agent shortcut: it exists so you can say "address these" in one
action, and so you can read every comment at once. The agent works from the
files.

---

## 9. Refresh

Unchanged from reviewr: the 2 s poll rebuilds the changed-file set, and `r`
forces it. No filesystem watcher — neither reviewr nor lazygit uses one.

What matters here is what reviewr already does correctly and must not be
broken: the file cursor is restored by **path anchor** rather than row index, a
poll deliberately does not reveal the cursor or disturb wheel scroll, diff
reload is deferred entirely while a modal or mouse drag is live, and a
content-hash cache means an unchanged file is not re-highlighted.

---

## 10. Deliberately out of scope

- Any herdr integration, including agent send and turn-status scopes
- The PR/MR tab and all forge integration
- Comment threading, authorship, or resolution state — this is a solo tool and
  the export is the only handoff
- A config file. It is the first thing that makes §7 negotiable, and that is
  not wanted yet.
- A generated-file heuristic (§3.4)
- Windows Terminal live theme switching (§7)

---

## 11. Known capability loss vs reviewr

reviewr tracks a comment `side` of New or Old and can comment on removed lines
directly. Writing into the file makes that impossible. §3.3 recovers most of it
via the nearest-surviving-line anchor and the `[DELETED: ...]` tag; wholly
deleted files are simply not commentable.
