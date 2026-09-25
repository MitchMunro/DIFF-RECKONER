//! diff-reckoner — a standalone terminal diff reviewer.
//!
//! Browse a change (uncommitted / branch / commits), leave review comments, and hand
//! them to an agent. Forked from herdr-reviewr by Dmitry Persiyanov (MIT).
//!
//! This crate is split into a thin binary (`src/main.rs`) and this library so the
//! interaction logic in [`app`] stays terminal-free and unit-testable. This module
//! owns the terminal lifecycle and the event loop; it maps input events onto
//! [`app::App`] methods and renders with [`ui`].

pub mod app;
pub mod browser;
pub mod comments_tab;
pub mod config;
pub mod diff;
pub mod editor;
pub mod export;
pub mod file_list;
pub mod git;
pub mod highlight;
pub mod keymap;
#[macro_use]
pub mod log;
pub mod markdown;
pub mod model;
pub mod proc;
pub mod review;
pub mod search;
pub mod selection;
pub mod snippet;
pub mod theme;
pub mod ui;
pub mod world;

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::crossterm::{cursor, execute};
use ratatui::layout::Rect;

use std::process::Stdio;

use crate::app::{App, Focus, Mode};
use crate::config::{Config, PluginConfig};
use crate::export::Clipboard;
use crate::keymap::Keymap;
use crate::model::Scope;

/// Entry point: parse config, set up the terminal, run the loop, restore.
pub fn run() -> Result<()> {
    let mut cfg = Config::from_env();
    log::init();
    // The config directory resolves once, at startup; every later read rereads only the
    // file inside it.
    cfg.plugin_config_dir = config::resolve_config_dir(|| None);
    // Before `ratatui::init` claims raw mode: the probe reads the tty itself.
    theme::detect_appearance();
    let initial_config = config::plugin_config(cfg.plugin_config_dir.as_deref());
    let mut app = app_for(&cfg, &initial_config);

    let mut terminal = ratatui::init();
    // The kitty keyboard protocol reports modifiers on keys the legacy encoding drops — most
    // notably Ctrl/Alt+arrows — so word-jump by arrow works where the terminal supports it.
    let kbd = supports_keyboard_enhancement().unwrap_or(false);
    logln!("keyboard enhancement supported={kbd}");
    // `ratatui::init` already claimed the alternate screen and raw mode, so only the input
    // modes are left to claim here.
    claim_input_modes(kbd);
    // Render before the first load, so a slow, failing, or hung `git` scan shows the UI
    // instead of a blank screen. Paint the empty frame first; then the initial load,
    // non-fatal — an error opens with the reason in the status line, the same contract as a
    // failed poll refresh.
    if let Err(error) = terminal.draw(|f| ui::render(f, &app)) {
        restore_terminal(kbd);
        return Err(error.into());
    }
    if initial_config.is_ok()
        && let Err(e) = app.reload()
    {
        logln!("startup reload failed: {e:#}");
        app.status = format!("load failed: {e}");
    }
    event_loop(&mut terminal, &mut app, &cfg, kbd)
}

/// Claim the input modes the event loop reads, on a screen something else already owns.
///
/// Bracketed paste so a multi-line paste arrives as one event, not raw keystrokes whose
/// embedded newlines would submit the comment early. The kitty keyboard protocol reports
/// modifiers on keys the legacy encoding drops, most notably Ctrl/Alt+arrows.
fn claim_input_modes(kbd: bool) {
    let _ = execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste, cursor::Hide);
    if kbd {
        let _ = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

/// Release what [`claim_input_modes`] claimed.
fn release_input_modes(kbd: bool) {
    if kbd {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture, cursor::Show);
}

/// Claim the screen as well as the input modes. The exact inverse of [`release_terminal`], for
/// the one caller that hands the whole terminal to another program and takes it back
///
/// Startup does not use this pair: `ratatui::init` already owns the alternate screen and raw
/// mode there, and claiming either twice is not the no-op it looks like.
fn claim_terminal(kbd: bool) {
    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), EnterAlternateScreen);
    claim_input_modes(kbd);
}

