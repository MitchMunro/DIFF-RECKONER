# diff-reckoner

A standalone terminal diff reviewer. You read a change, drop review comments into the code,
and hand the lot to an agent to address.

Forked from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr) by Dmitry Persiyanov
(MIT). This fork removes the herdr coupling and the PR/forge integration; the review UI,
mouse handling, syntax highlighting and diff engine are his work.

## Status

Early. This commit is the strip: the fork with herdr, the PR tab, the `last turn` scope and
the agent-send export removed. The four features the design doc calls for — in-file
`[- REVIEW -]` comments, the whole-repo comment sweep, the Comments tab, and the
terminal-palette colour model — are not built yet. See
`diff-reckoner-design-doc.md`.

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
