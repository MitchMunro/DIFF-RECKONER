# AGENTS.md

diff-reckoner is a Rust TUI (ratatui) diff reviewer: it shows a git diff, takes line
comments, and exports them for an agent to address. One binary, one git worktree.

Forked from [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr) (MIT). The design
this fork is working towards lives in `diff-reckoner-design-doc.md`; read it before adding
behaviour.

## Commands

- `just test` — full test suite. Single test: `cargo test <name>` (unit tests live beside the
  code, integration tests in `tests/`: `cargo test --test app_flow <name>`).
- `just lint` — clippy with warnings as errors. `just fmt` / `just fmt-check` — rustfmt.
- `just ci` — exactly what CI runs (fmt-check, lint, test, release build).
- `just smoke-edit` — PTY smoke test of the editor path (`e`) against a real release binary.
  Run it after any change to `run_editor`, the terminal mode stack, or the editor dialects.
- `python3 scripts/bench_tui.py --binary target/release/diff-reckoner --fixture` —
  perceived-latency benchmark (keypress → painted frame, via PTY). The one committed baseline
  is `scripts/bench-results/baseline.json`.

## Invariants

Inherited from reviewr and still load-bearing. Cite them by name:

- **No writes (today)**: the reviewer does not mutate the worktree, index, or branches. Its
  only git writes are private refs under `refs/worktree/reviewr/`. The design doc's in-file
  comment model will change this deliberately — nothing else should.
- **Comments survive**: comments are never lost to a refresh or an agent's edits.
- **Continuity**: place state (cursor, scroll, tab, scope, folds, selection, layout) moves
  only under the user's own input. World events (polls, refreshes) may only *reconcile* it:
  match by identity first (path, comment author+anchor — never row index), fall back to the
  nearest surviving target, clamp last. Derived state on screen may be stale, never wrong.

## Architecture

The runtime is a single-threaded frame loop (`event_loop` in `src/lib.rs`): draw → wait for
input or poll deadline → mutate `App` → draw. Clipboard and per-file diff builds run
synchronously between frames, and a terminal editor holds the loop for its whole session by
design (`policies/ux-responsiveness.md`). Two things run on worker threads: the world worker
(`src/world.rs`) and the search worker (`src/search.rs`). World results land through
`land_world_completion`: input-tagged, latest-wins, reconciled only while the view still
matches.

- `src/app.rs` — the `App` state machine. Tabs (`Changes`/`AllFiles`), scopes
  (`Uncommitted`/`Branch`/`Commits`), `Focus` (files vs diff pane), `Mode` (`Normal`, the
  `Composing`/`List` overlays, the pickers, and the body-replacing `Search` screen).
  `reconcile_world()` is the one place a world snapshot touches place state.
- `src/world.rs` — the world worker: the pure snapshot build (`WorldInput` → `WorldSnapshot`)
  and the request/completion channels (latest-wins by generation).
- `src/git.rs` — every git subprocess.
- `src/diff.rs` — `FileDiff` build (syntect highlight both sides, similar-line pairing, word
  emphasis, folds) and `DiffCache`, keyed by path and gated by content hash.
- `src/ui.rs` — all rendering.
- `src/model.rs` — `CommentStore` (in-memory today; the design doc replaces this with the
  file itself), comment anchoring.
- `src/editor.rs` — the editor command: a name-keyed dialect table and the `editor` key's
  `{file}`/`{line}` template. `run_editor` in `lib.rs` owns the spawn.
- `src/export.rs` — comment export: format all, copy to the clipboard.
- `src/config.rs` — the config file boundary, inherited from reviewr's plugin config. The
  design doc puts a config file out of scope; this is leftover surface, not a commitment.

## Known leftovers from the strip

- `src/git.rs` still carries the forge remote-identity helpers (`RepositoryIdentity`,
  `classify_remote`, `remote_identities` and friends) and `src/config.rs` still parses
  `github_host` / `gitlab_host` / `azure_devops_host`. Nothing reads them. They go when the
  config surface is revisited.
- `BaseChoice::pr_base` is always `false` now; the base picker's first sort key is inert.