/// Release everything [`claim_terminal`] claimed, leaving a plain terminal another program can
/// own outright.
fn release_terminal(kbd: bool) {
    release_input_modes(kbd);
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Leave the alternate screen and release terminal input modes before any bounded worker drain.
fn restore_terminal(kbd: bool) {
    release_input_modes(kbd);
    ratatui::restore();
}

/// Take back the input stream an external program left behind.
///
/// Its exit can leave bytes queued, and its own teardown terminal queries answer in exactly
/// those bytes, so anything still buffered belongs to it and not to the review. A resize is the
/// terminal's rather than the program's, so it is answered instead of dropped.
///
/// The timeout is non-zero because those replies are still in flight when the editor exits: a
/// zero-timeout check sees only what has already arrived at the descriptor, which on a fast
/// return is nothing. The deadline bounds a terminal that keeps talking.
fn drain_input(app: &mut App) -> Result<()> {
    let deadline = Instant::now() + Duration::from_millis(50);
    let mut resized = false;
    while Instant::now() < deadline && event::poll(Duration::from_millis(5))? {
        resized |= matches!(event::read(), Ok(Event::Resize(_, _)));
    }
    if resized {
        handle_resize(app);
    }
    Ok(())
}

/// Service one `edit` request that named a file.
///
/// A terminal editor owns the pane outright: it suspends down to a plain terminal, and the
/// return rebuilds the same mode stack and refreshes the changeset. A window editor is opened
/// and forgotten, so the pane never moves and the poll shows the writes like any other change
/// to the worktree (Continuity). diff-reckoner itself still writes nothing.
fn run_editor(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    configured: Option<&str>,
    kbd: bool,
    open: &mut Vec<std::process::Child>,
) -> Result<()> {
    let Some(target) = app.editor_request.take() else { return Ok(()) };
    // Absolute, so no editor can read the file name as one of its own flags and no dialect
    // needs a `--` guard (`src/editor.rs`). `absolute` keeps symlinks, unlike canonicalize, so
    // a worktree reached through one opens under the name the reviewer knows.
    let joined = app.repo.join(&target.path);
    let path = std::path::absolute(&joined).unwrap_or(joined);
    let command = match editor::resolve(
        configured,
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
        &path,
        target.line,
    ) {
        Ok(command) => command,
        // Two causes, and the second would otherwise be told to set what it set.
        Err(editor::NoEditor::Unset) => {
            app.status = "set `editor` in config.toml, or $EDITOR".into();
            return Ok(());
        }
        Err(editor::NoEditor::NamesNoProgram) => {
            app.status = "the editor setting names no program".into();
            return Ok(());
        }
    };
    // `all_files` lists what the index tracks, so a file removed from the worktree can still
    // be a row, and the changeset only catches it inside a scope that diffs the worktree
    // An editor opens an empty buffer for it and recreates it on
    // save, so the press stops here. Off the render path, so one `stat` costs nothing.
    if !path.is_file() {
        app.status = format!("{} is gone", target.path);
        return Ok(());
    }
    logln!("editor run {} {:?}", command.program, command.args);
    // The reviewer's own PATH first, so a version-managed editor wins over a stale copy in a
    // common bin, with the host locations as the fallback a stripped pane PATH needs
    // (`src/proc.rs`).
    // Asked before the pane changes hands: an editor that is not there would otherwise flip
    // the screen down to the shell and back for a spawn that never happened, on every press
    let Some(mut cmd) = proc::user_command(&command.program) else {
        app.status = format!("no editor at {}", command.program);
        return Ok(());
    };
    cmd.args(&command.args).current_dir(&app.repo);

    if !command.wants_terminal {
        // A window editor never reads the terminal, so diff-reckoner keeps it. The reviewer keeps
        // the diff on screen, and raw mode stays on, which is what keeps a `ctrl+c` in the
        // pane a key event rather than a signal that would take an unsaved draft with it
        // Nothing waits on it either: the poll shows the write. The
        // launchers that have since exited are collected here, so a session leaves none behind.
        open.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        app.status = match cmd.spawn() {
            Ok(child) => {
                open.push(child);
                format!("opened {}", target.path)
            }
            Err(e) => format!("editor failed: {e}"),
        };
        return Ok(());
    }

    // A terminal editor paints in the pane, so it gets the pane and the loop waits it out.
    // Cooked mode comes back with it, so a `ctrl+c` pressed before the editor installs its own
    // raw mode is a signal, not a key. That gap is the editor's own startup and diff-reckoner adds
    // nothing to it.
    app.forget_pointer();
    release_terminal(kbd);
    let launched = cmd.status();
    claim_terminal(kbd);
    drain_input(app)?;

    match launched {
        // The editor ran, so the worktree may have moved whatever it exited with: `:cq` after a
        // write is a non-zero exit over a real edit.
        Ok(status) => {
            app.status = if status.success() {
                format!("edited {}", target.path)
            } else {
                format!("editor exited with {status}")
            };
            app.request_world_refresh(false);
            app.refresh_commanded = true;
        }
        Err(e) => app.status = format!("editor failed: {e}"),
    }
    invalidate_screen(terminal)?;
    Ok(())
}

/// Drop what ratatui believes is on screen, so the loop's next draw is a full one.
///
/// The editor owned the terminal meanwhile, so the previous buffer no longer describes it and a
/// diffed frame would leave the editor's leavings up. The loop draws at the top of every pass,
/// which is the repaint itself; this only makes that draw whole.
///
/// `Terminal::resize` rather than `Terminal::clear`: `clear` first round-trips a cursor-position
/// query through stdin and blocks until the terminal answers, which swallows the reviewer's next
/// keypress. `resize` clears the same region and resets the same buffer with no query.
fn invalidate_screen(terminal: &mut DefaultTerminal) -> Result<()> {
    let area = terminal.size()?.into();
    terminal.resize(area)?;
    Ok(())
}

/// The reviewed repository, resolved to its git top level. Every `App` goes through this,
/// blocked or ready, so the app and the worker's `TurnHost` hold the same spelling: they key
/// the baseline ref off it independently, and turn membership compares resolved top levels
/// against it.
///
/// A non-repo path is not an error — the pane opens to an empty state and starts showing
/// changes if the directory becomes a repo.
fn repo_root(cfg: &Config) -> std::path::PathBuf {
    git::toplevel(&cfg.repo).unwrap_or_else(|| cfg.repo.clone())
}

/// The startup app for one config snapshot: ready on `Ok`, blocked with the error on
/// `Err`. Both the env-resolved build and the post-paint CLI-resolved rebuild go through
/// here, so the two paths cannot drift.
fn app_for(cfg: &Config, initial_config: &Result<PluginConfig, config::PluginConfigError>) -> App {
    match initial_config {
        Ok(plugin_config) => ready_app(cfg, plugin_config.clone()),
        Err(error) => {
            let mut app = App::blocked(repo_root(cfg), Scope::Uncommitted, cfg.base.clone());
            app.set_config_error(error.to_string());
            app
        }
    }
}

/// Build a fresh working pane only after the plugin configuration has validated.
fn ready_app(cfg: &Config, plugin_config: PluginConfig) -> App {
    let repo = repo_root(cfg);
    let scope = plugin_config.default_scope();
    let whole_file = plugin_config.whole_file();
    logln!(
        "start repo={} poll={:?} base={:?} scope={}",
        repo.display(),
        cfg.poll,
        cfg.base,
        scope.name()
    );
    let mut app = App::new(repo, scope, cfg.base.clone());
    app.whole_file = whole_file;
    app.set_config_dir(cfg.plugin_config_dir.clone());
    app.set_plugin_config(plugin_config);
    app.set_cli_theme(cfg.theme.clone());
    if let Some(wrap) = cfg.wrap {
        app.wrap = wrap;
    }
    app
}

/// A transient status message (e.g. "sent 3 comments") fades after this long idle.
const STATUS_TTL: Duration = Duration::from_secs(4);

/// The exit deadline: stillness this long after the pointer's last event sat on the pane's
/// edge completes the live gesture — the host may route mouse by pointer position, so a release
/// past the pane never arrives. Long enough that a pause while border-scrolling survives,
/// short enough that an overshoot release's copy beats the paste.
/// Its own constant, never the `--poll` cadence: the two measure unrelated things.
const EXIT_DEADLINE: Duration = Duration::from_secs(1);

/// How long an ambient refresh must stay in flight before the tab-strip glyph shows —
/// routine refreshes stay invisible; a commanded one (`r`) shows immediately.
const INDICATOR_DELAY: Duration = Duration::from_millis(200);
/// Once lit, the glyph holds at least this long, so a fast landing still reads.
const INDICATOR_MIN_SHOW: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigGate {
    Blocked,
    Unchanged,
    Changed,
}

impl ConfigGate {
    fn ready(self) -> bool {
        self != Self::Blocked
    }
}

/// The config and layout that produced the visible frame. Input dispatches only while these
/// values still match, so a late observation can never reinterpret painted keys or geometry.
#[derive(Debug)]
struct PaintedFrameSnapshot {
    plugin_config: Option<PluginConfig>,
    config_error: Option<String>,
    navigator_position: crate::config::NavigatorPosition,
    navigator_side_pct: u16,
    navigator_stack_pct: u16,
}

impl PaintedFrameSnapshot {
    fn capture(app: &App) -> Self {
        Self {
            plugin_config: app.plugin_config().cloned(),
            config_error: app.config_error().map(str::to_owned),
            navigator_position: app.navigator_position,
            navigator_side_pct: app.navigator_side_pct,
            navigator_stack_pct: app.navigator_stack_pct,
        }
    }

    fn still_current(&self, app: &App) -> bool {
        self.plugin_config.as_ref() == app.plugin_config()
            && self.config_error.as_deref() == app.config_error()
            && self.navigator_position == app.navigator_position
            && self.navigator_side_pct == app.navigator_side_pct
            && self.navigator_stack_pct == app.navigator_stack_pct
    }

    /// This frame's `editor` command, so one press uses one validated snapshot
    fn editor(&self) -> Option<&str> {
        self.plugin_config.as_ref().and_then(PluginConfig::editor)
    }

    fn keymap(&self) -> &Keymap {
        match &self.plugin_config {
            Some(config) => config.keymap(),
            None => keymap::default_keymap(),
        }
    }
}

/// Land one world completion. The snapshot reconciles only when the completion carries
/// the live generation and its input still matches the view — a mismatched snapshot is
/// discarded whole and a fresh refresh queued. Returns whether the
/// completion matched the live generation — the caller clears the in-flight marker on
/// `true`.
pub fn land_world_completion(
    app: &mut App,
    completion: crate::world::WorldCompletion,
    generation: u64,
) -> bool {
    if completion.generation != generation {
        // A superseding job carries reveal=false, so a superseded switch's reveal would
        // die here; re-arm it to ride the next dispatch instead.
        if completion.reveal {
            app.request_world_refresh(true);
        }
        return false;
    }
    match completion.snapshot {
        Some(Ok(snapshot))
            if app.config_error().is_none() && app.world_input() == completion.input =>
        {
            app.reconcile_world(snapshot);
            if completion.reveal {
                // The switch frame revealed the stashed cursor; the landing may have
                // re-anchored it, so settle and reveal again.
                app.settle_tab_entry();
                app.reveal_files = true;
            }
        }
        // The view moved on while the build ran: discard whole, refresh again, keeping
        // an undelivered reveal alive.
        Some(Ok(_)) => app.request_world_refresh(completion.reveal),
        // A failed refresh reports and keeps the stale frame — the same contract as a
        // failed poll.
        Some(Err(e)) => app.status = format!("refresh failed: {e}"),
        None => {}
    }
    true
}

/// Land one search completion. A stale generation is discarded whole — a result set
/// paints only while it matches the query as typed. Returns whether the
/// completion matched the live generation, mirroring [`land_world_completion`].
pub fn land_search_completion(
    app: &mut App,
    completion: crate::search::SearchCompletion,
    generation: u64,
) -> bool {
    if completion.generation != generation {
        return false;
    }
    app.apply_search_completion(completion);
    true
}

/// Whether a file tab's in-flight refresh shows the tab-strip glyph: past the delay, and
/// only for a job that builds a snapshot — a sample-only job never lights it.
fn world_indicator(inflight: Option<(Duration, bool)>) -> bool {
    inflight.is_some_and(|(elapsed, builds)| builds && elapsed >= INDICATOR_DELAY)
}

/// Whether a lit glyph may go dark: only once the minimum display has passed, so the
/// acknowledgment is perceptible rather than a two-frame blink.
fn glyph_clears(lit_for: Duration) -> bool {
    lit_for >= INDICATOR_MIN_SHOW
}

/// The tight wake while a worker owes a completion, so its landing paints near the
/// build's own speed — shared by the world and search workers.
const WORKER_TIGHT_WAKE: Duration = Duration::from_millis(15);

/// The wake while a world job is in flight: tight for a building job so its landing paints
/// near the build's own speed, the fetch cadence for a sample-only one.
fn world_wake(builds: bool) -> Duration {
    if builds { WORKER_TIGHT_WAKE } else { Duration::from_millis(100) }
}

/// Draw, then wait up to the poll deadline for input; refresh on each tick.
fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    cfg: &Config,
    kbd: bool,
) -> Result<()> {
    let poll = cfg.poll;
    let mut last_poll = Instant::now();
    // The exit signature's two halves: the last mouse event's arrival, and whether that
    // event sat on the pane's edge — only both together let the exit deadline complete a
    // gesture whose release was lost past the pane.
    let mut last_mouse = Instant::now();
    let mut mouse_exited = false;
    let (recovery_tx, recovery_rx) = mpsc::channel::<(u64, PluginConfig, App)>();
    let mut recovery_inflight = false;
    // The world worker owns every refresh build; the loop sends input-tagged jobs and
    // reconciles the completions.
    let (world_tx, world_job_rx) = mpsc::channel::<crate::world::WorldJob>();
    let (world_res_tx, world_rx) = mpsc::channel::<crate::world::WorldCompletion>();
    let _world_worker = crate::world::spawn(world_job_rx, world_res_tx);
    // Window editors diff-reckoner launched, reaped at the next press.
    let mut open_editors: Vec<std::process::Child> = Vec::new();
    let mut world_generation = 0_u64;
    let mut world_inflight: Option<(Instant, bool)> = None;
    // The search worker spawns on the first overlay open, so a session that never
    // searches never pays for the engine's index.
    let mut search_worker: Option<(
        mpsc::Sender<crate::search::SearchJob>,
        mpsc::Receiver<crate::search::SearchCompletion>,
    )> = None;
    let mut search_generation = 0_u64;
    let mut search_inflight = false;
    // When the tab-strip glyph turned on — the minimum-display clock.
    let mut glyph_since: Option<Instant> = None;
    let mut config_epoch = 0_u64;
    let mut status_at = Instant::now();
    let mut last_status = String::new();
    let result: Result<()> = (|| {
        while !app.should_quit {
            if let Ok((epoch, target, mut recovered)) = recovery_rx.try_recv() {
                recovery_inflight = false;
                if epoch == config_epoch {
                    match config::plugin_config(cfg.plugin_config_dir.as_deref()) {
                        Ok(current) if current == target => {
                            recovered.carry_authored_state_from(app);
                            *app = recovered;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let message = error.to_string();
                            if app.config_error() != Some(message.as_str()) {
                                config_epoch = config_epoch.wrapping_add(1);
                            }
                            app.set_config_error(message);
                        }
                    }
                }
            }

            let size = terminal.size()?;
            let area = Rect::new(0, 0, size.width, size.height);
            // Every frame starts from one complete validated config snapshot. Input below uses the
            // keymap and geometry this draw paints; later observations continue before another input.
            reconcile_plugin_config(
                app,
                cfg,
                area,
                &mut config_epoch,
                &recovery_tx,
                &mut recovery_inflight,
            );
            // The tab-strip refresh glyph: a commanded refresh lights it
            // immediately, an ambient one past the appear delay. Once lit it holds a
            // minimum, so a fast landing still reads.
            if std::mem::take(&mut app.refresh_commanded) {
                glyph_since.get_or_insert_with(Instant::now);
            }
            let glyph_due = world_indicator(
                world_inflight.map(|(started, builds)| (started.elapsed(), builds)),
            );
            let mut glyph_wake = None;
            if glyph_due {
                glyph_since.get_or_insert_with(Instant::now);
            } else if let Some(lit) = glyph_since {
                if glyph_clears(lit.elapsed()) {
                    glyph_since = None;
                } else {
                    // Wake at the hold boundary, so the glyph goes dark on time when idle.
                    glyph_wake = Some(INDICATOR_MIN_SHOW.saturating_sub(lit.elapsed()));
                }
            }
            app.refresh_indicator = glyph_since.is_some();
            // Expire a stale status line: restart the timer when the message changes, and clear
            // it once it has lingered past the TTL, so a notification doesn't stay up forever.
            if app.status != last_status {
                last_status.clone_from(&app.status);
                status_at = Instant::now();
            }
            if !app.status.is_empty() && status_at.elapsed() >= STATUS_TTL {
                app.status.clear();
                last_status.clear();
            }
            // Settle both panes' scroll for this frame's viewport before painting, so the
            // diff window matches what mouse hit-testing will map against. Each pane reveals its
            // cursor only when a navigation requested it (so the wheel can scroll freely), then
            // bounds the offset every frame. While composing, reserve the inline box's rows and
            // keep revealing so the anchored line stays above the growing box.
            // Rebuild the search preview only once input has settled: with input still queued
            // the build defers, so a pick sweep never waits on it. `build_search_preview` is
            // idempotent — it rebuilds only when the preview no longer matches the pick
            if app.mode == crate::app::Mode::Search && !event::poll(Duration::ZERO)? {
                app.build_search_preview();
            }
            // Likewise a held file-list key opens the file it rests on once input settles.
            if app.read_pending() && !event::poll(Duration::ZERO)? {
                app.settle_read_pending();
            }
            let viewport = ui::diff_viewport_height(area, app);
            let effective = if app.composing() {
                let box_h = ui::composer_height(app, ui::diff_inner_width(area, app));
                viewport.saturating_sub(box_h).max(1)
            } else {
                viewport
            };
            let heights = ui::diff_row_heights(app, area);
            // A comment box open on the Comments tab keeps its card revealed instead
            // (`settle_comments`), leaving the file tab's diff where it was.
            let composing_here = app.composing() && app.tab != crate::app::Tab::Comments;
            if std::mem::take(&mut app.reveal_diff) || composing_here {
                app.reveal_diff_cursor(&heights, effective);
            }
            app.bound_diff_scroll(&heights, effective);
            let file_vp = ui::file_viewport_height(area, app);
            if app.tab == crate::app::Tab::Comments {
                let cards = ui::card_heights(app, area);
                app.settle_comments(&cards, viewport, file_vp);
            }
            // While the navigator is hidden its viewport is zero, and a reveal computed
            // there would zero the kept scroll — it stays pending for the show frame.
            if !app.navigator_hidden_here() && std::mem::take(&mut app.reveal_files) {
                app.reveal_file_cursor(file_vp);
            }
            app.bound_file_scroll(file_vp);
            let painted_frame = PaintedFrameSnapshot::capture(app);
            terminal.draw(|f| ui::render(f, app))?;

            // A world completion reconciles into the view only while the view it described is
            // still current; the worker's baseline is authoritative either way.
            // A navigator drag holds the drain — the input-tagged channel is the queue, no
            // stored deferral — and the pass that finds the gesture gone drains it to empty,
            // landing only the live generation.
            if !app.gates_world_drain() {
                let mut landed = false;
                while let Ok(completion) = world_rx.try_recv() {
                    if land_world_completion(app, completion, world_generation) {
                        world_inflight = None;
                    }
                    landed = true;
                }
                if landed {
                    continue;
                }
            }

            // A search completion paints only while it matches the query as typed: a stale
            // generation is discarded whole.
            if let Some((_, rx)) = &search_worker
                && let Ok(completion) = rx.try_recv()
            {
                if land_search_completion(app, completion, search_generation) {
                    // A warming engine answers `indexing…` and re-runs by itself, so
                    // the tight wake stays on until real results land.
                    search_inflight = app
                        .search
                        .as_ref()
                        .is_some_and(|s| s.phase == crate::app::SearchPhase::Indexing);
                }
                // Repaint at once, like a world landing — without this the results sit
                // computed but unpainted until the next wake (policies/ux-responsiveness.md).
                continue;
            }
            // A closed overlay owes no landing: without this, a still-warming engine's
            // periodic `indexing…` completions would re-arm the tight wake after `esc`
            // and spin the loop until the cold scan finishes.
            if app.search.is_none() {
                search_inflight = false;
            }

            // Dispatch the queued query after the frame above painted, so typing paints at
            // input speed and the results land behind it.
            if std::mem::take(&mut app.search_dirty)
                && app.mode == crate::app::Mode::Search
                && app.config_error().is_none()
            {
                let (tx, _) = search_worker.get_or_insert_with(|| {
                    let (job_tx, job_rx) = mpsc::channel();
                    let (res_tx, res_rx) = mpsc::channel();
                    crate::search::spawn(
                        app.repo.clone(),
                        crate::search::cache_dir(),
                        job_rx,
                        res_tx,
                    );
                    (job_tx, res_rx)
                });
                search_generation = search_generation.wrapping_add(1);
                let query = app.search.as_ref().map(|s| s.query.clone()).unwrap_or_default();
                search_inflight = tx
                    .send(crate::search::SearchJob::Query { generation: search_generation, query })
                    .is_ok();
                if !search_inflight
                    && let Some(s) = app.search.as_mut()
                    && !matches!(s.phase, crate::app::SearchPhase::Error(_))
                {
                    // A dead worker's first, specific error stays up; only a phase that
                    // never saw one gets the generic message.
                    s.phase = crate::app::SearchPhase::Error("search worker unavailable".into());
                    // Drop the last preview, like a failed completion, so no stale file shows
                    // under the error.
                    s.preview = None;
                }
            }
            if let Some(path) = app.search_track.take()
                && let Some((tx, _)) = &search_worker
            {
                let _ = tx.send(crate::search::SearchJob::Track { path });
            }

            // Dispatch the queued refresh after the frame above painted, so a switch stays
            // instant and the fresh state lands behind it.
            if app.world_request.is_some() && app.config_error().is_none() {
                let request = app.world_request.take().expect("checked above");
                world_generation = world_generation.wrapping_add(1);
                let job = crate::world::WorldJob {
                    generation: world_generation,
                    input: app.world_input(),
                    reveal: request.reveal,
                };
                let builds = job.input.tab.is_file_tab();
                world_inflight = if world_tx.send(job).is_ok() {
                    Some((Instant::now(), builds))
                } else {
                    // A dead worker must not pin the in-flight marker (and its glyph and
                    // tight wake) for the rest of the session.
                    app.status = "refresh worker unavailable".to_string();
                    None
                };
            }
            // Wake at the status-expiry boundary too, so it clears on time when idle.
            let poll_left = poll.saturating_sub(last_poll.elapsed());
            let mut timeout = if app.status.is_empty() {
                poll_left
            } else {
                poll_left.min(STATUS_TTL.saturating_sub(status_at.elapsed()))
            };
            // A world refresh usually lands within tens of milliseconds, so its wake is
            // tight — the landing paints near the build's own speed.
            if let Some((_, builds)) = world_inflight {
                timeout = timeout.min(world_wake(builds));
            }
            if search_inflight {
                timeout = timeout.min(WORKER_TIGHT_WAKE);
            }
            if let Some(wake) = glyph_wake {
                timeout = timeout.min(wake.max(Duration::from_millis(15)));
            }
            if app.config_error().is_none()
                && let Some(wait) = app.base_probe_wait()
            {
                timeout = timeout.min(wait);
            }
            // Wake at the exit deadline, so an abandoned gesture completes on time.
            if app.gesture_active() && mouse_exited {
                timeout = timeout.min(EXIT_DEADLINE.saturating_sub(last_mouse.elapsed()));
            }
            if event::poll(timeout)? {
                if !painted_frame.still_current(app) {
                    continue;
                }
                let event = event::read()?;
                if app.config_error().is_some() {
                    handle_blocked_event(app, &event);
                    continue;
                }
                match event {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        // More input already queued: a file-list move skips this file's load.
                        app.defer_reads = event::poll(Duration::ZERO)?;
                        let handled = handle_key(app, k, area, painted_frame.keymap());
                        app.defer_reads = false;
                        if let Err(e) = handled {
                            app.status = format!("error: {e}");
                        }
                        logln!(
                            "key {:?}{} -> mode={:?} focus={:?} scope={:?} file={}/{} diff_cursor={} scroll={} comments={}",
                            k.code,
                            if k.modifiers.is_empty() {
                                String::new()
                            } else {
                                format!(" {:?}", k.modifiers)
                            },
                            app.mode,
                            app.focus,
                            app.scope,
                            app.file_cursor,
                            app.entries.len(),
                            app.diff_cursor,
                            app.diff_scroll,
                            app.store.len()
                        );
                    }
                    Event::Mouse(m) => {
                        last_mouse = Instant::now();
                        // Reuse this frame's `area` and `heights` (computed above for the scroll
                        // settle) so a drag-select doesn't re-measure the whole diff per motion.
                        if let Err(e) =
                            handle_mouse(app, m, area, &heights, painted_frame.keymap(), &Clipboard)
                        {
                            app.status = format!("error: {e}");
                        }
                        mouse_exited = app.gesture_active() && pointer_at_pane_edge(m, area);
                        logln!(
                            "mouse {:?} col={} row={} -> focus={:?} file={} diff_cursor={} scroll={} anchor={:?}",
                            m.kind,
                            m.column,
                            m.row,
                            app.focus,
                            app.file_cursor,
                            app.diff_cursor,
                            app.diff_scroll,
                            app.select_anchor
                        );
                    }
                    // Bracketed paste: insert at the caret while composing, ignored otherwise.
                    Event::Paste(text) => {
                        app.input_paste(&text);
                        logln!("paste {} chars -> composing={}", text.len(), app.composing());
                    }
                    Event::Resize(_, _) => {
                        handle_resize(app);
                    }
                    _ => {}
                }
            }
            if app.config_error().is_none() {
                app.tick_base_picker_probe();
            }
            // An `edit` press named a file: run the editor, and hand it the pane when it is
            // one that paints there.
            if app.editor_request.is_some() {
                run_editor(terminal, app, painted_frame.editor(), kbd, &mut open_editors)?;
            }
            // An export wrote its file: open it. The write already stands, so a failure only
            // adds to the status that reports it.
            if let Some(path) = app.open_request.take()
                && let Err(e) = browser::open(&path)
            {
                app.status = format!("{}, but could not open it: {e}", app.status);
            }
            if app.should_quit {
                break;
            }
            // Interior stillness is a held button (a release inside would have arrived), so
            // only the exit signature reaches this completion.
            if app.gesture_active() && mouse_exited && last_mouse.elapsed() >= EXIT_DEADLINE {
                complete_gesture(app, area, &Clipboard);
            }
            if last_poll.elapsed() >= poll {
                let config_gate = reconcile_plugin_config(
                    app,
                    cfg,
                    area,
                    &mut config_epoch,
                    &recovery_tx,
                    &mut recovery_inflight,
                );
                if !config_gate.ready() {
                    last_poll = Instant::now();
                    continue;
                }
                // The tick's refresh runs on the worker.
                app.request_world_refresh(false);
                logln!(
                    "poll files={} composing={} diff_cursor={} scroll={}",
                    app.entries.len(),
                    app.composing(),
                    app.diff_cursor,
                    app.diff_scroll
                );
                last_poll = Instant::now();
            }
        }
        Ok(())
    })();
    restore_terminal(kbd);
    result
}

