//! Render tests: drive `ui::render` through ratatui's `TestBackend` and assert on
//! the painted buffer, so the layout and component wiring are checked for real.

mod common;

use common::{Repo, app_on, enter_tab};
use diff_reckoner::app::{App, Focus, Tab};
use diff_reckoner::config::NavigatorPosition;
use diff_reckoner::keymap::Keymap;
use diff_reckoner::model::Scope;
use diff_reckoner::ui::{self, HeaderHit};
use diff_reckoner::{handle_key, handle_mouse};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

fn dump(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    out
}

fn render(app: &App) -> String {
    dump(&render_size(app, 140, 40))
}

/// Render and return the buffer, for cell-style assertions.
fn render_buffer(app: &App) -> Buffer {
    render_size(app, 140, 40)
}

/// Render at a specific width (height fixed), for footer fit-to-width assertions.
fn render_at(app: &App, width: u16) -> String {
    dump(&render_size(app, width, 12))
}

fn render_size(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// Catppuccin surface2 — the shared selection/cursor fill.
const SELECTION_BG: ratatui::style::Color = ratatui::style::Color::Rgb(0x58, 0x5b, 0x70);
/// Catppuccin orange — the comment-editor caret block.
const PEACH: ratatui::style::Color = ratatui::style::Color::Rgb(0xfa, 0xb3, 0x87);

/// The right `100-pct`% of every frame row, for pane-scoped assertions — one home for
/// the column math, so the two panes' cut points can't drift apart silently.
fn right_column(out: &str, pct: usize) -> String {
    out.lines()
        .map(|l| l.chars().skip(l.chars().count() * pct / 100).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first painted link region anywhere on the test frame, scanned over its grid.
fn first_painted_link(app: &App) -> Option<std::sync::Arc<str>> {
    (0..40u16)
        .flat_map(|y| (0..140u16).map(move |x| (x, y)))
        .find_map(|(x, y)| app.painted_link_at(x, y))
}

/// Open the comment composer on the first changed line of `edited_app`.
fn composing(app: &mut App) {
    app.focus = Focus::Diff;
    app.diff_cursor = app.visible.iter().position(|r| r.marker() == '+').unwrap();
    app.start_comment();
}

#[test]
fn invalid_config_replaces_the_entire_pane_with_its_error() {
    let mut app = edited_app();
    app.set_config_error(
        "config /tmp/reviewr/config.toml: invalid value for `theme`; expected a built-in theme name"
            .to_string(),
    );

    let out = render(&app);

    assert!(out.contains("config /tmp/reviewr/config.toml"));
    assert!(out.contains("expected a built-in theme name"));
    assert!(out.contains("The config reloads automatically."));
    assert!(!out.contains("Changes"), "normal reviewr chrome must be hidden");
}

#[test]
fn the_empty_comment_box_shows_a_placeholder() {
    let mut app = edited_app();
    composing(&mut app);
    assert!(render(&app).contains("Leave a comment…"), "an empty box shows the placeholder");
}

#[test]
fn the_caret_block_sits_on_the_character_at_the_caret() {
    let mut app = edited_app();
    composing(&mut app);
    app.input_push('a');
    app.input_push('b');
    app.caret_left(); // caret between 'a' and 'b' → block over 'b'
    let buf = render_buffer(&app);
    let mut found = false;
    for y in 0..40 {
        for x in 0..140 {
            if buf.cell((x, y)).is_some_and(|c| c.bg == PEACH && c.symbol() == "b") {
                found = true;
            }
        }
    }
    assert!(found, "the caret block highlights the character at the caret");
}

#[test]
fn backspacing_a_wide_character_leaves_the_terminal_cursor_unpainted() {
    let mut app = edited_app();
    composing(&mut app);
    app.input_push('日');
    app.input_push('本');
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| ui::render(f, &app)).unwrap();

    let before = terminal.backend().cursor_position();
    app.input_backspace();
    terminal.draw(|f| ui::render(f, &app)).unwrap();

    let cursor = terminal.backend().cursor_position();
    assert_eq!(
        (cursor.x + 2, cursor.y),
        (before.x, before.y),
        "the cursor retreats one wide character"
    );
    let cell = terminal.backend().buffer().cell(cursor).unwrap();
    assert_eq!(cell.bg, app.palette().base, "an empty cell, on the theme's background");
    assert_eq!(cell.symbol(), " ");
}

#[test]
fn a_height_capped_composer_scrolls_to_keep_the_caret_visible() {
    let mut app = edited_app();
    composing(&mut app);
    for _ in 0..599 {
        app.input_push('x');
    }
    app.input_push('z'); // the unique last character locates the caret in the buffer
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal.draw(|f| ui::render(f, &app)).unwrap();

    let buffer = terminal.backend().buffer();
    let cursor = terminal.backend().cursor_position();
    let (zx, zy) = (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .find(|&(x, y)| buffer.cell((x, y)).unwrap().symbol() == "z")
        .expect("the box scrolled the last typed character into view");
    // The cursor sits where the next character lands: right after `z`, or on the first
    // text column of the fresh row below when `z` exactly filled its row.
    let inline = (cursor.x, cursor.y) == (zx + 1, zy);
    let row_start =
        (0..buffer.area.width).find(|&x| buffer.cell((x, zy)).unwrap().symbol() == "x").unwrap();
    let wrapped = (cursor.x, cursor.y) == (row_start, zy + 1);
    assert!(
        inline || wrapped,
        "the cursor sits after the text (cursor {cursor:?}, z at ({zx},{zy}))"
    );
    assert_eq!(
        buffer.cell(cursor).unwrap().symbol(),
        " ",
        "end of input leaves the cursor cell blank"
    );
}

#[test]
fn the_find_band_anchors_the_terminal_cursor_at_its_caret() {
    let r = Repo::init();
    r.write("base.txt", "x\n");
    r.commit_all("init");
    r.write("m.rs", "let total = 1;\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;
    let keymap = Keymap::default();
    let area = Rect::new(0, 0, 140, 40);
    handle_key(&mut app, KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL), area, &keymap)
        .unwrap();

    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    let empty = terminal.backend().cursor_position();

    handle_key(&mut app, KeyEvent::from(KeyCode::Char('日')), area, &keymap).unwrap();
    terminal.draw(|f| ui::render(f, &app)).unwrap();

    let after = terminal.backend().cursor_position();
    assert_eq!(
        (after.x, after.y),
        (empty.x + 2, empty.y),
        "the cursor advances one wide character"
    );
}

#[test]
fn caret_vertical_moves_between_wrapped_rows() {
    // "abcdef" hard-wraps at width 3 to "abc"/"def"; caret 4 (def col 1) up → 1; 1 down → 4.
    assert_eq!(ui::caret_vertical("abcdef", 4, 3, false), 1);
    assert_eq!(ui::caret_vertical("abcdef", 1, 3, true), 4);
    // Composer wrapping preserves repeated spaces so every caret index remains addressable.
    assert_eq!(ui::caret_vertical("ab  cd", 4, 2, false), 2);
    assert_eq!(ui::caret_vertical("ab  cd", 2, 2, true), 4);
    // A line exactly filling the width adds no phantom row, so one step crosses it.
    assert_eq!(ui::caret_vertical("abc\ndef", 0, 3, true), 4);
    assert_eq!(ui::caret_vertical("abc\ndef", 4, 3, false), 0);
    // The caret past the full line sits visually on the next row, and motion agrees.
    assert_eq!(ui::caret_vertical("abc\ndef", 3, 3, false), 0);
    assert_eq!(ui::caret_vertical("abc\ndef", 3, 3, true), 7);
}

#[test]
fn the_fold_hint_names_the_expand_binding() {
    use std::fmt::Write as _;
    let r = Repo::init();
    let mut body = String::new();
    for i in 0..30 {
        let _ = writeln!(body, "line {i}");
    }
    r.write("f.rs", &body);
    r.commit_all("init");
    r.write("f.rs", &body.replace("line 15", "LINE 15")); // one change, long runs fold
    let mut app = app_on(&r);
    app.focus = Focus::Diff;
    app.diff_cursor = app.visible.iter().position(|row| row.hidden() > 0).expect("a fold row");

    let out = render(&app);
    assert!(out.contains("→ expand"), "the fold hint names the `→` key");
    assert!(!out.contains("⏎ expand"), "no stale enter hint remains");

    // A rebound `expand` renames the fold row's inline label and the footer hint alike
    // (a hint shows the action's first bound key).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[keybindings]\nexpand = [\"x\"]\n").unwrap();
    app.set_plugin_config(diff_reckoner::config::plugin_config_in(dir.path()).unwrap());
    let out = render(&app);
    assert!(out.contains("x expand"), "the rebound key names the hint:\n{out}");
    assert!(!out.contains("→ expand"), "the freed arrow leaves the hint");
}

fn edited_app() -> App {
    let r = Repo::init();
    r.write("hello.rs", "alpha\nbeta\n");
    r.commit_all("init");
    r.write("hello.rs", "alpha\nBETA\n");
    // The repo is only needed through reload(); rendering reads cached state, so
    // `r` can drop here and clean up its tempdir.
    app_on(&r)
}

#[test]
fn the_file_list_renders_as_a_directory_tree() {
    let r = Repo::init();
    r.write("src/app.rs", "x\n");
    r.write("src/ui.rs", "y\n");
    r.write("Cargo.toml", "[package]\n");
    r.commit_all("init");
    r.write("src/app.rs", "x2\n");
    r.write("src/ui.rs", "y2\n");
    r.write("Cargo.toml", "[package]\nname='z'\n");
    let app = app_on(&r);

    // Scan only the default-right navigator so the diff header — which does show
    // the open file's full path — doesn't confuse the assertions.
    let files_pane = right_column(&render(&app), 70);
    assert!(files_pane.contains("src/"), "the directory groups its files: {files_pane:?}");
    assert!(files_pane.contains("app.rs") && files_pane.contains("ui.rs"), "files by basename");
    assert!(!files_pane.contains("src/app.rs"), "a grouped file is not shown by full path");
    assert!(files_pane.contains("Cargo.toml"), "the top-level file shows too");
}

#[test]
fn an_expanded_directory_nests_its_children() {
    // All files paints unchanged rows without a marker. Those two columns must still
    // hold the chevron's width, or child names line up with the parent.
    let r = Repo::init();
    r.write("src/app.rs", "x\n");
    r.write("src/ui.rs", "y\n");
    r.write("tests/a.rs", "a\n");
    r.write("tests/b.rs", "b\n");
    r.write("README.md", "hi\n");
    r.commit_all("init");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    app.focus = Focus::Files;
    app.file_cursor = app.file_rows.iter().position(|r| r.dir_path() == Some("src")).unwrap();
    app.expand_dir();

    let buf = render_buffer(&app);
    // Search from the files pane so a left-pane path cannot steal the match.
    let files_x0 = 140 - 140 * 32 / 100 + 1;
    let src = token_x(&buf, "src/", files_x0);
    let tests = token_x(&buf, "tests/", files_x0);
    let readme = token_x(&buf, "README.md", files_x0);
    let app_rs = token_x(&buf, "app.rs", files_x0);
    assert_eq!(src, tests, "sibling directories share a name column");
    assert_eq!(src, readme, "a root file name lines up with a root directory");
    assert!(app_rs > src, "a child file sits to the right of its parent: {app_rs} vs {src}");
}

/// First painted column of `token` in `buf` at or after `x0`. Panics if it never appears.
fn token_x(buf: &Buffer, token: &str, x0: u16) -> u16 {
    let chars: Vec<char> = token.chars().collect();
    let n = chars.len() as u16;
    for y in 0..buf.area.height {
        for x in x0..buf.area.width.saturating_sub(n) {
            let hit = (0..n).all(|i| {
                buf.cell((x + i, y)).is_some_and(|c| c.symbol() == chars[i as usize].to_string())
            });
            if hit {
                return x;
            }
        }
    }
    panic!("{token} was not painted at x>={x0}");
}

#[test]
fn a_saved_comment_renders_inline_as_a_card() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nBETA\n");
    let mut app = app_on(&r);

    app.focus = Focus::Diff;
    app.diff_cursor = app.visible.iter().position(|row| row.marker() == '+').unwrap();
    app.start_comment();
    for ch in "memoize this".chars() {
        app.input_push(ch);
    }
    app.submit_comment(); // box closes, comment saved

    let out = render(&app);
    assert!(out.contains("memoize this"), "the saved comment stays visible inline: {out:?}");
    assert!(out.contains("comment ·"), "the inline card is titled with the location");
}

#[test]
fn a_renamed_file_shows_old_arrow_new_in_the_header() {
    let r = Repo::init();
    r.write("old_name.rs", "stable contents that survive the move\nplus a second line\n");
    r.commit_all("init");
    r.git(&["mv", "old_name.rs", "new_name.rs"]);
    r.write("new_name.rs", "stable contents that survive the move\nplus an edited line\n");
    let app = app_on(&r);

    let out = render(&app);
    assert!(out.contains("old_name.rs → new_name.rs"), "header shows the rename: {out:?}");
}

#[test]
fn tabs_expand_to_spaces_in_the_diff() {
    let r = Repo::init();
    r.write("t.rs", "x\n");
    r.commit_all("init");
    r.write("t.rs", "x\n\tindented\n"); // a tab-indented added line
    let app = app_on(&r);
    let out = render(&app);
    let line = out.lines().find(|l| l.contains("indented")).expect("the added line renders");
    // The literal tab is gone; the word is preceded by spaces (4-col tab stop).
    assert!(!line.contains('\t'), "no literal tab in the rendered line");
    assert!(line.contains("    indented") || line.contains("   indented"), "tab became spaces");
}

#[test]
fn a_long_line_wraps_across_display_rows() {
    let long: String = std::iter::repeat_n("abcd", 60).collect(); // 240 cols, wider than the pane
    let r = Repo::init();
    r.write("w.rs", "x\n");
    r.commit_all("init");
    r.write("w.rs", &format!("x\n{long}\n"));
    let app = app_on(&r); // wrap defaults on

    // The whole long line is visible (no truncation): every chunk renders.
    let shown: String = render(&app).chars().filter(|c| *c == 'a').collect();
    assert!(shown.len() >= 60, "all of the wrapped line is shown, not truncated");
    // The logical row reports a display height > 1 (it wraps).
    let heights = ui::diff_row_heights(&app, AREA);
    let wrapped = app.visible.iter().position(|r| r.text().starts_with("abcd")).unwrap();
    assert!(heights[wrapped] > 1, "the long line spans multiple display rows");
}

#[test]
fn wrapping_breaks_at_word_boundaries() {
    // Words sized so the line must wrap, but no word is wider than the pane: every break
    // should land on a space, so no word is split across two display rows.
    let words = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima \
                 mike november oscar papa quebec romeo sierra tango";
    let r = Repo::init();
    r.write("w.rs", "x\n");
    r.commit_all("init");
    r.write("w.rs", &format!("x\n{words}\n"));
    let app = app_on(&r); // wrap defaults on

    let heights = ui::diff_row_heights(&app, AREA);
    let wrapped = app.visible.iter().position(|r| r.text().starts_with("alpha")).unwrap();
    assert!(heights[wrapped] > 1, "the line wraps across rows");

    // Every word survives intact on some rendered line (none straddles a wrap break).
    let out = render(&app);
    for word in words.split(' ') {
        assert!(out.lines().any(|l| l.contains(word)), "word {word:?} is not split across rows");
    }
}

#[test]
fn wide_glyphs_wrap_by_column_width_not_char_count() {
    // 50 wide CJK glyphs span 100 columns; 50 ASCII chars span 50. Width-aware wrapping
    // must give the CJK line more display rows — a char-counting wrap would tie them.
    let cjk: String = std::iter::repeat_n('あ', 50).collect();
    let ascii: String = std::iter::repeat_n('a', 50).collect();
    let r = Repo::init();
    r.write("w.rs", "x\n");
    r.commit_all("init");
    r.write("w.rs", &format!("x\n{ascii}\n{cjk}\n"));
    let app = app_on(&r); // wrap defaults on

    let heights = ui::diff_row_heights(&app, AREA);
    let ascii_h = heights[app.visible.iter().position(|r| r.text().starts_with('a')).unwrap()];
    let cjk_h = heights[app.visible.iter().position(|r| r.text().starts_with('あ')).unwrap()];
    assert!(cjk_h > ascii_h, "wide glyphs wrap by columns: cjk {cjk_h} > ascii {ascii_h}");
}

#[test]
fn horizontal_scroll_shifts_the_diff_left() {
    let r = Repo::init();
    r.write("w.rs", "x\n");
    r.commit_all("init");
    r.write("w.rs", "x\nAAAABBBBCCCCDDDD_marker\n");
    let mut app = App::new(r.path_buf(), Scope::Uncommitted, None);
    app.wrap = false; // horizontal scroll applies only with wrap off
    app.reload().unwrap();
    assert!(render(&app).contains("AAAABBBB"), "the line head shows before scrolling");

    app.scroll_h(8); // drop the first 8 code columns
    let out = render(&app);
    assert!(!out.contains("AAAABBBB"), "the scrolled-off head is gone");
    assert!(out.contains("CCCCDDDD_marker"), "the later columns are now visible");
}

#[test]
fn a_changed_word_gets_the_emphasis_background() {
    const EMPH_INS_BG: ratatui::style::Color = ratatui::style::Color::Rgb(0x30, 0x55, 0x3f);
    let r = Repo::init();
    r.write("e.rs", "let x = foo(a);\n");
    r.commit_all("init");
    r.write("e.rs", "let x = bar(a, b);\n");
    let mut app = app_on(&r);
    app.focus = Focus::Files; // no diff cursor, so the emphasis bg shows
    let buf = render_buffer(&app);

    // Somewhere in the diff pane a cell carries the brighter insertion-emphasis bg,
    // and it sits under a changed character (a `b` from `bar`), not the shared prefix.
    let mut found = false;
    for y in 0..40 {
        for x in 0..95 {
            if let Some(c) = buf.cell((x, y))
                && c.bg == EMPH_INS_BG
                && c.symbol() == "b"
            {
                found = true;
            }
        }
    }
    assert!(found, "a changed word carries the emphasis background");
}

/// Catppuccin surface1 — the cursor fill of the pane that does not hold focus.
const UNFOCUSED_CURSOR_BG: ratatui::style::Color = ratatui::style::Color::Rgb(0x45, 0x47, 0x5a);

#[test]
fn the_diff_cursor_row_is_marked_from_either_pane() {
    // The diff pane's cursor row fills like the file list's: brightest when the pane holds
    // focus, a step softer when it does not. A hunk step driven from the file list moves this
    // cursor, so it has to be visible from there.
    let mut app = edited_app();
    app.focus = Focus::Diff;
    app.next_hunk();
    let cursor_y = |app: &App| 2 + app.diff_cursor as u16; // border at y=1, first row at y=2
    let fill = |app: &App, bg| {
        let buf = render_buffer(app);
        let y = cursor_y(app);
        (1..40u16).filter(|&x| buf.cell((x, y)).is_some_and(|c| c.bg == bg)).count()
    };

    assert!(fill(&app, SELECTION_BG) > 10, "the focused diff fills its cursor row with surface2");

    app.focus = Focus::Files;
    assert!(
        fill(&app, UNFOCUSED_CURSOR_BG) > 10,
        "and still marks it, a step softer, while the file list holds focus"
    );
}

#[test]
fn the_selected_file_row_fills_with_the_shared_selection_color() {
    let app = edited_app(); // one file, file_cursor = 0, Files focused
    let buf = render_buffer(&app);
    // Files pane: right 32% of 140 cols; its border is at y=1, first content row at y=2.
    let files_x0 = 140 - 140 * 32 / 100 + 1;
    let selected =
        (files_x0..139).filter(|&x| buf.cell((x, 2)).is_some_and(|c| c.bg == SELECTION_BG)).count();
    assert!(selected > 10, "the selected file row fills wide with surface2: {selected} cells");
}

#[test]
fn a_hidden_navigator_gives_the_read_pane_the_whole_body() {
    let mut app = edited_app();
    app.focus = Focus::Diff;
    app.next_hunk();
    let cursor_y = 2 + app.diff_cursor as u16;
    let fill = |app: &App| {
        let buf = render_buffer(app);
        (1..139u16)
            .filter(|&x| buf.cell((x, cursor_y)).is_some_and(|c| c.bg == SELECTION_BG))
            .count()
    };
    let visible_fill = fill(&app);
    let out = render(&app);
    assert!(!out.contains("z hide"), "visible and collapsed, the hide key waits under `?`");

    app.toggle_navigator_hidden();
    let hidden_fill = fill(&app);
    assert!(
        hidden_fill > visible_fill && hidden_fill > 120,
        "the cursor row fills the whole body with surface2: {hidden_fill} vs {visible_fill}"
    );
    let out = render(&app);
    assert!(out.contains("z show"), "the collapsed footer names the way back");

    app.toggle_keys();
    let out = render(&app);
    assert!(out.contains("z show"), "row 1 keeps the way back in the expansion");
    assert!(!out.contains("p layout"), "`p layout` drops while hidden");

    app.toggle_navigator_hidden();
    let out = render(&app);
    assert!(out.contains("z hide"), "visible, the `go` band lists the hide key");
    assert!(out.contains("p layout"), "`p layout` returns with the navigator");
}

#[test]
fn shows_tab_bar_file_list_and_diff() {
    let app = edited_app();
    let out = render(&app);
    assert!(out.contains("Changes"), "tab bar names the view");
    assert!(out.contains("uncommitted"), "current scope shown");
    assert!(out.contains("hello.rs"), "file appears in the list");
    assert!(out.contains("BETA"), "diff content is rendered");
    assert!(out.contains("changed"), "the header shows the changed count");
}

#[test]
fn the_header_totals_the_scope_and_hides_them_at_zero() {
    let r = Repo::init();
    r.write("edited.rs", "old\n");
    r.commit_all("init");
    r.write("edited.rs", "new\n");
    r.write("untracked.rs", "one\ntwo\n");
    let app = app_on(&r);

    // 64 columns is the exact fit (the tab strip ends in the two-column reserved
    // indicator cell). The totals' `−` is multi-byte, so this breaks if the header
    // measures bytes instead of display width.
    let header = render_at(&app, 64).lines().next().unwrap().to_string();
    assert!(header.contains("2 changed  +3 −1"), "count, then the totals:\n{header}");

    let clean = Repo::init();
    clean.write("clean.rs", "same\n");
    clean.commit_all("init");
    let app = app_on(&clean);
    let header = render_at(&app, 80).lines().next().unwrap().to_string();
    assert!(header.contains("0 changed"), "the bare count remains:\n{header}");
    assert!(!header.contains('+'), "an empty changeset shows no totals:\n{header}");
}

/// The last non-blank rendered row — the footer band.
fn footer_line(out: &str) -> String {
    out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().to_string()
}

/// Focus the diff on its first changed line.
fn on_changed_line(app: &mut App) {
    app.focus = Focus::Diff;
    app.diff_cursor = app.visible.iter().position(|r| r.marker() == '+').unwrap();
}

#[test]
fn the_footer_offers_the_armed_crossing_in_both_directions() {
    // Two files, one hunk each, so a hunk step from either end has only a crossing left to offer.
    let r = Repo::init();
    r.write("a.rs", "one\ntwo\n");
    r.write("z.rs", "one\ntwo\n");
    r.commit_all("init");
    r.write("a.rs", "one\nEDIT A\n");
    r.write("z.rs", "one\nEDIT Z\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;

    app.next_hunk(); // onto a.rs's only hunk
    app.next_hunk(); // nothing below it: arms the crossing forward
    let footer = footer_line(&render(&app));
    assert!(footer.contains("] next file"), "the armed crossing leads the bar:\n{footer}");
    assert!(footer.contains("c comment"), "and the line's own action stays:\n{footer}");

    app.next_hunk(); // takes it
    app.prev_hunk(); // nothing above z.rs's hunk: arms the crossing back
    let footer = footer_line(&render(&app));
    assert!(footer.contains("[ prev file"), "armed backward, the bar names `[`:\n{footer}");
}

#[test]
fn the_footer_shows_the_action_for_the_context() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    let footer = footer_line(&render(&app));
    assert!(footer.contains("c comment"), "a diff line offers comment:\n{footer}");
    assert!(footer.contains("v select"), "and selecting a range:\n{footer}");
    assert!(!footer.contains("changed"), "the changed count is not in the footer:\n{footer}");
}

/// Whether a footer row closes with the `?` hint, labeled or bare.
fn ends_with_hint(row: &str) -> bool {
    let row = row.trim_end();
    row.ends_with("? shortcuts") || row.ends_with('?')
}

#[test]
fn the_footer_trims_trailing_actions_to_fit_keeping_the_primary_and_the_more_hint() {
    let mut app = edited_app();
    on_changed_line(&mut app); // diff focus, content line → c comment · v select … ?
    // Wide: every cursor action fits, and the `?` closes the row.
    let wide = footer_line(&render_at(&app, 120));
    assert!(
        wide.contains("c comment") && wide.contains("v select") && ends_with_hint(&wide),
        "wide footer shows all actions and the `?`:\n{wide}"
    );
    assert!(wide.trim_end().ends_with("? shortcuts"), "with room, the `?` is labeled:\n{wide}");
    // Narrow: the primary survives, the trailing action drops, and the `?` stays at the right.
    let narrow = footer_line(&render_at(&app, 18));
    assert!(narrow.contains("c comment"), "the primary action is never dropped:\n{narrow}");
    assert!(ends_with_hint(&narrow), "the `?` never drops:\n{narrow}");
    assert!(!narrow.contains("v select"), "the trailing action is trimmed off row 1:\n{narrow}");
    // Too narrow for the primary and the `?` together: the primary sheds its label to its key, and
    // the `?` still survives at the right.
    let tiny = footer_line(&render_at(&app, 11));
    assert!(tiny.contains(" c "), "the primary keeps its key:\n{tiny}");
    assert!(!tiny.contains("comment"), "the primary sheds its label:\n{tiny}");
    assert!(ends_with_hint(&tiny), "the `?` still survives:\n{tiny}");
}

#[test]
fn a_narrow_row_keeps_send_and_the_more_hint_by_shedding_the_primary_label() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.start_comment();
    for ch in "n".chars() {
        app.input_push(ch);
    }
    app.submit_comment(); // a written comment adds `s send 1` to row 1
    // A pane too narrow for the full primary alongside `send` and `?` keeps all three by shedding
    // the primary's label — `send` and the `?` must never clip off the right edge.
    let narrow = footer_line(&render_at(&app, 16));
    assert!(narrow.contains("y copy"), "copy never drops:\n{narrow}");
    assert!(ends_with_hint(&narrow), "the `?` never drops:\n{narrow}");
    assert!(narrow.chars().count() <= 16, "the row never overflows its width:\n{narrow}");
}

#[test]
fn the_footer_shows_the_sends_outcome_at_a_pane_width_by_yielding_the_cursor_actions() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.start_comment();
    app.input_push('n');
    app.submit_comment(); // a written comment adds `s send 1` to row 1

    // The status is the only answer `s` gives, and a pane is around 40 columns wide, so
    // the cursor's actions yield to it: the `?` panel repeats every action and nothing repeats the
    // status.
    app.status = "no agent here — copy to the clipboard instead".to_string();
    let narrow = footer_line(&render_at(&app, 40));
    assert!(narrow.contains("no agent here"), "the refusal shows at 40 columns:\n{narrow}");
    assert!(narrow.contains("y copy"), "copy never drops:\n{narrow}");
    assert!(ends_with_hint(&narrow), "the `?` never drops:\n{narrow}");
    assert!(!narrow.contains("d delete"), "the cursor's actions yield to the status:\n{narrow}");

    // With room for both, nothing yields.
    let wide = footer_line(&render_at(&app, 120));
    assert!(
        wide.contains("no agent here — copy to the clipboard instead"),
        "a wide row shows the whole refusal:\n{wide}"
    );
    assert!(wide.contains("d delete"), "and keeps the cursor's actions:\n{wide}");

    // Below a legible width the status drops rather than paint a lone `·` promising a message.
    let tiny = footer_line(&render_at(&app, 20));
    assert!(!tiny.contains("agent"), "no room for a legible message, so none is painted:\n{tiny}");
    assert!(tiny.contains("y copy"), "send still never drops:\n{tiny}");

    // A truncated status never pushes the `?` off the right edge, at any width that fits row 1's
    // own fixed parts. Below 14 columns the shed primary and `send` overflow it on their own, with
    // no status in play at all.
    for w in 14..=140u16 {
        let row = footer_line(&render_at(&app, w));
        assert!(ends_with_hint(&row), "the `?` left the row at width {w}:\n{row}");
    }

    // `s` is also the comments list's primary, so a refusal has to reach the reviewer there too.
    // The list has no `?`, so its trailing `…` is the only promise the trimmed actions exist, and
    // the status leaves room for it.
    app.open_list();
    let listed = footer_line(&render_at(&app, 40));
    assert!(listed.contains("no agent here"), "the refusal shows in the list at 40:\n{listed}");
    assert!(listed.contains("y copy"), "copy never drops in the list either:\n{listed}");
    assert!(listed.trim_end().ends_with('…'), "the trimmed actions keep their `…`:\n{listed}");
}

