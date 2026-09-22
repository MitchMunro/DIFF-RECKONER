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

## Licence

MIT. See [LICENSE](LICENSE) — the copyright notice is Dmitry Persiyanov's and stays.