/// A blocked frame accepts no normal input. Quit remains available, and terminal/pointer cleanup
/// may release state that was captured before the config became invalid.
fn handle_blocked_event(app: &mut App, event: &Event) {
    match event {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            // The blocked screen's escape hatch stays modifier-agnostic: a stuck user's `q` quits
            // whatever the modifiers, exactly as before the keymap gained chords.
            if let KeyCode::Char(c) = k.code
                && keymap::default_keymap().action_for(keymap::Key::plain(c))
                    == Some(keymap::Action::Quit)
            {
                app.should_quit = true;
            }
        }
        Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), .. })
            if app.divider_drag_captured() =>
        {
            app.finish_divider_drag();
        }
        Event::Resize(_, _) => handle_resize(app),
        _ => {}
    }
}

fn reconcile_plugin_config(
    app: &mut App,
    cfg: &Config,
    area: Rect,
    config_epoch: &mut u64,
    recovery_tx: &mpsc::Sender<(u64, PluginConfig, App)>,
    recovery_inflight: &mut bool,
) -> ConfigGate {
    let previous = app.plugin_config().cloned();
    let observed = config::plugin_config(cfg.plugin_config_dir.as_deref());
    // A config layout or theme change reflows the frame under a live gesture, and the block
    // screen replaces its body: both complete the gesture's copy before the new frame
    // applies, so a world event never silently ends a visible selection.
    if app.gesture_active()
        && let Some(p) = &previous
        && config_ends_gesture(p, observed.as_ref().ok())
    {
        complete_gesture(app, area, &Clipboard);
    }
    if !apply_plugin_config_observation(
        app,
        cfg,
        config_epoch,
        recovery_tx,
        recovery_inflight,
        observed,
    ) {
        return ConfigGate::Blocked;
    }
    let current = app.plugin_config().expect("ready after successful observation");
    let Some(previous) = previous.filter(|previous| previous != current) else {
        return ConfigGate::Unchanged;
    };

    if previous.active_theme() != current.active_theme() {
        // A theme change invalidates highlighted diffs. Rebuild before another input or
        // frame can mix states; `reload` preserves the frozen diff while composing.
        if let Err(error) = app.reload() {
            app.status = format!("config refresh failed: {error}");
        }
    }
    ConfigGate::Changed
}