#[test]
fn the_expansion_aligns_row_one_into_the_labeled_grid() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.toggle_keys();
    let out = render(&app);
    // Search only the footer rows, so a stray `move`/`go` in the diff or file list can't stand in.
    let footer_start = ui::body_rect(Rect::new(0, 0, 140, 40), &app);
    let footer_start = (footer_start.y + footer_start.height) as usize;
    let line_of = |lbl: &str| {
        out.lines()
            .skip(footer_start)
            .find(|l| l.trim_start().starts_with(lbl))
            .unwrap_or("")
            .to_string()
    };
    let (do_line, go_line, move_line) = (line_of("do"), line_of("go"), line_of("move"));

    // Row 1 is now the `do` band: the primary, and the `?` still at the right.
    assert!(
        do_line.contains("c comment") && ends_with_hint(&do_line),
        "row 1 is the `do` line with the primary and `?`:\n{do_line}"
    );
    assert!(go_line.contains("scope"), "the go band lists the always-there keys:\n{go_line}");
    assert!(
        move_line.contains("hunk") && move_line.contains("file"),
        "the move band names the hunk and file steps:\n{move_line}"
    );
    // The three labels share one gutter column, and their content aligns in the next.
    let at = |l: &str, s: &str| l.find(s).expect("token present");
    assert_eq!(at(&do_line, "do"), at(&go_line, "go"), "labels share a gutter column");
    assert_eq!(at(&go_line, "go"), at(&move_line, "move"), "labels share a gutter column");
    assert_eq!(
        at(&do_line, "c comment"),
        at(&go_line, "u/b/g"),
        "the primary aligns under the same column as the band keys"
    );
    assert_eq!(at(&go_line, "u/b/g"), at(&move_line, "j k"), "band keys align in one column");
}

