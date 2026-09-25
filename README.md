# diff-reckoner

A standalone terminal diff reviewer. You read a change, drop review comments into the code,
and hand the lot to an agent to address.

Forked from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr) by Dmitry Persiyanov
(MIT). This fork removes the herdr coupling and the PR/forge integration; the review UI,
mouse handling, syntax highlighting and diff engine are his work.

## Status

Early. The fork has herdr, the PR tab, the `last turn` scope and the agent-send export
removed, and comments are written into the files themselves. Still to come from the design
doc: the whole-repo comment sweep and the terminal-palette colour model.
See `diff-reckoner-design-doc.md`.

## Comments

`c` writes a comment into the file, directly above the line under the cursor, as a line
comment in the file's own syntax. Every line of it carries the tag:

```rust
// [- REVIEW -] this lock is held across an await
```

The file is the only store: `enter` rewrites a comment in place, `d` removes its lines, and a
comment an agent deletes leaves the list on the next refresh. A comment on a removed line goes
above the nearest surviving line and starts with `[DELETED: (...)]`, quoting the removed
line's first 16 characters. Markdown and plain text files take the tag on a bare line with no
comment marker. Files with no line-comment syntax (JSON, CSV) refuse a comment, and a
git-ignored file takes one only on a second `c`. `y` copies every comment to the clipboard;
`x` writes them to `.diff-reckoner/review.md` (a directory that git-ignores itself) and opens
it in your default app for `.md` files, for a shell with no clipboard: point the agent at the
path. Both leave the comments in place.

`enter` on a file in the file list moves into the diff (on a directory it expands or collapses
it), and `esc` moves back.

`3` opens the Comments tab: every comment in the repo, whatever the scope, as a card with five
lines of the file either side. In a file with no line-comment syntax (JSON, HTML, CSS, CSV), a
line that starts with the tag is a comment too, so it shows, edits, and deletes like the rest
(`c` still refuses to write a new one there). The navigator lists each commented file with its comments under
it. Selecting a comment opens it for editing in its card: click it (in the navigator or the
stack), or step with `j`/`k` and `}`/`{` and press `enter`. While it is open, `↑`/`↓`
run on past the box to the neighbouring card, and moving to another comment saves this one;
`esc` reverts it and closes the box. With no box open, `d` deletes the selected card and `f`
(or a click on a card's `path:line`) opens it in the Files tab with the cursor on it.

## Build

```
cargo build --release
cargo run              # review the current repo
```

Requires git on `PATH`, a truecolor terminal, macOS or Linux.

## Scopes

| Scope | Diff |
|---|---|
| `Uncommitted` | worktree vs HEAD, including untracked |
| `Branch` | vs merge-base with the base branch |
| `Commits` | a picked contiguous run, `A^`..`B` |

The Changes tab shows the whole file, changes in place. `a` switches to the changed regions
only, each unchanged stretch folded to a `▸ N unmodified lines hidden` marker. `enter` or a
click on it opens the lines below it, and the marker then reads `▾ N unmodified lines shown`;
`enter` again hides them. `a` again shows every line, and the folds you opened stay open.
`whole_file = false` in `config.toml` starts folded. `]`/`[` step between changes either way.

## Themes

The theme paints the whole window, background included. The terminal's background is detected
at startup, and the dark or light theme follows it.
Press `t` to pick them: the dark themes on the left, the light on the right. `↑`/`↓` move and
`←`/`→` switch sides, previewing each theme as you go; `enter` saves the highlighted theme
(✓) for its side; `t` or `esc` closes and returns to the saved theme.

`follow terminal`, above both lists, uses the terminal's own colors instead: its background,
text, and sixteen ANSI colors. How that looks depends on the terminal's color scheme. Saving it
pins `theme = "terminal"`; saving a dark or light theme afterwards unpins it.

The picker writes `config.toml` in `$DIFF_RECKONER_CONFIG_DIR`, else
`$XDG_CONFIG_HOME/diff-reckoner` (`~/.config/diff-reckoner`). You can also edit it by hand;
the names are the built-in list (`CATALOG` in `src/theme.rs`):

```toml
dark_theme = "cobalt2"        # default: catppuccin
light_theme = "xcode-light"   # default: catppuccin-latte
# theme = "nord"              # pin one theme regardless of background, or "terminal"
```

`--theme <name>` overrides both for one run.

## Licence

MIT. See [LICENSE](LICENSE) — the copyright notice is Dmitry Persiyanov's and stays.

Theme palettes ported from terminal color schemes come from
[iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes) (MIT); each theme's
copyright stays with its author.