/// Whether a fresh config observation ends a live gesture — the end table's config row: a
/// layout or theme change reflows the frame, and a failed observation (`None`) blocks the
/// body.
#[must_use]
fn config_ends_gesture(previous: &PluginConfig, observed: Option<&PluginConfig>) -> bool {
    match observed {
        Some(c) => {
            previous.navigator_position() != c.navigator_position()
                || previous.active_theme() != c.active_theme()
        }
        None => true,
    }
}

/// Apply one complete config observation. Invalid state blocks work. Recovery loads a fresh
/// app on a tagged worker, then the event loop revalidates its target and carries authored
/// review state before swapping it in.
fn apply_plugin_config_observation(
    app: &mut App,
    cfg: &Config,
    epoch: &mut u64,
    recovery_tx: &mpsc::Sender<(u64, PluginConfig, App)>,
    recovery_inflight: &mut bool,
    observed: Result<PluginConfig, config::PluginConfigError>,
) -> bool {
    match observed {
        Ok(next) => {
            let recovering = app.plugin_config().is_none();
            let changed = app.plugin_config().is_some_and(|current| current != &next);
            if recovering {
                if !*recovery_inflight {
                    *epoch = epoch.wrapping_add(1);
                    *recovery_inflight = true;
                    let (tx, cfg, target, recovery_epoch) =
                        (recovery_tx.clone(), cfg.clone(), next, *epoch);
                    thread::spawn(move || {
                        let mut recovered = ready_app(&cfg, target.clone());
                        if let Err(error) = recovered.reload() {
                            recovered.status = format!("load failed: {error}");
                        }
                        let _ = tx.send((recovery_epoch, target, recovered));
                    });
                }
                return false;
            } else if changed {
                app.set_plugin_config(next);
            }
            true
        }
        Err(error) => {
            let message = error.to_string();
            if app.plugin_config().is_some() || app.config_error() != Some(message.as_str()) {
                *epoch = epoch.wrapping_add(1);
            }
            app.set_config_error(message);
            false
        }
    }
}

/// Diff scroll steps: a full page for `PageUp`/`PageDown`, half for `ctrl+u`/`ctrl+d`.
const PAGE: isize = 15;
const HALF_PAGE: isize = 8;

/// Apply one readline-style editing key to the active field — the comment draft or the
/// search query — so the two input surfaces stay in lockstep, edited by one control set
/// The caller handles its own mode keys first and delegates the rest
/// here. `word` requests a word-wise horizontal move (Alt/Ctrl + arrow, terminal-dependent).
fn apply_text_edit(app: &mut App, code: KeyCode, ctrl: bool, alt: bool, word: bool) {
    use KeyCode::{Backspace, Char, Delete, End, Home, Left, Right};
    match code {
        Char('w') if ctrl => app.input_delete_word(),
        Char('a') if ctrl => app.caret_home(),
        Char('e') if ctrl => app.caret_end(),
        Char('u') if ctrl => app.input_kill_to_start(),
        Char('k') if ctrl => app.input_kill_to_end(),
        // Word-jump: `Alt+b`/`Alt+f` (readline; survives as ESC-prefixed, unlike modified
        // arrows, which many terminals/multiplexers strip) and modified arrows where they are
        // delivered. These precede the plain-character insert below.
        Char('b') if alt => app.caret_word_left(),
        Char('f') if alt => app.caret_word_right(),
        Left if word => app.caret_word_left(),
        Right if word => app.caret_word_right(),
        Left => app.caret_left(),
        Right => app.caret_right(),
        Home => app.caret_home(),
        End => app.caret_end(),
        Delete => app.input_delete_forward(),
        Backspace => app.input_backspace(),
        Char(c) if !ctrl => app.input_push(c),
        _ => {}
    }
}

/// Map one key press onto `App` through `keymap` — the keymap of the frame on screen, so a
/// stale hint never dispatches a different action than it advertised.
/// Public for the dispatch tests; the event loop is the runtime caller.
pub fn handle_key(app: &mut App, key: KeyEvent, area: Rect, keymap: &Keymap) -> Result<()> {
    use crate::keymap::Action as K;
    use KeyCode::{Char, Down, Enter, Esc, Left, PageDown, PageUp, Right, Tab, Up};
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // A keypress cancels the gesture but keeps consuming its drag events until mouse-up.
    app.cancel_divider_drag();
    // A reflow input cancels a live text or gutter gesture: nothing copies, and the key
    // still performs its own action. The hover `+` stays — it
    // recomputes each frame from the pointer's last reported cell.
    app.cancel_gesture();
    // Any keypress is the user doing something else: the settled highlight clears
    app.clear_settled_selection();

    if app.composing() {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let alt_or_shift = key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        let word = alt || ctrl; // word-jump on Alt/Ctrl + arrow (terminal-dependent)
        // The wrapped width of the box, for vertical (wrapped-row) caret movement.
        let cw = ui::composer_content_width(app, ui::diff_inner_width(area, app));
        match key.code {
            Esc => app.cancel_comment(),
            // Alt/Shift+Enter (and Ctrl+J) insert a newline; plain Enter submits.
            Enter if alt_or_shift => app.input_push('\n'),
            Enter => app.submit_comment(),
            Char('j') if ctrl => app.input_push('\n'),
            // The box wraps, so `↑`/`↓` walk display rows here rather than editing text.
            Up | Down => {
                app.caret = ui::caret_vertical(&app.input, app.caret, cw, key.code == Down);
            }
            code => apply_text_edit(app, code, ctrl, alt, word),
        }
        return Ok(());
    }

    // The delete popup: `←`/`→` move between its two buttons, `enter` takes the highlighted
    // one, `esc` cancels. Every other key is inert.
    if matches!(app.mode, Mode::ConfirmDelete { .. }) {
        match key.code {
            Esc => app.cancel_delete(),
            Enter => app.confirm_delete_pick(),
            Left => app.confirm_delete_choose(false),
            Right => app.confirm_delete_choose(true),
            _ => {}
        }
        return Ok(());
    }

    // The search screen: the query edits with the comment editor's caret controls,
    // newlines excluded — every edit re-queries off the frame loop. `tab` flips the
    // mode; the page keys scroll the preview.
    if app.mode == Mode::Search {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let word = alt || ctrl;
        match key.code {
            Esc => app.close_search(),
            Enter => app.search_open_pick()?,
            Tab => app.search_flip(),
            PageDown => app.scroll_search_preview(PAGE),
            PageUp => app.scroll_search_preview(-PAGE),
            // The single-line query has no rows, so `↑`/`↓` (and `ctrl+n`/`p`) move the pick.
            Down => app.search_move(1),
            Up => app.search_move(-1),
            Char('n') if ctrl => app.search_move(1),
            Char('p') if ctrl => app.search_move(-1),
            code => apply_text_edit(app, code, ctrl, alt, word),
        }
        return Ok(());
    }

    // The in-file find band: printable keys edit the query, the steps move the cursor between
    // matches (`↑`/`↓` are the steps, so the single-line query has no vertical caret), `esc`
    // closes. Every other key is inert.
    if app.mode == Mode::Find {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let word = alt || ctrl;
        match key.code {
            Esc => app.close_find(),
            Enter | Down => app.find_step(1),
            Up => app.find_step(-1),
            code => apply_text_edit(app, code, ctrl, alt, word),
        }
        return Ok(());
    }

    // The bound shortcuts dispatch through the frame's keymap: a character, a chord, or a
    // named key — the arrows and page keys are default bindings like any other
    // A key resolving to no action falls through to the fixed keys below
    // (`tab`, `esc`), which stay hardcoded per context.
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let code = match key.code {
        Char(c) => Some(keymap::KeyCode::Char(c)),
        Left => Some(keymap::KeyCode::Left),
        Right => Some(keymap::KeyCode::Right),
        Up => Some(keymap::KeyCode::Up),
        Down => Some(keymap::KeyCode::Down),
        PageUp => Some(keymap::KeyCode::PageUp),
        PageDown => Some(keymap::KeyCode::PageDown),
        Enter => Some(keymap::KeyCode::Enter),
        _ => None,
    };
    let action = code.and_then(|code| keymap.action_for(crate::keymap::Key { ctrl, alt, code }));
    // A deferred file-list move opens its file before any key but another move acts.
    let list_move = matches!(
        action,
        Some(
            K::Down
                | K::Up
                | K::PageDown
                | K::PageUp
                | K::HalfDown
                | K::HalfUp
                | K::NextFile
                | K::PrevFile
        )
    ) && app.focus == Focus::Files;
    if !list_move {
        app.settle_read_pending();
    }

    // An armed crossing waits for a repeat of the hunk step that armed it. Every other key drops
    // it, and still does its own work. The steps themselves settle their arm in
    // `step_hunk`, which is what makes the other direction disarm too. `esc` is exempt: the `esc`
    // ladder drops the crossing as its own explicit step, so a later layer is not also consumed.
    if !matches!(action, Some(K::NextHunk | K::PrevHunk)) && key.code != Esc {
        app.disarm_cross();
    }
    // The git-ignored warning holds for the very next `comment` press only.
    if action != Some(K::Comment) {
        app.ignored_confirm = None;
    }

    // The base picker: every printable narrows the filter — the bound shortcuts included, so
    // a branch named `qa` is typable — and the filter edits with the comment editor's
    // controls, like every other text field. `↑`/`↓` (and `ctrl+n`/`p`) and the page keys
    // move the highlight, so the single-line filter keeps `←`/`→`/`home`/`end` for its
    // caret, `enter` picks, `esc` cancels
    if app.mode == Mode::BasePick {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let word = alt || ctrl;
        match key.code {
            Esc => app.close_base_picker(),
            Enter => app.base_picker_pick()?,
            Down => app.base_picker_move(1),
            Up => app.base_picker_move(-1),
            PageDown => app.base_picker_move(PAGE),
            PageUp => app.base_picker_move(-PAGE),
            Char('n') if ctrl => app.base_picker_move(1),
            Char('p') if ctrl => app.base_picker_move(-1),
            code => apply_text_edit(app, code, ctrl, alt, word),
        }
        return Ok(());
    }

    // The commit picker: the movement bindings and the page actions move the highlight, `v`
    // (the `select` binding) sets the anchor, `enter` picks the run, `esc` clears the anchor
    // else closes. Every other key is inert, so `q` cannot quit and `/` cannot search from
    // inside it.
    if app.mode == Mode::CommitPick {
        match (action, key.code) {
            (_, Esc) => app.commit_picker_escape(),
            (_, Enter) => app.commit_picker_pick()?,
            (Some(K::Down), _) => app.commit_picker_move(1),
            (Some(K::Up), _) => app.commit_picker_move(-1),
            (Some(K::PageDown), _) => app.commit_picker_move(PAGE),
            (Some(K::PageUp), _) => app.commit_picker_move(-PAGE),
            (Some(K::HalfDown), _) => app.commit_picker_move(HALF_PAGE),
            (Some(K::HalfUp), _) => app.commit_picker_move(-HALF_PAGE),
            (Some(K::Select), _) => app.commit_picker_anchor(),
            _ => {}
        }
        return Ok(());
    }

    // The theme picker: `↑`/`↓` move within a side and `←`/`→` switch sides, each previewing
    // the highlight; `enter` saves it; `esc` and the `theme` binding close. Every other key is
    // inert. A theme swap drops the highlighted diffs, so a changed theme rebuilds the open one
    // before the next frame, as a config theme change does.
    if app.mode == Mode::ThemePick {
        use crate::theme::Appearance;
        let before = app.active_theme();
        match (action, key.code) {
            (Some(K::Theme), _) | (_, Esc) => app.close_theme_picker(),
            (_, Enter) => app.theme_picker_save(),
            (_, Down) => app.theme_picker_move(1),
            (_, Up) => app.theme_picker_move(-1),
            (_, Left) => app.theme_picker_side(Appearance::Dark),
            (_, Right) => app.theme_picker_side(Appearance::Light),
            _ => {}
        }
        if app.active_theme() != before && app.config_error().is_none() {
            app.reload()?;
        }
        return Ok(());
    }

    if app.tab == crate::app::Tab::Comments
        && app.mode == Mode::Normal
        && handle_comments_key(app, action, key.code, area)?
    {
        return Ok(());
    }

    if let Some(action) = action {
        match action {
            K::Quit => app.should_quit = true,
            K::Refresh => {
                app.request_world_refresh(false);
                app.refresh_commanded = true;
            }
            K::TabChanges => app.set_tab(crate::app::Tab::Changes)?,
            K::TabAllFiles => app.set_tab(crate::app::Tab::AllFiles)?,
            K::TabComments => app.set_tab(crate::app::Tab::Comments)?,
            K::Down => app.move_cursor(1)?,
            K::Up => app.move_cursor(-1)?,
            // `expand`/`collapse` act on a directory under the file list's cursor, and otherwise
            // scroll the diff sideways (`scroll_h` is a no-op while wrapping, so it only acts
            // when h-scroll is meaningful). A diff fold opens and hides on `activate`.
            K::Expand if app.on_folder() => app.expand_dir(),
            K::Collapse if app.on_folder() => app.collapse_dir(),
            K::Expand => app.scroll_h(8),
            K::Collapse => app.scroll_h(-8),
            K::PageDown => app.move_cursor(PAGE)?,
            K::PageUp => app.move_cursor(-PAGE)?,
            K::HalfDown => app.move_cursor(HALF_PAGE)?,
            K::HalfUp => app.move_cursor(-HALF_PAGE)?,
            K::NextHunk => app.next_hunk(),
            K::PrevHunk => app.prev_hunk(),
            K::NextFile => app.next_file(),
            K::PrevFile => app.prev_file(),
            K::Wrap => app.toggle_wrap(),
            K::WholeFile => {
                let heights = ui::diff_row_heights(app, area);
                let viewport = ui::diff_viewport_height(area, app);
                app.toggle_whole_file(&heights, viewport, |app| ui::diff_row_heights(app, area));
            }
            K::Theme => app.open_theme_picker(),
            K::Preview => app.toggle_preview(),
            K::NavigatorPosition => app.cycle_navigator_position(),
            K::NavigatorHide => app.toggle_navigator_hidden(),
            K::NavigatorGrow => app.resize_navigator(4),
            K::NavigatorShrink => app.resize_navigator(-4),
            K::ScopeUncommitted => app.set_scope(Scope::Uncommitted)?,
            K::ScopeBranch => app.set_scope(Scope::Branch)?,
            K::ScopeCommits => app.set_scope(Scope::Commits)?,
            K::BasePick => app.open_base_picker(),
            K::CommitPick => app.open_commit_picker(),
            K::Select => app.toggle_select(),
            K::Comment => app.start_comment(),
            // `delete` acts on the comment under the diff cursor, so it only fires with the
            // diff focused — otherwise it would silently drop a comment under an off-screen
            // cursor. `edit` runs from either pane: the read pane's file at its line, the
            // navigator's selected file.
            K::Edit => app.start_edit(),
            K::Delete if app.focus == Focus::Diff => app.ask_delete_comment(),
            K::Export => app.export_to_file(),
            K::Copy => {
                app.export(&Clipboard);
            }
            K::NextComment => app.jump_comment(1),
            K::PrevComment => app.jump_comment(-1),
            K::Search => app.open_search(),
            K::Find => app.open_find(),
            K::Keys => app.toggle_keys(),
            K::Activate => app.activate(),
            // `delete` off the diff is inert. `edit` is not: it reaches the navigator's file
            // rows too. `open-comment` has a target on the Comments tab alone.
            K::Delete | K::OpenComment => {}
        }
        return Ok(());
    }

    match key.code {
        Tab => app.toggle_focus(),
        // `esc` peels one layer: a live selection, then an armed crossing, then the footer
        // expansion, then the diff's focus (the `esc` ladder).
        Esc => app.escape(),
        _ => {}
    }
    Ok(())
}