#[test]
fn the_collapsed_footer_stays_a_flush_action_bar() {
    let mut app = edited_app();
    on_changed_line(&mut app); // collapsed: no expansion
    let footer = footer_line(&render(&app));
    assert!(
        footer.trim_start().starts_with("c comment"),
        "no `do` gutter when collapsed:\n{footer}"
    );
    assert!(!footer.contains(" do "), "the `do` label appears only when expanded:\n{footer}");
}

#[test]
fn the_expanded_row_one_never_drops_send_or_the_more_hint_on_a_narrow_pane() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.start_comment();
    for ch in "n".chars() {
        app.input_push(ch);
    }
    app.submit_comment(); // a written comment puts `s send 1` on row 1
    app.toggle_keys(); // expanded — the fixed `do` gutter cannot shed
    for w in [14u16, 16, 18, 20, 22, 30] {
        let out = dump(&render_size(&app, w, 40));
        let row1 = out.lines().find(|l| l.contains("y copy")).expect("row 1 carries copy");
        let row1 = row1.trim_end();
        assert!(row1.contains("y copy"), "copy survives at w={w}: [{row1}]");
        assert!(ends_with_hint(row1), "the `?` survives at w={w}: [{row1}]");
        assert!(row1.chars().count() <= w as usize, "row 1 never overflows at w={w}: [{row1}]");
    }
}

#[test]
fn the_expansion_caps_so_the_body_keeps_its_rows() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.toggle_keys();
    // On a short pane the wrapped bands would want more rows than fit, but the footer is capped so
    // the body keeps its Min(3).
    let body = ui::body_rect(Rect::new(0, 0, 40, 6), &app);
    assert!(body.height >= 3, "the body keeps at least three rows: got {}", body.height);
}

#[test]
fn the_footer_keeps_its_actions_alongside_a_status() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.status = "comment added".to_string();
    let footer = footer_line(&render(&app));
    // A status sits among the actions, never replacing them.
    assert!(footer.contains("comment added"), "the status shows:\n{footer}");
    assert!(
        footer.contains("c comment"),
        "the primary action persists alongside a status:\n{footer}"
    );
}

/// The editor's failure has to reach the reviewer on the frame it happened, from either pane.
#[test]
fn the_footer_shows_an_editor_failure_from_either_pane() {
    let mut app = edited_app();
    on_changed_line(&mut app);
    app.status = "editor failed: No such file or directory (os error 2)".to_string();
    let footer = footer_line(&render(&app));
    assert!(footer.contains("editor failed"), "on the read pane:\n{footer}");

    app.focus = diff_reckoner::app::Focus::Files;
    let footer = footer_line(&render(&app));
    assert!(footer.contains("editor failed"), "and on the navigator:\n{footer}");

    // At the pane width the reviewer actually runs, not only the test default.
    let footer = footer_line(&render_at(&app, 120));
    assert!(footer.contains("editor failed"), "at 120 columns:\n{footer}");
}

#[test]
fn empty_repo_shows_empty_states() {
    let r = Repo::init();
    r.write("seed.rs", "x\n");
    r.commit_all("init");
    let app = app_on(&r);

    let out = render(&app);
    assert!(out.contains("no changes"), "empty file list state");
}

#[test]
fn composing_renders_the_inline_multiline_box() {
    let mut app = edited_app();
    app.focus = Focus::Diff;
    app.diff_cursor = app.diff.rows.iter().position(|r| r.marker() == '+').unwrap();
    app.start_comment();
    for ch in "line one".chars() {
        app.input_push(ch);
    }
    app.input_push('\n');
    for ch in "line two".chars() {
        app.input_push(ch);
    }

    let out = render(&app);
    assert!(out.contains("comment ·"), "box titled with the location");
    assert!(out.contains("line one"), "first input line shown");
    assert!(out.contains("line two"), "second input line shown — the box is multi-line");
}

#[test]
fn the_box_grows_with_multiline_input_and_keeps_the_anchor_visible() {
    let r = Repo::init();
    r.write("mid.rs", "a\nb\nc\nd\ne\n");
    r.commit_all("init");
    r.write("mid.rs", "a\nB\nc\nd\ne\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;
    app.diff_cursor =
        app.diff.rows.iter().position(|r| r.marker() == '+' && r.text().contains('B')).unwrap();
    app.start_comment();
    for ch in "one\ntwo\nthree".chars() {
        app.input_push(ch);
    }

    let out = render(&app);
    assert!(out.contains("one") && out.contains("two") && out.contains("three"), "all box lines");
    let lines: Vec<&str> = out.lines().collect();
    // The inserted line is the only one carrying an uppercase `B` (no `+` glyph now).
    let anchor = lines.iter().position(|l| l.contains('B')).expect("anchor line visible");
    let box_row = lines.iter().position(|l| l.contains("comment ·")).expect("box");
    assert!(anchor < box_row, "the commented line stays above the box as it grows");
}

#[test]
fn the_box_is_inserted_under_the_selected_line() {
    let r = Repo::init();
    r.write("mid.rs", "alpha\nbeta\ngamma\n");
    r.commit_all("init");
    r.write("mid.rs", "alpha\nBETA\ngamma\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;
    app.diff_cursor = app.diff.rows.iter().position(|r| r.text().contains("BETA")).unwrap();
    app.start_comment();
    for ch in "note".chars() {
        app.input_push(ch);
    }

    let out = render(&app);
    let lines: Vec<&str> = out.lines().collect();
    let box_row = lines.iter().position(|l| l.contains("comment ·")).expect("box rendered");
    let below_row = lines.iter().position(|l| l.contains("gamma")).expect("context below shown");
    assert!(below_row > box_row, "the diff line below the selection is pushed under the box");
}

const AREA: Rect = Rect { x: 0, y: 0, width: 140, height: 40 };

#[test]
fn header_clicks_map_to_the_scope_chip() {
    let app = edited_app(); // scope uncommitted, no comments
    // Scan the header row instead of hardcoding columns, so the test survives changes
    // to the label text.
    let scope: Vec<u16> = (0..AREA.width)
        .filter(|&c| ui::hit_header(AREA, &app, app.keymap(), c, 0) == Some(HeaderHit::Scope))
        .collect();

    assert!(!scope.is_empty(), "scope chip is clickable");

    let gap = scope.iter().max().unwrap() + 1;
    assert_eq!(
        ui::hit_header(AREA, &app, app.keymap(), gap, 0),
        None,
        "the space right of the chip is inert"
    );
    assert_eq!(
        ui::hit_header(AREA, &app, app.keymap(), scope[0], 5),
        None,
        "only row 0 is the header"
    );
}

#[test]
fn file_and_diff_clicks_map_to_row_indices() {
    let app = edited_app();
    // Right pane: the first file row maps to index 0; clicking past the list misses.
    assert_eq!(ui::hit_file(AREA, &app, 120, 2, app.file_rows.len(), 0), Some(0));
    assert_eq!(ui::hit_file(AREA, &app, 120, 9, app.file_rows.len(), 0), None);
    // With the list scrolled down, the top visible row maps to that scrolled-to index.
    assert_eq!(ui::hit_file(AREA, &app, 120, 2, 50, 7), Some(7));
    assert_eq!(ui::hit_file(AREA, &app, 120, 3, 50, 7), Some(8));
    // The wheel routes by pointer: a column in the navigator is "in" the file list,
    // one in the read pane is not.
    assert!(ui::in_files_pane(AREA, &app, 120, 3));
    assert!(!ui::in_files_pane(AREA, &app, 10, 3));
    // Left pane: diff rows map top-down to diff-line indices.
    assert!(app.visible.len() > 1);
    let heights = ui::diff_row_heights(&app, AREA);
    assert_eq!(ui::hit_diff(AREA, &app, 10, 2, &heights, 0), Some(0));
    assert_eq!(ui::hit_diff(AREA, &app, 10, 3, &heights, 0), Some(1));
    // With a nonzero scroll and wrapped (multi-row) lines, the click must skip the
    // scrolled-off rows and account for each visible row's display height. Rows are
    // 2 tall each; diff_scroll=1 puts row index 1 at the top of the pane (inner.y == 2).
    let tall = [2usize, 2, 2, 2];
    assert_eq!(ui::hit_diff(AREA, &app, 10, 2, &tall, 1), Some(1)); // top visible row
    assert_eq!(ui::hit_diff(AREA, &app, 10, 3, &tall, 1), Some(1)); // its second display row
    assert_eq!(ui::hit_diff(AREA, &app, 10, 4, &tall, 1), Some(2)); // next logical row
}

#[test]
fn navigator_layout_rects_cover_every_position_and_tiny_axis() {
    let mut app = edited_app();
    let body = ui::body_rect(AREA, &app);

    for position in [
        NavigatorPosition::Right,
        NavigatorPosition::Bottom,
        NavigatorPosition::Left,
        NavigatorPosition::Top,
    ] {
        app.navigator_position = position;
        let _ = render_size(&app, AREA.width, AREA.height);
        let app_ref = &app;
        let files: Vec<(u16, u16)> = (body.y..body.y + body.height)
            .flat_map(|row| {
                (body.x..body.x + body.width)
                    .filter(move |&col| ui::in_files_pane(AREA, app_ref, col, row))
                    .map(move |col| (col, row))
            })
            .collect();
        let diff: Vec<(u16, u16)> = (body.y..body.y + body.height)
            .flat_map(|row| {
                (body.x..body.x + body.width)
                    .filter(move |&col| ui::in_diff_pane(AREA, app_ref, col, row))
                    .map(move |col| (col, row))
            })
            .collect();
        let files_x = (
            files.iter().map(|&(x, _)| x).min().unwrap(),
            files.iter().map(|&(x, _)| x).max().unwrap(),
        );
        let files_y = (
            files.iter().map(|&(_, y)| y).min().unwrap(),
            files.iter().map(|&(_, y)| y).max().unwrap(),
        );
        let diff_x = (
            diff.iter().map(|&(x, _)| x).min().unwrap(),
            diff.iter().map(|&(x, _)| x).max().unwrap(),
        );
        let diff_y = (
            diff.iter().map(|&(_, y)| y).min().unwrap(),
            diff.iter().map(|&(_, y)| y).max().unwrap(),
        );
        assert_eq!(files.len() + diff.len(), usize::from(body.width * body.height));
        assert!(!files.is_empty() && !diff.is_empty());
        assert!(
            (body.y..body.y + body.height).any(|row| {
                (body.x..body.x + body.width).any(|col| ui::hit_divider(AREA, &app, col, row))
            }),
            "divider is hittable for {position:?}"
        );
        match position {
            NavigatorPosition::Right => {
                assert!(files.iter().map(|(x, _)| x).min() > diff.iter().map(|(x, _)| x).min());
                assert_eq!(files.len() / usize::from(body.height), 44);
                assert!(!ui::hit_divider(AREA, &app, files_x.0 + 1, body.y + 4));
                assert!(!ui::hit_divider(AREA, &app, diff_x.1 - 1, body.y + 4));
            }
            NavigatorPosition::Left => {
                assert!(files.iter().map(|(x, _)| x).min() < diff.iter().map(|(x, _)| x).min());
                assert_eq!(files.len() / usize::from(body.height), 44);
                assert!(!ui::hit_divider(AREA, &app, files_x.1 - 1, body.y + 4));
                assert!(!ui::hit_divider(AREA, &app, diff_x.0 + 1, body.y + 4));
            }
            NavigatorPosition::Bottom => {
                assert!(files.iter().map(|(_, y)| y).min() > diff.iter().map(|(_, y)| y).min());
                assert_eq!(files.len() / usize::from(body.width), 9);
                assert!(!ui::hit_divider(AREA, &app, body.x + 4, files_y.0 + 1));
                assert!(!ui::hit_divider(AREA, &app, body.x + 4, diff_y.1 - 1));
            }
            NavigatorPosition::Top => {
                assert!(files.iter().map(|(_, y)| y).min() < diff.iter().map(|(_, y)| y).min());
                assert_eq!(files.len() / usize::from(body.width), 9);
                assert!(!ui::hit_divider(AREA, &app, body.x + 4, files_y.1 - 1));
                assert!(!ui::hit_divider(AREA, &app, body.x + 4, diff_y.0 + 1));
            }
        }
    }

    app.navigator_position = NavigatorPosition::Right;
    let six = Rect::new(0, 0, 6, 10);
    let row = ui::body_rect(six, &app).y;
    assert_eq!((0..6).filter(|&col| ui::in_files_pane(six, &app, col, row)).count(), 3);
    assert_eq!((0..6).filter(|&col| ui::in_diff_pane(six, &app, col, row)).count(), 3);

    let five = Rect::new(0, 0, 5, 10);
    let row = ui::body_rect(five, &app).y;
    assert_eq!((0..5).filter(|&col| ui::in_files_pane(five, &app, col, row)).count(), 2);
    assert_eq!((0..5).filter(|&col| ui::in_diff_pane(five, &app, col, row)).count(), 3);

    app.navigator_position = NavigatorPosition::Top;
    let eight = Rect::new(0, 0, 10, 10); // body height 8
    let col = ui::body_rect(eight, &app).x;
    assert_eq!((1..9).filter(|&row| ui::in_files_pane(eight, &app, col, row)).count(), 3);
    assert_eq!((1..9).filter(|&row| ui::in_diff_pane(eight, &app, col, row)).count(), 5);

    let seven = Rect::new(0, 0, 10, 7); // body height 5: navigator gets floor(5 / 2)
    let col = ui::body_rect(seven, &app).x;
    assert_eq!((1..6).filter(|&row| ui::in_files_pane(seven, &app, col, row)).count(), 2);
    assert_eq!((1..6).filter(|&row| ui::in_diff_pane(seven, &app, col, row)).count(), 3);
}

#[test]
fn a_binary_file_shows_the_no_line_comments_message() {
    let r = Repo::init();
    r.write("logo.bin", "\0\0\0\0seed\0\0");
    r.commit_all("init");
    r.write("logo.bin", "\0\0\0\0changed\0\0\0");
    let mut app = app_on(&r);
    let idx = app.entries.iter().position(|f| f.path == "logo.bin").expect("binary file listed");
    app.select_file(idx).unwrap();

    let out = render(&app);
    assert!(out.contains("binary — no line comments"), "binary diff message shown:\n{out}");
}

#[test]
fn the_comments_list_flags_a_stale_comment() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nBETA\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;
    app.diff_cursor = app.diff.rows.iter().position(|r| r.marker() == '+').unwrap();
    app.start_comment();
    for ch in "look here".chars() {
        app.input_push(ch);
    }
    app.submit_comment();

    // a.rs reverts to its committed state → leaves the changeset → the comment is stale.
    r.write("a.rs", "alpha\nbeta\n");
    app.reload().unwrap();
    app.open_list();

    let out = render(&app);
    assert!(out.contains("(stale)"), "stale comment flagged in the list:\n{out}");
}

#[test]
fn open_list_renders_the_comments_overlay() {
    let mut app = edited_app();
    app.focus = Focus::Diff;
    app.diff_cursor = app.diff.rows.iter().position(|r| r.marker() == '+').unwrap();
    app.start_comment();
    for ch in "overlay note".chars() {
        app.input_push(ch);
    }
    app.submit_comment();
    app.open_list();

    let out = render(&app);
    assert!(out.contains("Comments ("), "overlay titled with a count");
    assert!(out.contains("overlay note"), "comment text listed");
}

#[test]
fn all_files_tab_bar_footer_and_count_read_for_the_tab() {
    use diff_reckoner::app::Tab;
    let r = Repo::init();
    r.write("a.rs", "one\n");
    r.commit_all("init");
    r.write("a.rs", "ONE\n"); // one change
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);

    let out = render(&app);
    assert!(out.contains("1 Changes"), "tab labels carry their switch digit:\n{out}");
    assert!(out.contains("2 Files"));
    assert!(
        out.contains("1 changed"),
        "the changed count stays in the header on All files:\n{out}"
    );
    let footer = footer_line(&out);
    assert!(ends_with_hint(&footer), "the collapsed footer closes with the `?`:\n{footer}");
    assert!(
        !footer.contains("changed"),
        "the changed count is not repeated in the footer:\n{footer}"
    );
    // `scope` is a `go` key now, revealed by the `?` expansion rather than crowding row 1.
    app.toggle_keys();
    let expanded = render(&app);
    assert!(expanded.contains("scope"), "the `?` expansion lists the scope keys:\n{expanded}");
    assert!(expanded.contains("move"), "and labels the movement band:\n{expanded}");
}