/// A key on the Comments tab. The movement bindings step and page through the cards, the
/// file steps cross files, `open-comment` opens the selected comment in `All files`, and
/// `activate`/`delete` act on it. The file tabs' cursor, fold, selection, pane, and `edit`
/// keys have no target here, so they are inert; every other key falls through to its usual
/// action.
/// Returns whether the key was taken.
fn handle_comments_key(
    app: &mut App,
    action: Option<crate::keymap::Action>,
    code: KeyCode,
    area: Rect,
) -> Result<bool> {
    use crate::keymap::Action as K;
    let page = |app: &mut App, pages: isize, halves: bool| {
        let heights = ui::card_heights(app, area);
        let viewport = ui::diff_viewport_height(area, app);
        let step = if halves { viewport / 2 } else { viewport.saturating_sub(2) };
        let lines = isize::try_from(step.max(1)).unwrap_or(isize::MAX);
        app.comments.page(pages * lines, &heights, viewport);
    };
    match action {
        Some(K::Down | K::NextComment) => app.step_comment(1),
        Some(K::Up | K::PrevComment) => app.step_comment(-1),
        Some(K::NextFile) => app.step_comment_file(true),
        Some(K::PrevFile) => app.step_comment_file(false),
        Some(K::PageDown) => page(app, 1, false),
        Some(K::PageUp) => page(app, -1, false),
        Some(K::HalfDown) => page(app, 1, true),
        Some(K::HalfUp) => page(app, -1, true),
        Some(K::OpenComment) => app.open_comment_in_files()?,
        Some(K::Activate) => app.activate(),
        Some(K::Delete) => app.ask_delete_comment(),
        Some(
            K::Edit
            | K::Expand
            | K::Collapse
            | K::NextHunk
            | K::PrevHunk
            | K::Select
            | K::Comment
            | K::Preview
            | K::WholeFile
            | K::Find
            | K::BasePick
            | K::CommitPick,
        ) => {}
        // `tab` and the `esc` ladder act on the file tab's panes and selection, which this
        // tab does not show; `esc` still folds the footer expansion.
        None if code == KeyCode::Tab => {}
        None if code == KeyCode::Esc => app.keys_expanded = false,
        _ => return Ok(false),
    }
    Ok(true)
}

/// A left click on the Comments tab's body. A navigator row (a file's row selects its first
/// comment) or a card's code selects the comment; its box opens it for editing, as in the
/// diff. A card's heading opens the comment in `All files` instead. The box being left saves
/// first, and a click on the card already open leaves its box, caret and all, as it is.
fn comments_click(app: &mut App, m: MouseEvent, area: Rect) -> Result<()> {
    use crate::comments_tab::Reveal;
    use ui::CommentsHit as H;
    let Some(hit) = ui::comments_hit(area, app, m.column, m.row) else { return Ok(()) };
    let on_open_card = matches!(hit, H::Box(i) | H::Card(i) if app.edited_card() == Some(i));
    if on_open_card || !app.close_card_box() {
        return Ok(());
    }
    match hit {
        H::NavComment(i) | H::NavFile(i) => app.select_comment(i, Reveal::Top),
        H::Heading(i) => {
            app.select_comment(i, Reveal::Visible);
            app.open_comment_in_files()?;
        }
        H::Box(i) => app.open_card(i, Reveal::Visible),
        H::Card(i) => app.select_comment(i, Reveal::Visible),
    }
    Ok(())
}

/// Cancel pointer state whose coordinates belonged to the old terminal geometry.
fn handle_resize(app: &mut App) {
    app.cancel_divider_drag();
    // A resize reflows wrapping under an active gesture, so it cancels like a keypress —
    // and re-wraps the display rows a settled span is anchored to, so that clears too
    app.cancel_gesture();
    app.clear_settled_selection();
    app.hover = None;
}

/// Route a mouse-down over selectable text: it arms the text gesture, carrying its
/// multi-click count. The count acts at the release, so a press inside the double-click
/// window that drags away is still a plain drag selection. Returns whether the event landed
/// on text.
fn handle_text_down(app: &mut App, m: MouseEvent, area: Rect) -> bool {
    use crate::selection::{Gesture, Point, Surface, TextDrag};

    let arm = |app: &mut App, surface: Surface, point: Point| {
        let count = app.note_click(m.column, m.row, point.row);
        app.gesture =
            Gesture::Text { drag: TextDrag { surface, anchor: point, extent: point }, count };
    };
    if let Some(point) = ui::read_point_at(area, app, m.column, m.row) {
        arm(app, Surface::Read, point);
        return true;
    }
    if let Some(point) = ui::painted_point(area, app, m.column, m.row, false) {
        arm(app, Surface::Painted, point);
        return true;
    }
    if let Some(i) = ui::hit_file(area, app, m.column, m.row, app.file_rows.len(), app.file_scroll)
    {
        arm(app, Surface::Files, Point { row: i, chr: 0 });
        return true;
    }
    false
}

/// Extend the active text drag to the pointer: scroll while the pointer sits past the pane's
/// content, then move the extent.
fn text_drag_extend(app: &mut App, m: MouseEvent, area: Rect) {
    text_drag_edge_scroll(app, m, area);
    text_drag_set_extent(app, m, area);
}

/// The vertical scroll for a drag pointer against `inner`'s rows: past them — on the border
/// or beyond — scrolls, so the outermost content rows stay selectable without scrolling
fn edge_delta(row: u16, inner: Rect) -> isize {
    if row < inner.y {
        return -1;
    }
    isize::from(row >= inner.y + inner.height)
}

/// Whether the pointer's cell sits on the pane edge — the position half of the
/// exit signature. some hosts deliver mouse events from anywhere
/// inside the pane, so a release anywhere further in would have arrived; only the outermost
/// cells can precede a lost release. Public for the gesture tests, like [`handle_mouse`].
#[must_use]
pub fn pointer_at_pane_edge(m: MouseEvent, area: Rect) -> bool {
    m.row <= area.y
        || m.row >= area.y + area.height.saturating_sub(1)
        || m.column <= area.x
        || m.column >= area.x + area.width.saturating_sub(1)
}

/// Scroll the active drag's pane while the pointer sits past its content rows.
fn text_drag_edge_scroll(app: &mut App, m: MouseEvent, area: Rect) {
    use crate::selection::Surface;
    let Some(drag) = app.text_drag() else { return };
    match drag.surface {
        Surface::Read => read_edge_scroll(app, m, area, true),
        Surface::Files => {
            let inner = ui::files_inner_rect(area, app);
            let delta = edge_delta(m.row, inner);
            if inner.height > 0 && delta != 0 {
                app.wheel_files(delta);
            }
        }
        Surface::Painted => {
            // The painted rect, not the pane's inner rect: a `PR` notice sits above the
            // content, and its rows must scroll a drag, not dead-zone it.
            let Some(rect) = ui::painted_sel(app, area).map(|s| s.rect) else { return };
            let delta = edge_delta(m.row, rect);
            if rect.height > 0 && delta != 0 {
                app.wheel_diff(delta); // the preview's scroll path
            }
        }
    }
}

/// Move the active drag's extent to the pointer, clamped into its surface (TS-ONE-SURFACE).
/// The wheel arms call this alone — their scroll already happened, and adding the edge scroll
/// on top would make wheel speed depend on where the pointer rests.
fn text_drag_set_extent(app: &mut App, m: MouseEvent, area: Rect) {
    use crate::selection::{Point, Surface};
    let Some(drag) = app.text_drag() else { return };
    let extent = match drag.surface {
        Surface::Read => ui::read_point_clamped(area, app, m.column, m.row),
        Surface::Painted => ui::painted_point(area, app, m.column, m.row, true),
        Surface::Files => {
            let inner = ui::files_inner_rect(area, app);
            if inner.height == 0 || app.file_rows.is_empty() {
                None
            } else {
                let y = m.row.clamp(inner.y, inner.y + inner.height - 1);
                let i = ((y - inner.y) as usize + app.file_scroll).min(app.file_rows.len() - 1);
                Some(Point { row: i, chr: 0 })
            }
        }
    };
    if let Some(p) = extent
        && let crate::selection::Gesture::Text { drag, .. } = &mut app.gesture
    {
        drag.extent = p;
    }
}

/// Scroll the read pane while a drag holds the pointer past its content rows (the find band
/// counts as chrome, not content): rows on the border or beyond, and with `horizontal` set
/// and wrap off the same for columns.
fn read_edge_scroll(app: &mut App, m: MouseEvent, area: Rect, horizontal: bool) {
    let content = ui::read_content_rect(area, app);
    if content.height == 0 {
        return;
    }
    let delta = edge_delta(m.row, content);
    if delta != 0 {
        app.wheel_diff(delta);
        // The same event's extent update maps against the post-scroll layout.
        ui::refresh_read_layout(app, area);
    }
    if horizontal && !app.wrap {
        if m.column < content.x {
            app.h_scroll = app.h_scroll.saturating_sub(2);
        } else if m.column >= content.x + content.width {
            // Capped at the widest visible row's last column, so a held border drag cannot
            // strand the view past all content — and never pulled back, so a keyboard
            // scroll already past the cap keeps its place.
            let cap = ui::widest_visible_row(app, area).saturating_sub(1);
            if app.h_scroll < cap {
                app.h_scroll = (app.h_scroll + 2).min(cap);
            }
        }
    }
}

/// Finish a text drag at its release: a release whose point never left the anchor's
/// performs the click, or the multi-click's copy — a navigator row, the word under the
/// cell, or the triple's whole line; anything else copies the selection. `clicks_act` is
/// false while composing, whose pane clicks are inert — the multi-click copies still fire
/// there, being selection copies, not pane clicks.
fn finish_text_drag(
    app: &mut App,
    m: MouseEvent,
    area: Rect,
    heights: &[usize],
    clicks_act: bool,
    target: &dyn crate::export::ExportTarget,
) -> Result<()> {
    use crate::selection::{Gesture, Surface};
    let Gesture::Text { count, .. } = app.gesture else { return Ok(()) };
    // One predicate decides the whole release: the extent moves to the release point, and a
    // release whose point never left the anchor's is the click — which grants the slop the
    // spec names for free (a navigator row, a wide character's cells, a tab's expansion all
    // map many cells onto one point). Anything else is a drag with a selection, and copies
    text_drag_set_extent(app, m, area);
    let drag = app.text_drag().expect("matched above");
    if drag.anchor == drag.extent {
        app.gesture = Gesture::None;
        match drag.surface {
            // A navigator double copies the row's path or text, and a triple repeats it;
            // an empty row falls back to the click, so the gesture is never a silent
            // no-op.
            Surface::Files if count >= 2 => {
                if !multi_click_copy(app, area, drag, target) && clicks_act {
                    perform_click(app, m, area, heights, drag)?;
                }
            }
            // A double on a character surface copies the word under the cell; whitespace
            // and wordless cells act as the click.
            Surface::Read | Surface::Painted if count == 2 => {
                if !word_click_copy(app, area, drag, target) && clicks_act {
                    perform_click(app, m, area, heights, drag)?;
                }
            }
            // A triple on a character surface copies the row's whole source line; an
            // empty line acts as the click.
            Surface::Read | Surface::Painted if count >= 3 => {
                if !line_click_copy(app, area, drag, target) && clicks_act {
                    perform_click(app, m, area, heights, drag)?;
                }
            }
            _ if clicks_act => perform_click(app, m, area, heights, drag)?,
            _ => {}
        }
        // The release continues the multi-click chain; only a non-release end resets it.
        app.lift_gesture_freeze();
    } else {
        complete_gesture(app, area, target);
    }
    Ok(())
}

/// The navigator double-click copy, fired at the release: a file row's repo-relative path, a
/// `PR` row's full text. Returns whether anything copied.
fn multi_click_copy(
    app: &mut App,
    area: Rect,
    drag: crate::selection::TextDrag,
    target: &dyn crate::export::ExportTarget,
) -> bool {
    use crate::selection::Point;
    let row = drag.anchor.row;
    let whole_row = Point { row, chr: usize::MAX };
    let t = surface_text(app, area, drag.surface, Point { row, chr: 0 }, whole_row);
    // An empty row yields empty text: fall back to the click, so the gesture is never a
    // silent no-op.
    if t.is_empty() {
        return false;
    }
    app.copy_selection_text(target, &t);
    app.settle_selection(drag, t);
    true
}

/// The word double-click copy on the character surfaces, fired at the release: the word
/// under the cell copies and its highlight settles. Whitespace, punctuation, and cells past
/// the text return `false`, falling back to the click.
fn word_click_copy(
    app: &mut App,
    area: Rect,
    drag: crate::selection::TextDrag,
    target: &dyn crate::export::ExportTarget,
) -> bool {
    use crate::selection::{Point, TextDrag};
    let row = drag.anchor.row;
    let whole_row = Point { row, chr: usize::MAX };
    let line = surface_text(app, area, drag.surface, Point { row, chr: 0 }, whole_row);
    let Some((s, e)) = crate::selection::token_at(&line, drag.anchor.chr) else {
        return false;
    };
    let word: String = line.chars().skip(s).take(e - s + 1).collect();
    app.copy_selection_text(target, &word);
    app.settle_selection(
        TextDrag {
            surface: drag.surface,
            anchor: Point { row, chr: s },
            extent: Point { row, chr: e },
        },
        word,
    );
    true
}

/// The line triple-click copy on the character surfaces, fired at the release: the row's
/// whole source line copies and its highlight settles. An empty line returns `false`,
/// falling back to the click.
fn line_click_copy(
    app: &mut App,
    area: Rect,
    drag: crate::selection::TextDrag,
    target: &dyn crate::export::ExportTarget,
) -> bool {
    use crate::selection::{Point, TextDrag};
    let row = drag.anchor.row;
    let whole_row = Point { row, chr: usize::MAX };
    let line = surface_text(app, area, drag.surface, Point { row, chr: 0 }, whole_row);
    if line.is_empty() {
        return false;
    }
    let last = line.chars().count() - 1;
    app.copy_selection_text(target, &line);
    app.settle_selection(
        TextDrag {
            surface: drag.surface,
            anchor: Point { row, chr: 0 },
            extent: Point { row, chr: last },
        },
        line,
    );
    true
}

/// The active drag's clipboard text. Public for the gesture tests, like [`handle_mouse`].
pub fn drag_text(app: &App, area: Rect) -> Option<String> {
    let drag = app.text_drag()?;
    let (a, b) = drag.ordered();
    Some(surface_text(app, area, drag.surface, a, b))
}

/// Complete the live gesture — the drag's own off-cell release, the release proofs, the
/// exit deadline, and a config layout or theme change: a drag with a visible selection copies it to `target`
/// (`TS-NO-SILENT-LOSS`), a press that never moved and a gutter gesture dissolve with
/// nothing. Public for the gesture tests.
pub fn complete_gesture(app: &mut App, area: Rect, target: &dyn crate::export::ExportTarget) {
    if let Some(drag) = app.text_drag()
        && drag.anchor != drag.extent
    {
        let text = drag_text(app, area).unwrap_or_default();
        app.copy_selection_text(target, &text);
        // The copy leaves its span highlighted as feedback.
        app.settle_selection(drag, text);
    }
    app.cancel_gesture();
}

/// The copied text for a span on `surface` — the one extractor behind the drag release and
/// the multi-click copies, so a new surface cannot be extractable in one and forgotten in
/// the other.
fn surface_text(
    app: &App,
    area: Rect,
    surface: crate::selection::Surface,
    a: crate::selection::Point,
    b: crate::selection::Point,
) -> String {
    use crate::selection::{Surface, lines_text};
    match surface {
        Surface::Read => crate::selection::read_text(&app.visible, a, b),
        Surface::Files => crate::selection::files_text(&app.file_rows, &app.entries, a.row, b.row),
        Surface::Painted => lines_text(&ui::painted_texts(app, area), a, b),
    }
}

/// The click a same-cell release performs — the pre-selection mouse-down meanings
fn perform_click(
    app: &mut App,
    m: MouseEvent,
    area: Rect,
    heights: &[usize],
    drag: crate::selection::TextDrag,
) -> Result<()> {
    use crate::selection::Surface;
    match drag.surface {
        // The navigators act on the drag's already-clamped row, not the raw release cell:
        // the row slop that classified the release as a click also delivers it
        Surface::Files => app.select_file(drag.extent.row)?,
        Surface::Read | Surface::Painted => {
            if let Some(url) = app.painted_link_at(m.column, m.row) {
                app.focus = Focus::Diff;
                app.open_link(&url);
            } else if let Some(summary) = app.painted_details_at(m.column, m.row) {
                app.focus = Focus::Diff;
                app.toggle_details(&summary);
            } else if app.preview_active() {
                // The painted surfaces have no cursor: a click only focuses the pane.
                if ui::in_diff_pane(area, app, m.column, m.row) {
                    app.focus = Focus::Diff;
                }
            } else if let Some(i) =
                ui::hit_diff(area, app, m.column, m.row, heights, app.diff_scroll)
            {
                app.focus = Focus::Diff;
                app.diff_cursor = i;
                app.select_anchor = None;
                app.toggle_fold();
            }
        }
    }
    Ok(())
}