#[test]
fn all_files_empty_pane_reads_select_a_file() {
    use diff_reckoner::app::Tab;
    let r = Repo::init();
    r.write("src/a.rs", "x\n");
    r.write("src/b.rs", "y\n"); // two children so src/ is a real collapsed dir, not a folded file
    r.commit_all("init");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles); // clean repo: no seed; cursor rests on collapsed src/

    let out = render(&app);
    assert!(out.contains("select a file to read"), "the empty All files read-pane copy:\n{out}");
    assert!(!out.contains("no diff"), "no diff vocabulary in the content browser:\n{out}");
}

#[test]
fn renders_a_light_theme_without_panic() {
    let mut app = edited_app();
    app.set_cli_theme(Some("catppuccin-latte".to_string()));
    // Driving the full render path with a derived light palette must not panic, and a Latte
    // color (the focused pane's blue border) reaches the painted buffer.
    let buf = render_buffer(&app);
    let latte_blue = diff_reckoner::theme::resolve(Some("catppuccin-latte")).palette.blue;
    let painted = (0..40)
        .flat_map(|y| (0..140).map(move |x| (x, y)))
        .any(|(x, y)| buf.cell((x, y)).is_some_and(|c| c.fg == latte_blue));
    assert!(painted, "the Latte palette reaches the painted buffer");
}

#[test]
fn every_cell_paints_on_the_theme_not_the_terminal_default() {
    use ratatui::style::Color;
    let mut app = edited_app();
    app.set_cli_theme(Some("solarized-light".to_string()));
    // The page alone, and under a popup whose `Clear` resets its cells.
    for open in [false, true] {
        if open {
            app.open_theme_picker();
        }
        let buf = render_buffer(&app);
        let base = app.palette().base;
        assert!(buf.content.iter().all(|c| c.bg != Color::Reset && c.fg != Color::Reset));
        assert!(buf.content.iter().filter(|c| c.bg == base).count() > 1000, "mostly base");
    }
}

#[test]
fn the_theme_picker_lists_both_sides_and_checks_each_saved_theme() {
    let mut app = edited_app();
    let closed = render_buffer(&app);
    app.open_theme_picker();
    let out = render(&app);
    let header = out.lines().find(|l| l.contains(" dark") && l.contains(" light"));
    assert!(header.is_some(), "the dark and light lists sit side by side:\n{out}");
    let row = out.lines().find(|l| l.contains("catppuccin ")).expect("the first row");
    assert!(row.contains("✓ catppuccin ") && row.contains("✓ catppuccin-latte"), "{row}");
    assert!(out.contains("iceberg-light") && out.contains("tomorrow-night"), "every theme lists");
    // No scrim: the page beside the popup is the preview, painted exactly as with it closed.
    let buf = render_buffer(&app);
    assert_eq!(buf.cell((0, 1)), closed.cell((0, 1)));
    // The active side's highlight takes the selection fill.
    let (x, y) = out
        .lines()
        .enumerate()
        .find_map(|(y, l)| l.find("✓ catppuccin ").map(|b| (l[..b].chars().count(), y)))
        .unwrap();
    assert_eq!(buf.cell((x as u16 + 2, y as u16)).unwrap().bg, SELECTION_BG);
}

/// An `edited_app` running under `[keybindings]` from a real config file.
fn rebound_app(keybindings: &str) -> App {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), format!("[keybindings]\n{keybindings}"))
        .unwrap();
    let config = diff_reckoner::config::plugin_config_in(dir.path()).unwrap();
    let mut app = edited_app();
    app.set_plugin_config(config);
    app.focus = Focus::Diff;
    app
}

#[test]
fn hints_show_the_first_bound_key() {
    let app = rebound_app("comment = [\"ㅊ\", \"c\"]\ntab-all-files = [\"x\"]\n");
    let out = render(&app);
    let footer = footer_line(&out);
    // A wide hint key spans two buffer cells, so the dump carries a placeholder space after it.
    assert!(footer.contains("ㅊ  comment"), "the hint is the first bound key:\n{footer}");
    assert!(out.contains("x Files"), "the header tab hint follows its binding:\n{out}");
    assert!(!out.contains("2 Files"), "the replaced digit is gone:\n{out}");
}

#[test]
fn the_markdown_preview_renders_styled_lines_without_a_gutter() {
    let r = Repo::init();
    r.write("README.md", "# Install\n\nRun `cargo test` for **all** checks.\n");
    r.commit_all("init");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);

    // Source view: raw markdown, and the footer surfaces the way into the preview.
    app.focus = Focus::Diff;
    let source = render(&app);
    assert!(source.contains("# Install"), "source shows raw markdown:\n{source}");
    let footer = source.lines().last().unwrap();
    assert!(footer.contains("m preview"), "source discovers the preview:\n{footer}");

    app.toggle_preview();
    let out = render(&app);
    assert!(out.contains("Install"), "the heading text renders:\n{out}");
    assert!(!out.contains("# Install"), "the # markers are gone in the preview:\n{out}");
    assert!(!out.contains("**all**"), "emphasis markers are consumed:\n{out}");
    assert!(!out.contains("  1 "), "the preview has no line-number gutter:\n{out}");
    let footer = out.lines().last().unwrap();
    assert!(footer.contains("m source"), "the footer leads back to source:\n{footer}");
    assert!(!footer.contains("c comment"), "no comment key in the preview:\n{footer}");
}

#[test]
fn a_deleted_markdown_file_offers_no_preview_in_the_footer() {
    let r = Repo::init();
    r.write("gone.md", "# Doc\n\nbody\n");
    r.commit_all("init");
    r.remove("gone.md");
    let mut app = app_on(&r);
    assert_eq!(app.diff_path.as_deref(), Some("gone.md"));
    app.focus = Focus::Diff;

    // The deletion rows are commentable, but a deleted file has no current content, so
    // the footer never offers the inert preview toggle.
    let out = render(&app);
    let footer = out.lines().last().unwrap();
    assert!(footer.contains("c comment"), "a deletion row is commentable:\n{footer}");
    assert!(!footer.contains("m preview"), "a deleted file offers no preview:\n{footer}");
}

#[test]
fn an_anchor_click_scrolls_the_preview_to_its_heading() {
    let mut md = String::from(
        "# Top

jump [go](#section-two)

",
    );
    for i in 0..40 {
        use std::fmt::Write as _;
        let _ = write!(md, "filler paragraph {i}\n\n");
    }
    md.push_str(
        "## Section Two

the target body
",
    );
    let r = Repo::init();
    r.write("doc.md", &md);
    r.commit_all("init");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);

    // In source view an anchor click is inert: no anchors are painted there.
    let _ = render(&app);
    app.open_link("#section-two");
    assert_eq!(app.preview_scroll, 0, "source view ignores anchor destinations");

    app.toggle_preview();
    let _ = render(&app); // paint: anchors and link regions note themselves

    assert_eq!(app.preview_scroll, 0);
    app.open_link("#section-two");
    assert!(app.preview_scroll > 40, "the preview jumped to the heading: {}", app.preview_scroll);
    let out = render(&app);
    assert!(
        out.contains("Section Two"),
        "the heading is on screen:
{out}"
    );
    assert!(
        !out.contains("# Top"),
        "the top scrolled away:
{out}"
    );
    assert!(out.contains('┃'), "an overflowing preview shows the scrollbar thumb:\n{out}");
}

#[test]
fn the_preview_paints_link_regions_and_names_itself_in_the_title() {
    let r = Repo::init();
    r.write("README.md", "# Install\n\nsee [docs](https://docs.example/x)\n");
    r.commit_all("init");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);

    let source = render(&app);
    assert!(!source.contains("· preview"), "source view has no preview marker");
    let miss = first_painted_link(&app);
    assert_eq!(miss, None, "raw source paints no link regions");

    app.toggle_preview();
    let out = render(&app);
    assert!(out.contains("README.md · preview"), "the title names the mode:\n{out}");
    let hit = first_painted_link(&app);
    assert_eq!(hit.as_deref(), Some("https://docs.example/x"));
}