/// Map one mouse event onto `App`. Header hit-testing uses `keymap` — the keymap of the frame
/// on screen — so a config swap at the click boundary cannot shift the spans under the pointer.
/// Public for the dispatch tests, like [`handle_key`]; the event loop is the runtime caller.
pub fn handle_mouse(
    app: &mut App,
    m: MouseEvent,
    area: Rect,
    heights: &[usize],
    keymap: &Keymap,
    target: &dyn crate::export::ExportTarget,
) -> Result<()> {
    app.settle_read_pending();
    app.hover = Some((m.column, m.row));
    // Pointer motion with no button held, or a fresh mouse-down, proves an active gesture's
    // release was lost — the host may route mouse by pointer position, so a release over another
    // pane never arrives here. The proof completes the old gesture (a visible selection
    // copies, `TS-NO-SILENT-LOSS`), then the event acts as any event. A motionless held drag
    // feeds no events at all and stays alive. One guard for every dispatch path below
    if app.gesture_active() && matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Down(_)) {
        complete_gesture(app, area, target);
    }
    // The next mouse-down is the user doing something else: the settled highlight clears,
    // after the proof above so a down-completed gesture never leaves one behind either
    if matches!(m.kind, MouseEventKind::Down(_)) {
        app.clear_settled_selection();
    }
    // On the search screen: chips flip, a click picks (a second click on the picked row
    // opens), the wheel moves the pick over results and scrolls the preview, and the
    // divider drags search's own share. A cancelled divider
    // gesture still owns its remaining drag and mouse-up events like in every modal.
    if app.mode == Mode::Search {
        use ui::SearchTarget as T;
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) if app.divider_drag_active() => {
                // The share maps the pointer's row into the band-to-footer span the two
                // panes divide, matching `search_layout`'s geometry.
                let l = ui::search_layout(ui::body_rect(area, app), app);
                let axis_len = l.results.height + l.preview.height;
                let offset = m.row.saturating_sub(l.results.y);
                app.drag_search_divider(axis_len, offset);
            }
            MouseEventKind::Drag(MouseButton::Left) if app.divider_drag_captured() => {}
            MouseEventKind::Up(MouseButton::Left) if app.divider_drag_captured() => {
                app.finish_divider_drag();
            }
            MouseEventKind::Down(MouseButton::Left) => {
                match ui::search_target(app, area, m.column, m.row) {
                    Some(T::Chips) => app.search_flip(),
                    Some(T::Divider) => app.start_divider_drag(),
                    Some(T::Row(pick)) => {
                        let picked = app.search.as_ref().is_some_and(|s| s.pick == pick);
                        if picked {
                            app.search_open_pick()?;
                        } else if let Some(s) = app.search.as_mut() {
                            s.pick = pick;
                        }
                    }
                    _ => {}
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta: isize = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
                match ui::search_target(app, area, m.column, m.row) {
                    Some(T::Row(_) | T::Results) => app.search_move(delta),
                    Some(T::Preview | T::Divider) => app.scroll_search_preview(delta * 3),
                    _ => {}
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // A modal captures new mouse gestures, but a divider gesture cancelled by the key that
    // opened it still owns its remaining drag and mouse-up events. The theme picker is not
    // modal, but its popup captures the mouse the same way.
    // A card's open box on the Comments tab holds no place in a view the world can move, so
    // it frees the mouse: a click elsewhere saves it and selects there, and a tab saves it
    // and switches.
    if app.edited_card().is_some() {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                match ui::hit_header(area, app, keymap, m.column, m.row) {
                    Some(ui::HeaderHit::Tab(tab)) => {
                        if app.close_card_box() {
                            app.set_tab(tab)?;
                        }
                    }
                    Some(_) => {}
                    None => comments_click(app, m, area)?,
                }
            }
            MouseEventKind::ScrollDown if ui::in_files_pane(area, app, m.column, m.row) => {
                app.wheel_comment_nav(3);
            }
            MouseEventKind::ScrollUp if ui::in_files_pane(area, app, m.column, m.row) => {
                app.wheel_comment_nav(-3);
            }
            _ => {}
        }
        return Ok(());
    }
    if app.mode.is_modal() || app.mode == Mode::ThemePick {
        // Text selection stays available while the comment editor is open, selecting from the
        // frozen view under it; its clicks stay inert like the rest of the modal's pane
        if app.composing() {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) if !app.divider_drag_captured() => {
                    if handle_text_down(app, m, area) {
                        return Ok(());
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) if app.text_drag().is_some() => {
                    text_drag_extend(app, m, area);
                    return Ok(());
                }
                MouseEventKind::Up(MouseButton::Left) if app.text_drag().is_some() => {
                    finish_text_drag(app, m, area, heights, false, target)?;
                    return Ok(());
                }
                _ => {}
            }
        }
        match m.kind {
            // A click on either of the delete popup's buttons takes it.
            MouseEventKind::Down(MouseButton::Left)
                if matches!(app.mode, Mode::ConfirmDelete { .. }) =>
            {
                if let Some(delete) = ui::hit_delete_button(area, app, m.column, m.row) {
                    app.confirm_delete_choose(delete);
                    app.confirm_delete_pick();
                }
            }
            // Click to highlight, click the highlight to pick.
            MouseEventKind::Down(MouseButton::Left) if app.mode == Mode::BasePick => {
                match ui::hit_base_picker_row(area, app, m.column, m.row) {
                    Some(i) if app.base_picker.as_ref().is_some_and(|bp| bp.cursor == i) => {
                        app.base_picker_pick()?;
                    }
                    Some(i) => app.base_picker_goto(i),
                    None => {}
                }
            }
            // And in the commit picker, the run included.
            MouseEventKind::Down(MouseButton::Left) if app.mode == Mode::CommitPick => {
                match ui::hit_commit_picker_row(area, app, m.column, m.row) {
                    Some(i) if app.commit_picker.as_ref().is_some_and(|cp| cp.cursor == i) => {
                        app.commit_picker_pick()?;
                    }
                    Some(i) => app.commit_picker_goto(i),
                    None => {}
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if app.divider_drag_captured() => {
                return Ok(());
            }
            MouseEventKind::Up(MouseButton::Left) if app.divider_drag_captured() => {
                app.finish_divider_drag();
            }
            _ => {}
        }
        return Ok(());
    }
    // A mouse gesture is one of the "any other input" that drops an armed crossing: the reviewer
    // who reaches for the mouse has left the file's edge behind. Pointer motion
    // is not a gesture — capture reports every move over the pane, and a pointer resting on
    // the pane would otherwise disarm the crossing without the reviewer touching
    // anything.
    if !matches!(m.kind, MouseEventKind::Moved) {
        app.disarm_cross();
    }

    // Divider gestures are common to every tab. A cancelled drag remains consumed until its
    // mouse-up, so rotating or reconfiguring the layout cannot turn it into a line selection.
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if ui::hit_divider(area, app, m.column, m.row) => {
            app.start_divider_drag();
            return Ok(());
        }
        MouseEventKind::Drag(MouseButton::Left) if app.divider_drag_active() => {
            let body = ui::body_rect(area, app);
            let (axis_len, offset) = if app.navigator_position.stacked() {
                (body.height, m.row.saturating_sub(body.y))
            } else {
                (body.width, m.column.saturating_sub(body.x))
            };
            app.drag_divider(axis_len, offset);
            return Ok(());
        }
        MouseEventKind::Drag(MouseButton::Left) if app.divider_drag_cancelled() => return Ok(()),
        MouseEventKind::Up(MouseButton::Left) if app.divider_drag_captured() => {
            app.finish_divider_drag();
            return Ok(());
        }
        _ => {}
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(hit) = ui::hit_header(area, app, keymap, m.column, m.row) {
                match hit {
                    ui::HeaderHit::Tab(tab) => app.set_tab(tab)?,
                    ui::HeaderHit::Scope => app.set_scope(app.next_chip_scope())?,
                    // Inert when the picker cannot open here — with a `--base` flag the
                    // label names the base without offering a choice.
                    ui::HeaderHit::Base => app.open_base_picker(),
                    ui::HeaderHit::Pick => app.open_commit_picker(),
                }
            } else if app.tab == crate::app::Tab::Comments {
                comments_click(app, m, area)?;
            } else if let Some(row) = ui::note_row_at(area, app, m.column, m.row) {
                // A comment box opens for editing where it sits.
                app.click_comment(row);
            } else if let Some(row) = ui::gutter_row_at(area, app, m.column, m.row) {
                // The gutter owns mouse commenting: click a line or drag a range, and the
                // composer opens on release.
                app.start_gutter_drag(row);
            } else if handle_text_down(app, m, area) {
                // A pending click or text drag armed. Every painted preview cell is claimed
                // here, so link opens live in `perform_click`, at the release
            } else if app.preview_active() {
                // A preview click only focuses the pane. The pane-rect test, not the
                // source-row hit test — the rendered preview can be taller than the
                // source has rows.
                if ui::in_diff_pane(area, app, m.column, m.row) {
                    app.focus = Focus::Diff;
                }
            } else if let Some(i) =
                ui::hit_diff(area, app, m.column, m.row, heights, app.diff_scroll)
            {
                // Only non-text display lines reach here: a fold.
                app.focus = Focus::Diff;
                app.diff_cursor = i;
                app.select_anchor = None;
                // A click on a fold marker opens or hides it, the marker holding still.
                app.toggle_fold();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) if app.gutter_drag() => {
            // A gutter drag selects rows, so it never scrolls horizontally.
            read_edge_scroll(app, m, area, false);
            if let Some(p) = ui::read_point_clamped(area, app, m.column, m.row) {
                app.drag_select_to(p.row);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) if app.text_drag().is_some() => {
            text_drag_extend(app, m, area);
        }
        MouseEventKind::Up(MouseButton::Left) if app.gutter_drag() => {
            app.finish_gutter_drag();
        }
        MouseEventKind::Up(MouseButton::Left) if app.text_drag().is_some() => {
            finish_text_drag(app, m, area, heights, true, target)?;
        }
        // The wheel during an active drag scrolls the drag's pane and extends the selection
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            if app.text_drag().is_some() || app.gutter_drag() =>
        {
            let delta: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
            let files =
                app.text_drag().map(|d| d.surface) == Some(crate::selection::Surface::Files);
            if files {
                app.wheel_files(delta);
            } else {
                app.wheel_diff(delta);
                // The same event's extent update maps against the post-scroll layout.
                ui::refresh_read_layout(app, area);
            }
            if app.gutter_drag() {
                if let Some(p) = ui::read_point_clamped(area, app, m.column, m.row) {
                    app.drag_select_to(p.row);
                }
            } else {
                text_drag_set_extent(app, m, area);
            }
        }
        // The wheel scrolls the viewport of whichever pane it is over — never the cursor, so
        // a comment is never anchored to a wheeled-past line. Horizontal scroll is
        // keyboard-only (`←`/`→`), since multiplexers don't reliably deliver h-wheel events.
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            if app.tab == crate::app::Tab::Comments =>
        {
            let delta: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
            if ui::in_files_pane(area, app, m.column, m.row) {
                app.wheel_comment_nav(delta);
            } else {
                let heights = ui::card_heights(app, area);
                let viewport = ui::diff_viewport_height(area, app);
                app.comments.scroll_by(delta, &heights, viewport);
            }
        }
        MouseEventKind::ScrollDown if ui::in_files_pane(area, app, m.column, m.row) => {
            app.wheel_files(3);
        }
        MouseEventKind::ScrollUp if ui::in_files_pane(area, app, m.column, m.row) => {
            app.wheel_files(-3);
        }
        MouseEventKind::ScrollDown => app.wheel_diff(3),
        MouseEventKind::ScrollUp => app.wheel_diff(-3),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod refresh_tests {
    use super::{
        apply_plugin_config_observation, glyph_clears, handle_blocked_event, handle_resize,
        ready_app, world_indicator, world_wake,
    };
    use crate::app::App;
    use crate::config::{Config, plugin_config_in};
    use crate::model::Scope;
    use ratatui::crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use std::sync::mpsc;
    use std::time::Duration;
    #[test]
    fn the_indicator_lights_only_for_a_building_job_past_the_delay() {
        use std::time::Duration;
        assert!(!world_indicator(None), "nothing in flight, nothing lit");
        assert!(
            !world_indicator(Some((Duration::from_millis(500), false))),
            "sample-only jobs never light it"
        );
        assert!(!world_indicator(Some((Duration::from_millis(100), true))), "below the delay");
        assert!(
            world_indicator(Some((Duration::from_millis(200), true))),
            "a building job past the delay lights it"
        );
    }

    #[test]
    fn the_lit_glyph_holds_its_minimum_display() {
        use std::time::Duration;
        assert!(!glyph_clears(Duration::from_millis(100)), "a fast landing keeps the glyph lit");
        assert!(glyph_clears(Duration::from_millis(300)), "past the hold it goes dark");
    }

    #[test]
    fn the_in_flight_wake_is_tight_only_for_a_building_job() {
        use std::time::Duration;
        assert_eq!(world_wake(true), Duration::from_millis(15));
        assert_eq!(world_wake(false), Duration::from_millis(100));
    }
    #[test]
    fn terminal_resize_cancels_the_active_divider_coordinates() {
        let mut app = App::new(std::path::PathBuf::from("."), Scope::Uncommitted, None);
        app.start_divider_drag();

        handle_resize(&mut app);

        assert!(app.divider_drag_cancelled());
    }

    #[test]
    fn terminal_resize_cancels_a_live_text_drag_without_copying() {
        use crate::selection::{Gesture, Point, Surface, TextDrag};
        let mut app = App::new(std::path::PathBuf::from("."), Scope::Uncommitted, None);
        let drag = TextDrag {
            surface: Surface::Read,
            anchor: Point { row: 0, chr: 0 },
            extent: Point { row: 0, chr: 4 },
        };
        app.gesture = Gesture::Text { drag, count: 1 };
        app.settle_selection(drag, "alpha".into());

        handle_resize(&mut app);

        // The reflow row of the gesture end table: nothing copies, the input acts — and
        // the settled span clears, since the resize re-wraps the rows it anchors to
        assert!(!app.gesture_active(), "a resize ends the gesture");
        assert_eq!(app.status, "", "a resize-cancelled drag copies nothing");
        assert!(app.settled_selection().is_none(), "a resize clears the settled highlight");
    }
    #[test]
    fn blocked_frames_ignore_normal_events_but_keep_quit_and_capture_cleanup() {
        let mut app = App::new(std::path::PathBuf::from("."), Scope::Uncommitted, None);
        app.mode = crate::app::Mode::Composing { editing: None };
        app.input = "draft".to_string();
        app.start_divider_drag();
        app.set_config_error("invalid config".to_string());
        assert!(app.divider_drag_cancelled());

        handle_blocked_event(&mut app, &Event::Paste(" hidden paste".to_string()));
        handle_blocked_event(
            &mut app,
            &Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(app.input, "draft");
        assert!(!app.should_quit);

        handle_blocked_event(
            &mut app,
            &Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert!(!app.divider_drag_cancelled());

        handle_blocked_event(&mut app, &Event::Key(KeyEvent::from(KeyCode::Char('q'))));
        assert!(app.should_quit);
    }
    #[test]
    fn shell_only_config_changes_do_not_invalidate_runtime_work() {
        let repo = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        std::fs::write(config_dir.path().join("config.toml"), "auto_open = false\n").unwrap();
        let cfg = Config::parse([repo.path().display().to_string()]);
        let mut app = App::new(repo.path().to_path_buf(), Scope::Uncommitted, None);
        let (tx, _rx) = mpsc::channel();
        let mut epoch = 0;
        let mut recovery_inflight = false;

        assert!(apply_plugin_config_observation(
            &mut app,
            &cfg,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
            plugin_config_in(config_dir.path()),
        ));
        assert_eq!(epoch, 0);
        assert!(!app.plugin_config().unwrap().auto_open());
    }

    #[test]
    fn default_scope_seeds_a_fresh_pane_and_a_reread_never_switches_it() {
        let repo = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let path = config_dir.path().join("config.toml");
        std::fs::write(&path, "default_scope = \"branch\"\n").unwrap();
        let cfg = Config::parse([repo.path().display().to_string()]);
        let mut app = ready_app(&cfg, plugin_config_in(config_dir.path()).unwrap());
        assert_eq!(app.scope, Scope::Branch, "startup seeds the configured scope");

        // The user switches in-session; a reread with a different default must not move it.
        app.set_scope(Scope::Uncommitted).unwrap();
        std::fs::write(&path, "default_scope = \"uncommitted\"\n").unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut epoch = 0;
        let mut recovery_inflight = false;
        assert!(apply_plugin_config_observation(
            &mut app,
            &cfg,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
            plugin_config_in(config_dir.path()),
        ));
        assert_eq!(app.scope, Scope::Uncommitted, "a reread never switches the active scope");
        assert_eq!(epoch, 0, "a default_scope change invalidates no running work");
    }

    #[test]
    fn whole_file_seeds_a_fresh_pane_and_a_reread_never_flips_it() {
        let repo = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let path = config_dir.path().join("config.toml");
        std::fs::write(&path, "whole_file = false\n").unwrap();
        let cfg = Config::parse([repo.path().display().to_string()]);
        let mut app = ready_app(&cfg, plugin_config_in(config_dir.path()).unwrap());
        assert!(!app.whole_file, "startup seeds the configured view");

        std::fs::write(&path, "whole_file = true\n").unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut epoch = 0;
        let mut recovery_inflight = false;
        assert!(apply_plugin_config_observation(
            &mut app,
            &cfg,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
            plugin_config_in(config_dir.path()),
        ));
        assert!(!app.whole_file, "a reread never flips the running view");
    }

    #[test]
    fn only_a_layout_or_theme_change_or_a_block_ends_a_gesture() {
        use crate::config::plugin_config_in;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = \"gruvbox\"\n").unwrap();
        let previous = plugin_config_in(dir.path()).unwrap();

        // The same config, and a change that reflows nothing, leave the gesture alone.
        assert!(!super::config_ends_gesture(&previous, Some(&previous)));
        std::fs::write(&path, "theme = \"gruvbox\"\ndefault_scope = \"branch\"\n").unwrap();
        let scoped = plugin_config_in(dir.path()).unwrap();
        assert!(!super::config_ends_gesture(&previous, Some(&scoped)));

        // A theme or layout change reflows the frame, and a failed observation blocks the
        // body: each ends the gesture with its copy.
        std::fs::write(&path, "theme = \"nord\"\n").unwrap();
        let themed = plugin_config_in(dir.path()).unwrap();
        assert!(super::config_ends_gesture(&previous, Some(&themed)));
        std::fs::write(&path, "theme = \"gruvbox\"\nnavigator_position = \"left\"\n").unwrap();
        let moved = plugin_config_in(dir.path()).unwrap();
        assert!(super::config_ends_gesture(&previous, Some(&moved)));
        assert!(super::config_ends_gesture(&previous, None));
    }

    #[test]
    fn a_config_theme_change_ends_a_live_gesture_through_the_observation_boundary() {
        use crate::selection::{Gesture, Point, Surface, TextDrag};
        let repo = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let path = config_dir.path().join("config.toml");
        std::fs::write(&path, "theme = \"gruvbox\"\n").unwrap();
        let mut cfg = Config::parse([repo.path().display().to_string()]);
        cfg.plugin_config_dir = Some(config_dir.path().to_path_buf());
        let mut app = App::new(repo.path().to_path_buf(), Scope::Uncommitted, None);
        app.set_plugin_config(crate::config::plugin_config_in(config_dir.path()).unwrap());
        let (tx, _rx) = mpsc::channel();
        let mut epoch = 0;
        let mut recovery_inflight = false;
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        // A pristine press, so the completion copies nothing and no clipboard runs.
        let press = Gesture::Text {
            drag: TextDrag {
                surface: Surface::Read,
                anchor: Point { row: 0, chr: 0 },
                extent: Point { row: 0, chr: 0 },
            },
            count: 1,
        };

        // An unchanged observation leaves the gesture alone.
        app.gesture = press;
        super::reconcile_plugin_config(
            &mut app,
            &cfg,
            area,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
        );
        assert!(app.gesture_active(), "an unchanged config leaves the gesture alive");

        // The end table's config row: a theme change ends the gesture at the boundary,
        // before the new frame applies.
        std::fs::write(&path, "theme = \"nord\"\n").unwrap();
        super::reconcile_plugin_config(
            &mut app,
            &cfg,
            area,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
        );
        assert!(!app.gesture_active(), "the theme change completes the gesture");
        assert_eq!(app.plugin_config().unwrap().theme(), "nord");
    }

    #[test]
    fn invalid_then_valid_observation_blocks_and_recovers_through_a_fresh_worker() {
        let repo = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let path = config_dir.path().join("config.toml");
        std::fs::write(&path, "unknown = true\n").unwrap();
        let cfg = Config::parse([repo.path().display().to_string()]);
        let mut app = App::new(repo.path().to_path_buf(), Scope::Uncommitted, None);
        let (tx, rx) = mpsc::channel();
        let mut epoch = 0;
        let mut recovery_inflight = false;

        assert!(!apply_plugin_config_observation(
            &mut app,
            &cfg,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
            plugin_config_in(config_dir.path()),
        ));
        assert!(app.plugin_config().is_none());
        assert!(app.config_error().unwrap().contains("unknown key"));

        std::fs::write(&path, "theme = \"gruvbox\"\n").unwrap();
        assert!(!apply_plugin_config_observation(
            &mut app,
            &cfg,
            &mut epoch,
            &tx,
            &mut recovery_inflight,
            plugin_config_in(config_dir.path()),
        ));
        let (recovery_epoch, target, recovered) =
            rx.recv_timeout(Duration::from_secs(5)).expect("recovery worker");
        assert_eq!(recovery_epoch, epoch);
        assert_eq!(target.theme(), "gruvbox");
        assert_eq!(recovered.plugin_config().unwrap().theme(), "gruvbox");
    }
}