#[test]
fn the_changes_tab_paints_the_markdown_preview() {
    let r = Repo::init();
    r.write("README.md", "# Install\n");
    r.commit_all("init");
    r.write("README.md", "# Install\n\nRun `cargo test` for **all** checks.\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;

    // The Changes diff shows raw markdown, and the footer surfaces the way into the preview.
    let source = render(&app);
    assert!(source.contains("# Install"), "the diff shows raw markdown:\n{source}");
    let footer = source.lines().last().unwrap();
    assert!(footer.contains("m preview"), "the diff discovers the preview:\n{footer}");

    // The toggle paints the rendered document over the diff and names the mode in the title.
    app.toggle_preview();
    let out = render(&app);
    assert!(out.contains("README.md · preview"), "the title names the mode:\n{out}");
    assert!(out.contains("Install"), "the heading text renders:\n{out}");
    assert!(!out.contains("# Install"), "the # markers are gone in the preview:\n{out}");
    // "checks" is on the new side only (the committed side is the bare heading), so this
    // proves the preview renders current content, not the old version being diffed.
    assert!(out.contains("checks"), "the preview renders the new-side content:\n{out}");
    let footer = out.lines().last().unwrap();
    assert!(footer.contains("m source"), "the footer leads back to the diff:\n{footer}");
}

#[test]
fn an_uppercase_unicode_anchor_still_finds_its_heading() {
    use std::fmt::Write as _;
    let mut md = String::from("# Über Top\n\njump [go](#ÜBER-TOP)\n\n");
    for i in 0..40 {
        let _ = writeln!(md, "filler {i}\n");
    }
    md.push_str("## Über Ziel\n\nend\n");
    let r = Repo::init();
    r.write("doc.md", &md);
    r.commit_all("init");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    app.toggle_preview();
    let _ = render(&app);

    // The click side must Unicode-lowercase like the slugger: #ÜBER-ZIEL → über-ziel.
    app.open_link("#ÜBER-ZIEL");
    assert!(app.preview_scroll > 40, "the jump matched the slug: {}", app.preview_scroll);
}

// In-file find rendering.
#[test]
fn the_find_band_and_match_highlight_paint() {
    let r = Repo::init();
    r.write("base.txt", "x\n");
    r.commit_all("init");
    r.write("m.rs", "let total = 1;\ncompute();\ntotal += 2;\n");
    let mut app = app_on(&r);
    app.focus = Focus::Diff;
    app.diff_cursor = 0; // the first "total" row
    let keymap = Keymap::default();
    let area = Rect::new(0, 0, 140, 40);

    handle_key(&mut app, KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL), area, &keymap)
        .unwrap();
    for ch in "total".chars() {
        handle_key(&mut app, KeyEvent::from(KeyCode::Char(ch)), area, &keymap).unwrap();
    }

    let buf = render_buffer(&app);
    let out = dump(&buf);
    // The band carries the label, the query, and the count: two matches, the cursor on the first.
    assert!(out.contains("find"), "the band shows the find label:\n{out}");
    assert!(out.contains("1/2"), "the band shows the cursor's ordinal over the total:\n{out}");

    // A matched character reverses to the bright fill with dark text, so it reads over any row.
    let fill = app.palette().yellow;
    let ink = app.palette().surface0;
    let highlighted = (0..40u16).flat_map(|y| (0..140u16).map(move |x| (x, y))).any(|(x, y)| {
        buf.cell((x, y)).is_some_and(|c| c.symbol() == "t" && c.bg == fill && c.fg == ink)
    });
    assert!(highlighted, "a matched character reverses to the bright find highlight");
}

// Search screen rendering.
mod search_screen_render {
    use super::{common, dump, render, render_size};
    use common::{Repo, app_on, enter_tab};
    use diff_reckoner::app::{App, Mode, Tab};
    use diff_reckoner::keymap::default_keymap;
    use diff_reckoner::land_search_completion;
    use diff_reckoner::search::{CodeHit, FileHit, SearchCompletion, SearchOutcome, SearchResults};
    use diff_reckoner::{handle_key, handle_mouse, ui};
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Rect;

    const AREA: Rect = Rect { x: 0, y: 0, width: 140, height: 40 };

    fn open_on_all_files(repo: &Repo) -> App {
        let mut app = app_on(repo);
        enter_tab(&mut app, Tab::AllFiles);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('/')), AREA, default_keymap()).unwrap();
        assert_eq!(app.mode, Mode::Search);
        app
    }

    fn key(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::from(code), AREA, default_keymap()).unwrap();
    }

    fn land(app: &mut App, results: SearchResults) {
        let completion = SearchCompletion { generation: 1, outcome: SearchOutcome::Ready(results) };
        land_search_completion(app, completion, 1);
    }

    #[test]
    fn the_band_anchors_the_terminal_cursor_at_its_caret() {
        let repo = Repo::init();
        repo.write("src/registry.rs", "fn resolve() {}\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
        terminal.draw(|f| ui::render(f, &app)).unwrap();
        let empty = terminal.backend().cursor_position();

        key(&mut app, KeyCode::Char('日'));
        terminal.draw(|f| ui::render(f, &app)).unwrap();

        let after = terminal.backend().cursor_position();
        assert_eq!(
            (after.x, after.y),
            (empty.x + 2, empty.y),
            "the cursor advances one wide character"
        );
    }

    #[test]
    fn screen_shows_band_chips_and_both_modes() {
        let repo = Repo::init();
        repo.write("src/registry.rs", "fn resolve() {}\nregistry.resolve()\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        for c in "reg".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        land(
            &mut app,
            SearchResults {
                files: vec![FileHit { path: "src/registry.rs".into(), spans: vec![(4, 7)] }],
                code: vec![
                    CodeHit {
                        path: "src/registry.rs".into(),
                        line: 1,
                        text: "fn resolve() {}".into(),
                        spans: vec![(3, 6)],
                    },
                    CodeHit {
                        path: "src/registry.rs".into(),
                        line: 2,
                        text: "registry.resolve()".into(),
                        spans: vec![(0, 3)],
                    },
                ],
                file_total: 4,
                code_more: true,
            },
        );

        // Files mode: the band, both chips with live counts, path rows, the Files clip.
        let out = render(&app);
        let band = out.lines().find(|l| l.contains("> reg")).expect("the band row renders");
        assert!(band.contains("files 4 │ code 2+"), "both chips carry a live count: {band}");
        assert!(!band.contains('⇥'), "the chips drop the flip glyph — the footer owns the key");
        assert!(out.contains("src/registry.rs"), "a path match renders as a file row");
        assert!(out.contains("… more"), "a clipped list marks that there is more");
        assert!(out.contains("─ results"), "the results pane carries a titled rule");
        assert!(out.contains("─ preview"), "the divider row carries the preview title");
        assert!(out.contains("↑↓ move") && out.contains("enter open"), "the screen's footer shows");

        // Code mode: grouped rows under a header, `line:` locators, the clip.
        key(&mut app, KeyCode::Tab);
        let out = render(&app);
        assert!(out.contains("> reg"), "the flip keeps the query");
        assert!(out.contains("1: fn resolve"), "a match row shows its line number");
        assert!(out.contains("2: registry.resolve"), "grouped rows keep engine order");
        assert!(out.contains("… more"), "a cut-short grep shows there is more");
        let header_rows =
            out.lines().filter(|l| l.contains("src/registry.rs") && !l.contains(':')).count();
        assert!(header_rows >= 1, "the file emits one header row: {out}");
    }

    #[test]
    fn screen_shows_indexing_until_warm() {
        let repo = Repo::init();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all("c");
        let app = open_on_all_files(&repo);
        let out = render(&app);
        assert!(out.contains("indexing…"), "the screen reads indexing… before the first scan");
        assert!(out.contains("files │ code"), "the count slots stay empty while warming");
    }

    #[test]
    fn no_matches_only_where_the_engine_looked() {
        let repo = Repo::init();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(&mut app, SearchResults::default());
        let out = render(&app);
        assert!(out.contains("no matches"), "an empty warm Files result reads no matches");

        // An empty query lists nothing in Code mode — no copy at all.
        key(&mut app, KeyCode::Tab);
        let out = render(&app);
        assert!(!out.contains("no matches"), "an empty query in Code mode lists nothing");
    }

    #[test]
    fn click_picks_then_opens_and_chip_click_flips() {
        let repo = Repo::init();
        repo.write("a.rs", "one\ntwo\n");
        repo.write("b.rs", "three\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(
            &mut app,
            SearchResults {
                files: vec![
                    FileHit { path: "a.rs".into(), spans: vec![] },
                    FileHit { path: "b.rs".into(), spans: vec![] },
                ],
                code: Vec::new(),
                file_total: 2,
                code_more: false,
            },
        );
        // Paint once so the screen scroll settles, then resolve rows the frame mapped.
        let _ = dump(&render_size(&app, 140, 40));
        let hit_row = |app: &App, pick: usize| {
            (0..40u16)
                .find(|&y| ui::search_target(app, AREA, 30, y) == Some(ui::SearchTarget::Row(pick)))
                .expect("the result row is clickable")
        };
        let click = |app: &mut App, row: u16| {
            let event = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 30,
                row,
                modifiers: KeyModifiers::NONE,
            };
            handle_mouse(
                app,
                event,
                AREA,
                &[],
                default_keymap(),
                &diff_reckoner::export::Clipboard,
            )
            .unwrap();
        };

        // A click on an unpicked row picks it; a second click opens it.
        let row = hit_row(&app, 1);
        click(&mut app, row);
        assert_eq!(app.mode, Mode::Search, "the first click only picks");
        assert_eq!(app.search.as_ref().unwrap().pick, 1);
        click(&mut app, row);
        assert_eq!(app.mode, Mode::Normal, "the second click opens the pick");
        assert_eq!(app.diff_path.as_deref(), Some("b.rs"));

        // A chip click flips the mode.
        let mut app = open_on_all_files(&repo);
        let _ = dump(&render_size(&app, 140, 40));
        let band_y = ui::body_rect(AREA, &app).y;
        let chip_x = (0..140u16)
            .find(|&x| ui::search_target(&app, AREA, x, band_y) == Some(ui::SearchTarget::Chips))
            .expect("the chips are clickable");
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: chip_x,
            row: band_y,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            &mut app,
            event,
            AREA,
            &[],
            default_keymap(),
            &diff_reckoner::export::Clipboard,
        )
        .unwrap();
        assert_eq!(
            app.search.as_ref().unwrap().search_mode,
            diff_reckoner::app::SearchMode::Code,
            "a chip click flips the mode"
        );
    }

    #[test]
    fn preview_centers_and_bands_the_hit() {
        let repo = Repo::init();
        let lines: Vec<String> = (1..=60).map(|i| format!("line_{i}")).collect();
        let body = lines.join("\n") + "\n";
        repo.write("a.rs", &body);
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(
            &mut app,
            SearchResults {
                files: Vec::new(),
                code: vec![CodeHit {
                    path: "a.rs".into(),
                    line: 30,
                    text: "line_30".into(),
                    spans: vec![(0, 7)],
                }],
                file_total: 0,
                code_more: false,
            },
        );
        key(&mut app, KeyCode::Tab);
        app.build_search_preview();

        let buf = render_size(&app, 140, 40);
        let out = dump(&buf);
        assert!(out.contains("─ preview · a.rs"), "the pane title names the previewed file");
        let y = out
            .lines()
            .position(|l| l.contains("30 line_30"))
            .expect("the hit line is visible with its number") as u16;
        let x = out.lines().nth(y as usize).unwrap().find("line_30").unwrap() as u16;
        let style = buf.cell((x, y)).expect("cell").style();
        assert_eq!(
            style.bg,
            Some(app.palette().match_hl),
            "the hit's matched span wears the match highlight: {style:?}"
        );
        assert!(!out.contains(" 1 line_1\n"), "the hit is centered, not previewed from the top");

        // PageDown moves the pane; the scroll survives the next paint.
        key(&mut app, KeyCode::PageDown);
        let scrolled = app.search.as_ref().unwrap().preview.as_ref().unwrap().scroll.get();
        let _ = render_size(&app, 140, 40);
        assert!(scrolled > 0, "PageDown scrolls the preview");
    }

    #[test]
    fn preview_highlight_lands_on_the_match_under_indentation() {
        // The worker trims each grep line's leading indentation and reports offsets into the
        // trimmed text; the preview keeps the true indentation, so the highlight must shift
        // over it and still cover the match, not slide left into the whitespace or the
        // preceding tokens.
        let repo = Repo::init();
        let mut lines: Vec<String> = (1..=60).map(|i| format!("let x{i} = {i};")).collect();
        lines[29] = "    fn resolve() {}".to_string(); // line 30, four-space indented
        repo.write("a.rs", &(lines.join("\n") + "\n"));
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(
            &mut app,
            SearchResults {
                files: Vec::new(),
                // As the worker emits it: the trimmed line, offsets into the trimmed text.
                code: vec![CodeHit {
                    path: "a.rs".into(),
                    line: 30,
                    text: "fn resolve() {}".into(),
                    spans: vec![(3, 10)], // "resolve" within the trimmed line
                }],
                file_total: 0,
                code_more: false,
            },
        );
        key(&mut app, KeyCode::Tab);
        app.build_search_preview();

        let buf = render_size(&app, 140, 40);
        let out = dump(&buf);
        // The results row above is correctly trimmed; assert on the preview row, which keeps
        // the true indentation — that is where the trimmed spans had to be shifted.
        let preview_at =
            out.lines().position(|l| l.contains("─ preview")).expect("the preview divider");
        let below = out
            .lines()
            .skip(preview_at + 1)
            .position(|l| l.contains("fn resolve() {}"))
            .expect("the hit line previews with its indentation");
        let y = (preview_at + 1 + below) as u16;
        let line = out.lines().nth(y as usize).unwrap();
        let rx = line.find("resolve").unwrap() as u16;
        assert_eq!(
            buf.cell((rx, y)).unwrap().style().bg,
            Some(app.palette().match_hl),
            "the highlight lands on the match under indentation",
        );
        // The indentation and the preceding `fn ` keep the cursor band, not the match highlight.
        let fx = line.find("fn ").unwrap() as u16;
        assert_ne!(
            buf.cell((fx, y)).unwrap().style().bg,
            Some(app.palette().match_hl),
            "the highlight did not slide left into the un-trimmed indentation",
        );
    }

    #[test]
    fn tiny_screen_keeps_the_band() {
        let repo = Repo::init();
        repo.write("a.rs", "one\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(
            &mut app,
            SearchResults {
                files: vec![FileHit { path: "a.rs".into(), spans: vec![] }],
                code: Vec::new(),
                file_total: 1,
                code_more: false,
            },
        );
        app.build_search_preview();
        let out = dump(&render_size(&app, 24, 6));
        assert!(out.contains('>'), "the input band keeps its one row at tiny sizes");
    }

    #[test]
    fn empty_query_shows_a_placeholder_and_no_preview() {
        let repo = Repo::init();
        repo.write("a.rs", "one\n");
        repo.commit_all("c");
        let app = open_on_all_files(&repo);
        // Warm but no results landed yet: the band teaches, the preview isn't blank.
        let out = render(&app);
        assert!(out.contains("Search files and code…"), "the empty query shows a placeholder");
        let mut app = app;
        land(&mut app, SearchResults::default());
        let out = render(&app);
        assert!(out.contains("no preview"), "nothing to preview shows a dim notice, not a blank");
    }

    #[test]
    fn an_elided_file_result_still_highlights_the_visible_match() {
        // A head-elided path must still mark a match that survives in the shown tail — the
        // highlight is unconditional, remapped across the elision, not dropped.
        let repo = Repo::init();
        let path = "aaaaaaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbb/target_match.rs";
        repo.write(path, "x\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        let at = path.find("target").unwrap() as u32;
        land(
            &mut app,
            SearchResults {
                files: vec![FileHit { path: path.into(), spans: vec![(at, at + 6)] }],
                code: Vec::new(),
                file_total: 1,
                code_more: false,
            },
        );
        // A pane narrow enough to head-elide the long path onto its tail.
        let buf = render_size(&app, 44, 20);
        let out = dump(&buf);
        let y = out
            .lines()
            .position(|l| l.contains('…') && l.contains("target"))
            .expect("the elided path row shows its tail") as u16;
        let line = out.lines().nth(y as usize).unwrap();
        let tx = line.find("target").unwrap() as u16;
        assert_eq!(
            buf.cell((tx, y)).unwrap().style().bg,
            Some(app.palette().match_hl),
            "the match highlight survives the head-elision on the visible tail",
        );
    }

    #[test]
    fn long_query_scrolls_to_keep_the_caret_visible() {
        let repo = Repo::init();
        repo.write("a.rs", "one\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(&mut app, SearchResults::default()); // warm, so the chips have a fixed width
        // The caret sits at the end of the query; a band narrower than the query must
        // scroll its head off and keep the tail (and caret) on screen.
        let query = "aaaaHEAD_bbbbccccddddeeeeffffgggg_TAILzzzz";
        for c in query.chars() {
            key(&mut app, KeyCode::Char(c));
        }
        let out = dump(&render_size(&app, 44, 12));
        let band = out.lines().find(|l| l.contains("TAIL")).expect("the caret end stays visible");
        assert!(!band.contains("HEAD"), "the overflowing head scrolls off the band: {band:?}");
    }

    #[test]
    fn a_changed_file_result_shows_its_marker_and_stats() {
        // A Files result on an uncommitted file wears the same change marker and stats as the
        // file list, alongside the match highlight.
        let repo = Repo::init();
        repo.write("a.rs", "one\n");
        repo.commit_all("c");
        repo.write("a.rs", "one\ntwo\n"); // uncommitted: one added line
        let mut app = open_on_all_files(&repo);
        land(
            &mut app,
            SearchResults {
                files: vec![FileHit { path: "a.rs".into(), spans: vec![(0, 1)] }],
                code: Vec::new(),
                file_total: 1,
                code_more: false,
            },
        );
        let buf = render_size(&app, 140, 40);
        let out = dump(&buf);
        let row = out.lines().find(|l| l.contains("a.rs")).expect("the file row renders");
        assert!(row.contains("+1"), "the changed file's stats render on its row: {row:?}");
        // The match highlight coexists with the marker and stats.
        let y = out.lines().position(|l| l.contains("a.rs")).unwrap() as u16;
        let x = row.find("a.rs").unwrap() as u16;
        assert_eq!(
            buf.cell((x, y)).expect("cell").style().bg,
            Some(app.palette().match_hl),
            "the match highlight lands on the matched path character",
        );
    }

    #[test]
    fn a_poll_refreshes_the_open_preview_in_place() {
        // A landed poll rebuilds the previewed file's diff in place, so the preview follows
        // the worktree while the held results stay as queried (Continuity). Exercises the real reload → reconcile_world → refresh_search_preview
        // wiring, not the method in isolation.
        let repo = Repo::init();
        repo.write("a.rs", "alpha\n");
        repo.commit_all("c");
        let mut app = open_on_all_files(&repo);
        land(
            &mut app,
            SearchResults {
                files: Vec::new(),
                code: vec![CodeHit {
                    path: "a.rs".into(),
                    line: 1,
                    text: "alpha".into(),
                    spans: vec![(0, 5)],
                }],
                file_total: 0,
                code_more: false,
            },
        );
        key(&mut app, KeyCode::Tab);
        app.build_search_preview();
        assert!(render(&app).contains("alpha"), "the preview shows the file's content");

        // The worktree changes, then a poll lands through the synchronous reload path.
        repo.write("a.rs", "alpha\nBETA_LINE\n");
        app.reload().unwrap();
        assert!(render(&app).contains("BETA_LINE"), "the poll refreshed the preview in place");
    }
}

// Style-level emphasis coverage for the match rows.
mod search_row_emphasis {
    use super::{common, dump, render_size};
    use common::{Repo, app_on, enter_tab};
    use diff_reckoner::app::Tab;
    use diff_reckoner::handle_key;
    use diff_reckoner::keymap::default_keymap;
    use diff_reckoner::land_search_completion;
    use diff_reckoner::search::{CodeHit, SearchCompletion, SearchOutcome, SearchResults};
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::layout::Rect;

    const AREA: Rect = Rect { x: 0, y: 0, width: 140, height: 40 };

    fn code_only(hit: CodeHit) -> SearchCompletion {
        let results =
            SearchResults { files: Vec::new(), code: vec![hit], file_total: 0, code_more: false };
        SearchCompletion { generation: 1, outcome: SearchOutcome::Ready(results) }
    }

    /// A code row too wide for the pane clips around its first matched span, keeping the
    /// `line:` locator and marking the cut head with `…`.
    #[test]
    fn clipped_code_row_keeps_and_emphasizes_the_match() {
        let repo = Repo::init();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all("c");
        let mut app = app_on(&repo);
        enter_tab(&mut app, Tab::AllFiles);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('/')), AREA, default_keymap()).unwrap();

        // A long head of `x`s pushes the match past the pane, so the row clips around the
        // first matched span (`needle_marker`, 13 bytes) rather than the un-shown head.
        let text = format!("{}needle_marker tail", "x".repeat(200));
        let start = 200u32;
        let hit = CodeHit { path: "a.rs".into(), line: 1, text, spans: vec![(start, start + 13)] };
        land_search_completion(&mut app, code_only(hit), 1);
        handle_key(&mut app, KeyEvent::from(KeyCode::Tab), AREA, default_keymap()).unwrap();

        let buf = render_size(&app, 140, 40);
        let out = dump(&buf);
        let row = out
            .lines()
            .find(|l| l.contains("needle_marker"))
            .expect("the clipped row keeps the first matched span visible");
        assert!(row.contains("1:"), "the line locator survives the clip: {row}");
        assert!(row.contains("…x"), "the cut head is marked with an ellipsis: {row}");

        let y = out.lines().position(|l| l.contains("needle_marker")).unwrap() as u16;
        // Cell column = char count before the token (every cell here is one column wide).
        let byte = row.find("needle_marker").unwrap();
        let x = row[..byte].chars().count() as u16;
        assert_eq!(
            buf.cell((x, y)).expect("cell").style().bg,
            Some(app.palette().match_hl),
            "the matched span wears the match highlight",
        );
        // A cell in the clipped `…x` head keeps the selection fill, not the match highlight —
        // the band covers the match only, never spilling left across the cut.
        let ell = row.find('…').unwrap();
        let head_x = row[..ell].chars().count() as u16 + 1;
        assert_ne!(
            buf.cell((head_x, y)).expect("cell").style().bg,
            Some(app.palette().match_hl),
            "the clipped head is not highlighted",
        );
    }

    /// A tab-indented code row expands its tabs to spaces, so the indentation shows and
    /// the emphasis lands on the matched word, not shifted by the collapsed tabs
    #[test]
    fn tab_indented_code_row_expands_and_emphasizes() {
        let repo = Repo::init();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all("c");
        let mut app = app_on(&repo);
        enter_tab(&mut app, Tab::AllFiles);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('/')), AREA, default_keymap()).unwrap();

        // Two leading tabs, then `needle` — the match is at bytes 2..8 of the raw line.
        let hit = CodeHit {
            path: "a.rs".into(),
            line: 1,
            text: "\t\tneedle here".into(),
            spans: vec![(2, 8)],
        };
        land_search_completion(&mut app, code_only(hit), 1);
        handle_key(&mut app, KeyEvent::from(KeyCode::Tab), AREA, default_keymap()).unwrap();

        let buf = render_size(&app, 140, 40);
        let out = dump(&buf);
        let y = out.lines().position(|l| l.contains("needle")).unwrap();
        let row = out.lines().nth(y).unwrap();
        // Eight spaces of expanded indent sit between the locator and `needle`.
        assert!(row.contains("1:         needle"), "tabs expand to spaces: {row:?}");
        let x = row.find("needle").unwrap() as u16;
        let style = buf.cell((x, y as u16)).expect("cell").style();
        assert_eq!(
            style.bg,
            Some(app.palette().match_hl),
            "the highlight tracks the word past the expanded tabs: {style:?}"
        );
    }

    /// A multi-byte head forced through the clip path must paint, not panic — the
    /// engine's span offsets are bytes and the cut walks char boundaries.
    #[test]
    fn clipped_multibyte_code_row_paints() {
        let repo = Repo::init();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all("c");
        let mut app = app_on(&repo);
        enter_tab(&mut app, Tab::AllFiles);
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('/')), AREA, default_keymap()).unwrap();

        // A wide multi-byte head (each `中` is 3 bytes, 2 columns) forces the clip's
        // char-boundary walk onto boundaries a byte/column confusion would land off.
        let head = "中".repeat(200);
        let start = head.len() as u32; // 600 bytes in
        let hit = CodeHit {
            path: "a.rs".into(),
            line: 1,
            text: format!("{head}needle tail"),
            spans: vec![(start, start + 6)],
        };
        land_search_completion(&mut app, code_only(hit), 1);
        handle_key(&mut app, KeyEvent::from(KeyCode::Tab), AREA, default_keymap()).unwrap();

        let buf = render_size(&app, 140, 40);
        let out = dump(&buf);
        let y = out
            .lines()
            .position(|l| l.contains("needle"))
            .expect("the clipped multibyte row paints without panicking") as u16;
        // The highlight starts exactly on the match, not shifted onto the multibyte head:
        // the first highlighted cell on the row is `needle`'s `n`.
        let hx = (0..buf.area.width)
            .find(|&x| buf.cell((x, y)).expect("cell").style().bg == Some(app.palette().match_hl))
            .expect("the match is highlighted");
        assert_eq!(
            buf.cell((hx, y)).expect("cell").symbol(),
            "n",
            "the match highlight lands on the match, past the multibyte head",
        );
    }
}

// --- Agent picker ----------------------------------------------------

// --- Header base label ------------------------------------------------------

/// A repo on branch `feature` past `main`, with `origin/HEAD` naming `main` the default.
/// The repo rides along: opening the picker shells out to git at click time.
fn based_app() -> (Repo, App) {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["branch", "dev"]);
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    (r, app)
}

#[test]
fn the_branch_header_names_the_base_and_its_click_opens_the_picker() {
    let (_repo, mut app) = based_app();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(line0.contains("[branch] vs main"), "the bare base name follows the scope: {line0}");

    let base: Vec<u16> = (0..AREA.width)
        .filter(|&c| ui::hit_header(AREA, &app, app.keymap(), c, 0) == Some(HeaderHit::Base))
        .collect();
    assert!(!base.is_empty(), "the base label is clickable");
    let click = MouseEvent {
        kind: MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
        column: base[0],
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    let keymap = app.keymap().clone();
    handle_mouse(&mut app, click, AREA, &[], &keymap, &diff_reckoner::export::Clipboard).unwrap();
    let frame = render(&app);
    assert!(frame.contains("base · 3 branches"), "the click opens the picker popup");
    assert!(frame.contains("dev"), "the sibling branch is a row");
    assert!(frame.contains("default"), "the default branch is marked");
    assert!(frame.contains("current"), "the checked-out branch is marked");
    assert!(!frame.contains('★'), "no glyph: the trail words carry the facts");

    // The box is sized to its rows: the filter line, three branch rows, two borders, and
    // no blank row held for a probe that is not showing.
    let top = frame.lines().position(|l| l.contains("┌ base")).unwrap();
    let bottom = frame.lines().skip(top).position(|l| l.contains("└────")).unwrap();
    assert_eq!(bottom, 5, "top border, filter line, three rows, bottom border: {frame}");
}

#[test]
fn the_picker_title_counts_matches_while_filtering() {
    let (r, mut app) = based_app();
    app.open_base_picker();
    assert!(render(&app).contains("base · 3 branches"));
    app.input_push('d');
    assert!(render(&app).contains("base · 1/3"), "matched over total: {}", render(&app));
    app.close_base_picker();

    // A current non-branch pick is a row, never a count: `HEAD~1` is listed under the
    // three branches but both numbers still say three.
    diff_reckoner::git::write_base_pick(r.path(), "HEAD~1").unwrap();
    app.set_scope(Scope::Branch).unwrap();
    app.open_base_picker();
    let frame = render(&app);
    assert!(frame.contains("HEAD~1"), "{frame}");
    assert!(frame.contains("base · 3 branches"), "{frame}");
    app.input_push('e');
    let frame = render(&app);
    assert!(
        frame.contains("base · 2/3"),
        "dev and feature match, the rev row counts nowhere: {frame}"
    );
}

#[test]
fn a_probe_row_still_fits_when_every_branch_matches() {
    // One branch, and it matches the query: the frozen full-list height has no spare
    // row, so the box must grow by the hit's row or the tag is unpainted and unclickable.
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["checkout", "-q", "-b", "v1.2-hotfix"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    r.git(&["branch", "-D", "main"]);
    r.git(&["tag", "v1.2", "HEAD~1"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    app.open_base_picker();
    for ch in "v1.2".chars() {
        app.input_push(ch);
    }
    app.run_base_probe();
    let frame = render(&app);
    assert!(frame.contains("v1.2-hotfix"), "{frame}");
    assert!(frame.contains('(') && frame.contains("v1.2 "), "the tag row paints: {frame}");
    let rows: std::collections::BTreeSet<usize> = (0..AREA.height)
        .flat_map(|row| (0..AREA.width).map(move |col| (col, row)))
        .filter_map(|(col, row)| ui::hit_base_picker_row(AREA, &app, col, row))
        .collect();
    assert_eq!(rows.into_iter().collect::<Vec<_>>(), [0, 1], "the hit row is inside the box");
}

#[test]
fn a_probe_row_is_clickable_below_the_matches() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["branch", "v1.2-hotfix"]);
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    r.git(&["tag", "v1.2", "HEAD~1"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    app.open_base_picker();
    for ch in "v1.2".chars() {
        app.input_push(ch);
    }
    app.run_base_probe();
    let frame = render(&app);
    assert!(frame.contains("v1.2-hotfix"), "{frame}");
    let hits: Vec<(u16, u16, usize)> = (0..AREA.height)
        .flat_map(|row| (0..AREA.width).map(move |col| (col, row)))
        .filter_map(|(col, row)| {
            ui::hit_base_picker_row(AREA, &app, col, row).map(|i| (col, row, i))
        })
        .collect();
    let rows: std::collections::BTreeSet<usize> = hits.iter().map(|h| h.2).collect();
    assert_eq!(
        rows.into_iter().collect::<Vec<_>>(),
        [0, 1],
        "both rows hit-test, the probe row too"
    );
    let probe_y = hits.iter().find(|h| h.2 == 1).unwrap().1;
    let branch_y = hits.iter().find(|h| h.2 == 0).unwrap().1;
    assert_eq!(probe_y, branch_y + 1, "the probe row sits under the match");
}

#[test]
fn a_named_rev_paints_the_spelling_and_abbrev() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    diff_reckoner::git::write_base_pick(r.path(), "HEAD~1").unwrap();
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    assert!(
        line0.contains(&format!("vs HEAD~1 ({short})")),
        "a named rev paints the spelling and the abbreviated SHA: {line0}"
    );
}

#[test]
fn a_sha_pick_paints_once() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    diff_reckoner::git::write_base_pick(r.path(), &parent).unwrap();
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(line0.contains(&format!("vs {short}")), "a SHA spelling paints once: {line0}");
    assert!(
        !line0.contains(&format!("vs {short} (")),
        "a SHA spelling does not repeat as a marker: {line0}"
    );

    diff_reckoner::git::write_base_pick(r.path(), &short).unwrap();
    app.reload().unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(
        line0.contains(&format!("vs {short}")),
        "an abbreviated SHA spelling paints once: {line0}"
    );
    assert!(
        !line0.contains(&format!("vs {short} (")),
        "an abbreviated SHA spelling does not repeat as a marker: {line0}"
    );
}

#[test]
fn a_flag_named_rev_paints_the_same_form() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    let mut app = App::new(r.path_buf(), Scope::Branch, Some("HEAD~1".to_string()));
    app.reload().unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    assert!(
        line0.contains(&format!("vs HEAD~1 ({short})")),
        "the --base flag uses the same paint: {line0}"
    );
}

#[test]
fn a_probe_row_is_the_typed_spelling() {
    let (r, mut app) = based_app();
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    app.open_base_picker();
    for ch in "HEAD~1".chars() {
        app.input_push(ch);
    }
    app.run_base_probe();
    let frame = render(&app);
    assert!(frame.contains("HEAD~1"), "the probe row is the typed spelling:\n{frame}");
    assert!(
        frame.contains(&format!("({short})")),
        "a named rev is marked with the abbreviated SHA:\n{frame}"
    );
}

#[test]
fn a_short_sha_prefix_probe_completes_to_the_abbrev() {
    let (r, mut app) = based_app();
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    let prefix = short[..4].to_string();
    app.open_base_picker();
    for ch in prefix.chars() {
        app.input_push(ch);
    }
    app.run_base_probe();
    let bp = app.base_picker.as_ref().unwrap();
    assert_eq!(bp.visible()[0].name(), short, "the row is the abbreviated SHA, not the prefix");
    let frame = render(&app);
    assert!(
        frame.lines().any(|l| l.contains(&short) && !l.contains(&format!("({short})"))),
        "a unique prefix completes to the abbreviated SHA with no marker:\n{frame}"
    );
}

#[test]
fn a_seven_char_sha_probe_is_not_marked() {
    let (r, mut app) = based_app();
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    app.open_base_picker();
    for ch in short.chars() {
        app.input_push(ch);
    }
    app.run_base_probe();
    let frame = render(&app);
    assert!(frame.contains(&short), "the probe row is the abbreviated SHA:\n{frame}");
    assert!(
        !frame.contains(&format!("({short})")),
        "a spelling that already is that SHA carries no marker:\n{frame}"
    );
}

#[test]
fn a_skipped_named_rev_uses_the_stored_spelling() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    diff_reckoner::git::write_base_pick(r.path(), "HEAD~1").unwrap();
    r.git(&["checkout", "-q", "-b", "feature"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(
        line0.contains("vs main · HEAD~1 missing"),
        "a skipped non-branch spelling uses the stored spelling: {line0}"
    );
}

#[test]
fn a_skipped_pick_warns_beside_the_resolved_base() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    diff_reckoner::git::write_base_pick(r.path(), "gone").unwrap();
    r.git(&["checkout", "-q", "-b", "feature"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(line0.contains("vs main · gone missing"), "the dormant pick reads as skipped: {line0}");
}

#[test]
fn without_a_resolving_base_the_header_reads_no_base() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.git(&["branch", "-m", "main", "trunk"]); // no `main`/`master`: no default to fall back on
    r.git(&["checkout", "-q", "-b", "feature"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let frame = render(&app);
    let line0 = frame.lines().next().unwrap().to_string();
    assert!(line0.contains("[branch] no base"), "the empty state is named: {line0}");
    assert!(frame.contains("B base"), "the footer advertises the picker");
}

#[test]
fn a_local_only_repo_has_its_main_as_the_base() {
    // No remote at all: `origin/HEAD` names nothing, and the local `main` is the default,
    // so the header never reads `no base` in a repo that plainly has a trunk.
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(line0.contains("[branch] vs main"), "the local main is the base: {line0}");
    assert!(!line0.contains("missing"), "nothing is skipped: {line0}");
    assert!(line0.contains("1 changed"), "the branch diffs against it: {line0}");

    // On `main` itself the base is still `main`: the scope is the uncommitted diff.
    r.git(&["checkout", "-q", "main"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(line0.contains("[branch] vs main"), "{line0}");
}

#[test]
fn a_dormant_pick_shows_beside_the_empty_state() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.git(&["branch", "-m", "main", "trunk"]); // no `main`/`master`: no default to fall back on
    diff_reckoner::git::write_base_pick(r.path(), "gone").unwrap();
    r.git(&["checkout", "-q", "-b", "feature"]);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = render(&app).lines().next().unwrap().to_string();
    assert!(
        line0.contains("no base · gone missing"),
        "a dormant choice never reads as never-chosen: {line0}"
    );
}

#[test]
fn a_named_rev_clips_the_spelling_and_keeps_the_sha() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let parent = r.git(&["rev-parse", "HEAD~1"]).trim().to_string();
    let long = format!("release-{}", "x".repeat(80));
    r.git(&["tag", &long, &parent]);
    diff_reckoner::git::write_base_pick(r.path(), &long).unwrap();
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = dump(&render_size(&app, 80, 20)).lines().next().unwrap().to_string();
    let short = diff_reckoner::git::abbreviate_oid(&parent);
    assert!(line0.contains(&format!("({short})")), "the SHA marker survives the clip: {line0}");
    assert!(line0.contains('…'), "the spelling truncates: {line0}");
    assert!(line0.contains("1 changed"), "the right-aligned stats survive: {line0}");
}

#[test]
fn an_overlong_base_name_truncates_with_an_ellipsis() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    let long = format!("feature/{}", "x".repeat(80));
    r.git(&["branch", &long]);
    diff_reckoner::git::write_base_pick(r.path(), &long).unwrap();
    r.git(&["checkout", "-q", "-b", "work"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = dump(&render_size(&app, 80, 20)).lines().next().unwrap().to_string();
    assert!(line0.contains("vs feature/x"), "the name paints up to the fit: {line0}");
    assert!(line0.contains('…'), "the overflow truncates with a trailing ellipsis: {line0}");
    assert!(line0.contains("1 changed"), "the right-aligned stats survive the long name: {line0}");
}

#[test]
fn a_narrow_header_never_maps_a_click_outside_the_painted_base() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    let long = format!("feature/{}", "x".repeat(60));
    r.git(&["branch", &long]);
    diff_reckoner::git::write_base_pick(r.path(), &long).unwrap();
    r.git(&["checkout", "-q", "-b", "work"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();

    // The base label truncates to its budget at a narrow width, and the hit test walks the
    // same arithmetic the paint does: every column it claims carries painted label, and the
    // claim is one unbroken run.
    for width in [40u16, 56, 72] {
        let area = Rect { x: 0, y: 0, width, height: 12 };
        let line0 = dump(&render_size(&app, width, 12)).lines().next().unwrap().to_string();
        let cells: Vec<char> = line0.chars().collect();
        let hits: Vec<u16> = (0..width)
            .filter(|&c| ui::hit_header(area, &app, app.keymap(), c, 0) == Some(HeaderHit::Base))
            .collect();
        let Some((&first, &last)) = hits.first().zip(hits.last()) else {
            // Too narrow for even one column of the name: the base left the header whole,
            // so nothing paints a nameless `vs` and nothing claims it.
            assert!(!line0.contains("vs"), "width {width}: a nameless `vs` paints: {line0}");
            assert!(line0.contains("[branch]"), "width {width}: the scope survives: {line0}");
            continue;
        };
        assert_eq!(
            hits.len() as u16,
            last - first + 1,
            "width {width}: the base claims one unbroken run"
        );
        let claimed: String = hits.iter().map(|&c| cells[c as usize]).collect();
        assert_ne!(claimed.trim(), "vs", "width {width}: a nameless `vs` is claimed: {line0}");
        assert!(
            !claimed.ends_with(' '),
            "width {width}: the claim runs past the painted label: {line0}"
        );
        assert_eq!(
            cells.get(last as usize + 1).copied(),
            Some(' '),
            "width {width}: the claim stops short of the painted label: {line0}"
        );
    }
}

#[test]
fn an_overlong_skipped_tail_never_evicts_the_base_name() {
    let r = Repo::init();
    r.write("hello.rs", "alpha\n");
    r.commit_all("init");
    r.set_origin_default("main", "main");
    let long = format!("feature/{}", "x".repeat(80));
    diff_reckoner::git::write_base_pick(r.path(), &long).unwrap();
    r.git(&["checkout", "-q", "-b", "work"]);
    r.write("hello.rs", "alpha\nBETA\n");
    r.commit_all("edit");
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    let line0 = dump(&render_size(&app, 80, 20)).lines().next().unwrap().to_string();
    assert!(line0.contains("vs main"), "the resolved name keeps first claim: {line0}");
    assert!(line0.contains("· feature/x"), "the skipped tail paints in what remains: {line0}");
    assert!(line0.contains('…'), "the tail truncates with a trailing ellipsis: {line0}");
    assert!(line0.contains("1 changed"), "the right-aligned stats survive the long tail: {line0}");
}

// ---- mouse text selection ----

/// A repo with one uncommitted three-line file, for selection geometry.
fn selection_app() -> (Repo, App) {
    let r = Repo::init();
    r.write("base.rs", "fn main() {}\n");
    r.commit_all("init");
    r.write("m.rs", "alpha beta\n\tif x {\n日本 z\n");
    let app = app_on(&r);
    (r, app)
}

#[test]
fn the_hovered_row_shows_a_plus_in_its_change_bar_cell() {
    let (_repo, mut app) = selection_app();
    let area = Rect::new(0, 0, 140, 40);
    let inner = ui::read_inner_rect(area, &app);

    // No hover: the insertion row paints its change bar.
    let buf = render_buffer(&app);
    assert_eq!(buf.cell((inner.x, inner.y)).unwrap().symbol(), "▌");

    // Hovering anywhere on the row puts the `[+]` button over the number field; the
    // change bar stays, so the diff signal never blinks.
    app.hover = Some((inner.x + 8, inner.y));
    let buf = render_buffer(&app);
    assert_eq!(buf.cell((inner.x, inner.y)).unwrap().symbol(), "▌");
    assert_eq!(buf.cell((inner.x + 1, inner.y)).unwrap().symbol(), "[");
    assert_eq!(buf.cell((inner.x + 2, inner.y)).unwrap().symbol(), "+");
    assert_eq!(buf.cell((inner.x + 3, inner.y)).unwrap().symbol(), "]");
    // The unhovered row below keeps its bar and number.
    assert_eq!(buf.cell((inner.x, inner.y + 1)).unwrap().symbol(), "▌");
}

#[test]
fn the_plus_button_right_aligns_in_a_wide_number_field() {
    // A 1000-line file widens the number field past the 3-column minimum, so the
    // right-aligned `[+]` leaves blank padding on its left, like the numbers it
    // replaces.
    let r = Repo::init();
    r.write("base.rs", "fn main() {}\n");
    r.commit_all("init");
    let body = (1..=1000).fold(String::new(), |mut s, i| {
        use std::fmt::Write;
        let _ = writeln!(s, "line {i}");
        s
    });
    r.write("long.rs", &body);
    let mut app = app_on(&r);
    let area = Rect::new(0, 0, 140, 40);
    let inner = ui::read_inner_rect(area, &app);

    app.hover = Some((inner.x + 8, inner.y));
    let buf = render_buffer(&app);
    assert_eq!(buf.cell((inner.x, inner.y)).unwrap().symbol(), "▌");
    assert_eq!(buf.cell((inner.x + 1, inner.y)).unwrap().symbol(), " ", "left pad, not `[`");
    assert_eq!(buf.cell((inner.x + 2, inner.y)).unwrap().symbol(), "[");
    assert_eq!(buf.cell((inner.x + 3, inner.y)).unwrap().symbol(), "+");
    assert_eq!(buf.cell((inner.x + 4, inner.y)).unwrap().symbol(), "]");
    // The unhovered row below right-aligns its number in the same field.
    assert_eq!(buf.cell((inner.x + 4, inner.y + 1)).unwrap().symbol(), "2");
}

#[test]
fn the_text_selection_highlights_the_dragged_span() {
    use diff_reckoner::selection::{Point, Surface, TextDrag};
    let (_repo, mut app) = selection_app();
    let area = Rect::new(0, 0, 140, 40);
    let inner = ui::read_inner_rect(area, &app);
    let sel_bg = app.palette().sel_bg;
    // The selection fill is its own slot, distinct by hue from the cursor fills, so a
    // selection reads inside a cursor row. Park the cursor on the fully
    // selected middle row so its cells still assert the selection fill won.
    app.diff_cursor = 1;

    // `beta` on row 0 through char 1 (`本`) of row 2: a three-row stream selection.
    app.gesture = diff_reckoner::selection::Gesture::Text {
        drag: TextDrag {
            surface: Surface::Read,
            anchor: Point { row: 0, chr: 6 },
            extent: Point { row: 2, chr: 1 },
        },
        count: 1,
    };
    let buf = render_buffer(&app);
    let bg = |x: u16, y: u16| buf.cell((x, y)).unwrap().style().bg;
    // The `b` of beta is selected; the chars before the anchor are not — the first row runs
    // from its start character, not whole.
    assert_eq!(bg(inner.x + 5 + 6, inner.y), Some(sel_bg));
    assert_ne!(bg(inner.x + 5, inner.y), Some(sel_bg));
    assert_ne!(bg(inner.x + 5 + 5, inner.y), Some(sel_bg));
    // Row 1 lies whole between the endpoints: tab expansion through its last char.
    assert_eq!(bg(inner.x + 5, inner.y + 1), Some(sel_bg));
    assert_eq!(bg(inner.x + 5 + 6, inner.y + 1), Some(sel_bg));
    // Row 2 runs up to its end character: both wide glyphs (each asserted at its first
    // cell — the buffer diff skips a wide char's hidden continuation cell), and nothing
    // past them.
    assert_eq!(bg(inner.x + 5, inner.y + 2), Some(sel_bg));
    assert_eq!(bg(inner.x + 5 + 2, inner.y + 2), Some(sel_bg));
    assert_ne!(bg(inner.x + 5 + 4, inner.y + 2), Some(sel_bg));
}

// --- Commit picker and the commits header --------------------

/// `main` with four commits, root first, plus an uncommitted edit. Returns the shas root
/// first.
fn commits_app() -> (Repo, App, Vec<String>) {
    let r = Repo::init();
    r.write("root.rs", "r\n");
    r.commit_all("root");
    r.write("one.rs", "1\n");
    r.commit_all("one");
    r.write("two.rs", "2\n");
    r.commit_all("two");
    r.write("three.rs", "3\n");
    r.commit_all("Stop counting git's own lock files");
    r.write("root.rs", "dirty\n");
    let shas: Vec<String> =
        r.git(&["rev-list", "--reverse", "HEAD"]).lines().map(str::to_string).collect();
    let app = app_on(&r);
    (r, app, shas)
}

#[test]
fn a_wide_glyph_author_keeps_the_age_column() {
    let (r, mut app, _) = commits_app();
    r.write("four.rs", "4\n");
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "four", "--author=田中太郎 <t@example.com>"]);
    app.open_commit_picker();
    let out = render(&app);
    let ages: Vec<usize> = out
        .lines()
        .skip(1)
        .filter(|l| l.contains("  ") && (l.contains("Test") || l.contains("田")))
        // The test backend dumps one char per cell, so a char index is a column.
        .map(|l| {
            let cells: Vec<char> = l.trim_end_matches([' ', '│']).chars().collect();
            cells.iter().rposition(|c| *c == ' ').unwrap()
        })
        .collect();
    assert!(out.contains("田"), "{out}");
    assert!(ages.len() >= 2 && ages.iter().all(|&a| a == ages[0]), "ages align:\n{out}");
}

/// The default-right navigator's first inner column on the 140-wide test frame.
const FILES_X0: u16 = 140 - 140 * 32 / 100 + 1;
/// Its last inner column: the frame edge less the right border.
const FILES_X1: u16 = 140 - 2;

/// The files-pane row holding `token`: its y, and its text from the pane's first inner
/// column to its last, untrimmed, so a test can check both what a row ends in and where.
fn files_row_at(buf: &Buffer, token: &str) -> (u16, String) {
    let row = |y: u16| -> String {
        (FILES_X0..=FILES_X1).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect()
    };
    let y = (0..buf.area.height)
        .find(|&y| row(y).contains(token))
        .unwrap_or_else(|| panic!("no files-pane row holds {token:?}"));
    (y, row(y))
}

/// The files-pane row holding `token`, trailing padding trimmed.
fn files_row(app: &App, token: &str) -> String {
    files_row_at(&render_buffer(app), token).1.trim_end().to_string()
}

/// Whether the row holding `token` ends in the dot, painted in the pane's last inner column
/// and in the `M` marker's color (the cell style of `m_marker_fg`).
fn dot_at_edge(app: &App, token: &str) -> bool {
    let buf = render_buffer(app);
    let (y, _) = files_row_at(&buf, token);
    let cell = buf.cell((FILES_X1, y)).unwrap();
    cell.symbol() == "•" && cell.style().fg == Some(m_marker_fg(&buf))
}

/// The color of the `M` marker on the fixtures' edited `zz.rs` row.
fn m_marker_fg(buf: &Buffer) -> ratatui::style::Color {
    let (y, text) = files_row_at(buf, "M zz.rs");
    let x = FILES_X0 + text.find("M zz.rs").unwrap() as u16;
    buf.cell((x, y)).unwrap().style().fg.expect("the marker is colored")
}

/// A worktree with `src/{app.rs,ui.rs}` and `docs/{a.md,b.md}` committed and `src/ui.rs`
/// edited, plus a top-level edited `zz.rs` so an `M` marker is always painted for the color
/// reference.
fn dotted_repo() -> Repo {
    let r = Repo::init();
    r.write("src/app.rs", "x\n");
    r.write("src/ui.rs", "y\n");
    r.write("docs/a.md", "a\n");
    r.write("docs/b.md", "b\n");
    r.write("zz.rs", "z\n");
    r.commit_all("init");
    r.write("src/ui.rs", "y2\n");
    r.write("zz.rs", "z2\n");
    r
}

#[test]
fn a_collapsed_all_files_folder_with_a_change_wears_a_dot() {
    let r = dotted_repo();
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    assert!(dot_at_edge(&app, "src/"), "src/ holds the edit: {:?}", files_row(&app, "src/"));
    assert!(!files_row(&app, "docs/").contains('•'), "docs/ holds no change");
    assert!(files_row(&app, "src/").starts_with("▸ src/"), "the chevron is the first column");

    // Expanded, the children carry their own marker and the folder drops the dot.
    app.focus = Focus::Files;
    app.file_cursor = app.file_rows.iter().position(|r| r.dir_path() == Some("src")).unwrap();
    app.expand_dir();
    assert!(!files_row(&app, "src/").contains('•'), "an expanded folder wears no dot");
    assert!(files_row(&app, "ui.rs").starts_with("  M ui.rs"), "the child carries the marker");
}

#[test]
fn a_kept_change_under_an_ignored_folder_wears_the_same_dot() {
    // The folder name dims, the dot keeps its color.
    let r = dotted_repo();
    r.write(".gitignore", "vendor/\n");
    r.write("vendor/lib.rs", "v\n");
    r.write("vendor/other.rs", "o\n");
    r.git(&["add", "-f", "vendor/lib.rs", "vendor/other.rs", ".gitignore"]);
    r.commit_all("vendor");
    r.write("vendor/lib.rs", "v2\n");
    r.write("zz.rs", "z3\n");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    assert!(dot_at_edge(&app, "vendor/"), "{:?}", files_row(&app, "vendor/"));
}

#[test]
fn a_folder_whose_only_change_has_no_row_wears_no_dot() {
    // A staged deletion leaves the index, so `All files` has no row for it and the folder
    // stays quiet: a dot there would open onto nothing. A plain `rm` keeps the index entry,
    // so its row stays, marked `D`, and the folder wears the dot.
    let r = dotted_repo();
    r.write("gone/a.rs", "a\n");
    r.write("gone/b.rs", "b\n");
    r.write("rmd/a.rs", "a\n");
    r.write("rmd/b.rs", "b\n");
    r.commit_all("more");
    r.git(&["rm", "-q", "gone/a.rs"]);
    std::fs::remove_file(r.path_buf().join("rmd/a.rs")).unwrap();
    r.write("zz.rs", "z3\n");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    assert!(!files_row(&app, "gone/").contains('•'), "{:?}", files_row(&app, "gone/"));
    assert!(dot_at_edge(&app, "rmd/"), "{:?}", files_row(&app, "rmd/"));
}

#[test]
fn a_collapsed_changes_folder_wears_no_dot_and_reserves_nothing() {
    // On `Changes` every folder holds a change, so the dot would say nothing, and the row
    // keeps its full width for the name.
    let r = dotted_repo();
    r.write("src/app.rs", "x2\n"); // two changed files keep `src/` a directory row
    let mut app = app_on(&r);
    app.focus = Focus::Files;
    app.file_cursor = app.file_rows.iter().position(|r| r.dir_path() == Some("src")).unwrap();
    app.collapse_dir();
    assert!(!files_row(&app, "src/").contains('•'), "no dot on Changes");
    let wide = "n".repeat((FILES_X1 - FILES_X0) as usize - 2); // `▸ ` + name + `/` fills the row
    r.write(&format!("{wide}/a.rs"), "a\n");
    r.write(&format!("{wide}/b.rs"), "b\n");
    app.reload().unwrap();
    let row = files_row(&app, &wide[..20]);
    assert_eq!(row, format!("▾ {wide}/"), "the exact-fit name is whole on Changes");
    enter_tab(&mut app, Tab::AllFiles);
    assert!(dot_at_edge(&app, "…"), "{:?}", files_row(&app, "…"));
    assert_eq!(
        files_row(&app, "…"),
        format!("▸ …{}/ •", &wide[3..]),
        "the reserve elides two columns"
    );
}

#[test]
fn the_folder_dot_follows_the_scope() {
    let r = Repo::init();
    r.write("src/a.rs", "x\n");
    r.write("src/b.rs", "y\n");
    r.write("zz.rs", "z\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("src/a.rs", "x2\n");
    r.write("zz.rs", "z2\n");
    r.commit_all("feature work"); // committed on the branch, worktree clean
    let mut app = App::new(r.path_buf(), Scope::Uncommitted, Some("main".to_string()));
    app.reload().unwrap();
    enter_tab(&mut app, Tab::AllFiles);
    assert!(!files_row(&app, "src/").contains('•'), "nothing uncommitted under src/");

    app.set_scope(Scope::Branch).unwrap();
    common::land_world(&mut app);
    assert!(dot_at_edge(&app, "src/"), "the branch scope changed src/a.rs");

    app.set_scope(Scope::Uncommitted).unwrap();
    common::land_world(&mut app);
    assert!(!files_row(&app, "src/").contains('•'), "back to uncommitted, the dot clears");
}

#[test]
fn a_long_folder_name_leaves_room_for_the_dot() {
    let r = Repo::init();
    let dir = "a_directory_name_far_wider_than_the_files_pane_can_ever_hold_at_this_width";
    r.write(&format!("{dir}/one.rs"), "1\n");
    r.write(&format!("{dir}/two.rs"), "2\n");
    r.write("zz.rs", "z\n");
    r.commit_all("init");
    r.write(&format!("{dir}/one.rs"), "1b\n");
    r.write("zz.rs", "z2\n");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    assert!(dot_at_edge(&app, "…"), "{:?}", files_row(&app, "…"));
    let row = files_row(&app, "…");
    assert!(row.starts_with("▸ …") && row.contains("this_width/ •"), "head-elided: {row:?}");
    let collapsed_name = row.trim_end_matches(" •").to_string();

    app.focus = Focus::Files;
    app.file_cursor = app.file_rows.iter().position(|r| r.dir_path() == Some(dir)).unwrap();
    app.expand_dir();
    let row = files_row(&app, "…");
    assert_eq!(row.replacen('▾', "▸", 1), collapsed_name, "the name reads the same expanded");
}
