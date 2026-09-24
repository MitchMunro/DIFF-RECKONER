//! The Comments tab's place state and its pure geometry (design doc §5.3): the selected
//! comment, the card stack's scroll, and the navigator's rows.
//!
//! The tab shows the store's comments as cards in store order (file, then line), every one
//! in the repo whatever the scope. Painting lives in `ui.rs`; this module holds what the
//! reviewer's input moves and how a rescan reconciles it (Continuity).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::diff::{Row, language_of};
use crate::highlight::Highlighter;
use crate::model::{Comment, CommentStore};

/// A card's context as highlighted rows: the lines above the comment's tag lines and those
/// below, numbered as the file numbers them.
#[derive(Clone, Debug, Default)]
pub struct CardCode {
    pub before: Vec<Row>,
    pub after: Vec<Row>,
}

/// Every card's [`CardCode`], keyed by the comment's place and context, so a rescan that
/// changed nothing re-highlights nothing.
#[derive(Debug, Default)]
pub struct CardRows {
    entries: HashMap<u64, CardCode>,
}

/// Clear the cache at this many cards, so a long session cannot grow it without bound.
const CARD_ROWS_CAP: usize = 512;

impl CardRows {
    pub fn get(&mut self, c: &Comment, hl: &Highlighter) -> CardCode {
        let mut h = DefaultHasher::new();
        (&c.file, c.start, c.end, &c.before, &c.after).hash(&mut h);
        let key = h.finish();
        if let Some(code) = self.entries.get(&key) {
            return code.clone();
        }
        let code = card_code(c, hl);
        if self.entries.len() >= CARD_ROWS_CAP {
            self.entries.clear();
        }
        self.entries.insert(key, code.clone());
        code
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Highlight a comment's context as one run, the tag lines left out: they are line comments,
/// so the grammar's state across them is the state either side.
fn card_code(c: &Comment, hl: &Highlighter) -> CardCode {
    let mut content = String::new();
    for line in c.before.iter().chain(&c.after) {
        content.push_str(line);
        content.push('\n');
    }
    let mut spans = hl.highlight(&content, language_of(&c.file).as_deref()).into_iter();
    let mut rows = |first: u32, n: usize| -> Vec<Row> {
        (0..n)
            .map(|k| {
                let no = first + k as u32;
                Row::Context { old_no: no, new_no: no, spans: spans.next().unwrap_or_default() }
            })
            .collect()
    };
    let before = rows(c.start.saturating_sub(c.before.len() as u32), c.before.len());
    let after = rows(c.end + 1, c.after.len());
    CardCode { before, after }
}

/// How the next frame brings the selected card on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reveal {
    /// Scroll only as far as it takes to show the card's heading through its comment box.
    Visible,
    /// Put the card's heading at the top of the pane: a navigator pick lands where the eye
    /// goes.
    Top,
}

/// One card's measured height in display lines, and how many of them from its top a
/// [`Reveal::Visible`] keeps on screen — the heading through the comment box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CardHeight {
    pub lines: usize,
    pub keep: usize,
}

/// A navigator row: a file holding comments, or one of its comments (a store index).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NavRow {
    /// `first` is the store index of the file's first comment, the card a click scrolls to.
    File {
        path: String,
        count: usize,
        first: usize,
    },
    Comment(usize),
}

/// The tab's place state. Moved by the reviewer's input; a rescan only reconciles it
/// ([`Self::follow`]).
#[derive(Debug, Default)]
pub struct CommentsView {
    /// The selected comment, a store index.
    pub cursor: usize,
    /// The card at the top of the stack's viewport, and the display lines scrolled into it.
    /// Held as a card, not an absolute line, so a rescan that reshapes the cards above keeps
    /// the view on the same card.
    pub top: usize,
    pub top_offset: usize,
    /// Set by navigation; the frame consumes it to bring the selected card on screen. The
    /// wheel never sets it.
    pub reveal: Option<Reveal>,
    /// The navigator's top visible row.
    pub nav_scroll: usize,
    /// Set by navigation; the frame consumes it to bring the selected row on screen.
    pub reveal_nav: bool,
}

impl CommentsView {
    /// Select store index `i` and bring it on screen in both panes.
    pub fn select(&mut self, i: usize, reveal: Reveal) {
        self.cursor = i;
        self.reveal = Some(reveal);
        self.reveal_nav = true;
    }

    /// Step the selection `delta` cards over `len`, clamped at the ends.
    pub fn step(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let to = self.cursor.saturating_add_signed(delta).min(len - 1);
        self.select(to, Reveal::Visible);
    }

    /// Put the selection and the scroll back on the comments they held before the store was
    /// replaced: by identity first, then the index they had, clamped (Continuity).
    pub fn follow(
        &mut self,
        store: &CommentStore,
        selected: Option<&Comment>,
        top: Option<&Comment>,
    ) {
        if let Some(i) = selected.and_then(|c| store.position_of(c)) {
            self.cursor = i;
        }
        match top.and_then(|c| store.position_of(c)) {
            Some(i) => self.top = i,
            // The top card is gone: the one that took its place starts the view.
            None => self.top_offset = 0,
        }
        let last = store.len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
        self.top = self.top.min(last);
    }

    /// The scroll as an absolute display line into the stack.
    fn scroll(&self, heights: &[CardHeight]) -> usize {
        lines_before(heights, self.top) + self.top_offset
    }

    fn set_scroll(&mut self, heights: &[CardHeight], line: usize) {
        let mut at = 0;
        for (i, h) in heights.iter().enumerate() {
            if line < at + h.lines {
                self.top = i;
                self.top_offset = line - at;
                return;
            }
            at += h.lines;
        }
        self.top = heights.len().saturating_sub(1);
        self.top_offset = 0;
    }

    /// Scroll the stack `delta` display lines, leaving the selection alone — the wheel.
    pub fn scroll_by(&mut self, delta: isize, heights: &[CardHeight], viewport: usize) {
        let line = self.scroll(heights).saturating_add_signed(delta);
        self.set_scroll(heights, line.min(max_scroll(heights, viewport)));
    }

    /// Page the stack `delta` display lines, and select the first card whose heading is on
    /// screen after it. The selection never moves against the page's direction, and a page
    /// with no room left to scroll moves it to the end.
    pub fn page(&mut self, delta: isize, heights: &[CardHeight], viewport: usize) {
        if heights.is_empty() {
            return;
        }
        let before = self.scroll(heights);
        self.scroll_by(delta, heights, viewport);
        let last = heights.len() - 1;
        let target = if self.scroll(heights) == before {
            if delta > 0 { last } else { 0 }
        } else if self.top_offset == 0 {
            self.top
        } else {
            (self.top + 1).min(last)
        };
        let to = if delta > 0 { target.max(self.cursor) } else { target.min(self.cursor) };
        self.cursor = to;
        self.reveal = Some(Reveal::Visible);
        self.reveal_nav = true;
    }

    /// Settle the stack's scroll for this frame: honour a pending reveal, then bound the
    /// scroll so the last card's end rests at the pane's foot at the furthest.
    pub fn settle(&mut self, heights: &[CardHeight], viewport: usize) {
        let Some(last) = heights.len().checked_sub(1) else {
            *self = Self { nav_scroll: self.nav_scroll, ..Self::default() };
            return;
        };
        self.cursor = self.cursor.min(last);
        self.top = self.top.min(last);
        self.top_offset = self.top_offset.min(heights[self.top].lines.saturating_sub(1));
        let mut line = self.scroll(heights);
        if let Some(reveal) = self.reveal.take() {
            let start = lines_before(heights, self.cursor);
            match reveal {
                Reveal::Top => line = start,
                Reveal::Visible => {
                    let end = start + heights[self.cursor].keep.max(1);
                    if start < line {
                        line = start;
                    } else if end > line + viewport {
                        // A card taller than the pane shows its heading.
                        line = end.saturating_sub(viewport).min(start);
                    }
                }
            }
        }
        self.set_scroll(heights, line.min(max_scroll(heights, viewport)));
    }

    /// Settle the navigator's scroll over `rows` for a `viewport` of rows: reveal the
    /// selected row (with its file row, when that fits too) if asked, then bound.
    pub fn settle_nav(&mut self, rows: &[NavRow], viewport: usize) {
        if std::mem::take(&mut self.reveal_nav)
            && viewport > 0
            && let Some(at) = rows.iter().position(|r| *r == NavRow::Comment(self.cursor))
        {
            let file_row = rows[..at].iter().rposition(|r| matches!(r, NavRow::File { .. }));
            let from = file_row.filter(|&f| at - f < viewport).unwrap_or(at);
            if from < self.nav_scroll {
                self.nav_scroll = from;
            } else if at >= self.nav_scroll + viewport {
                self.nav_scroll = at + 1 - viewport;
            }
        }
        self.nav_scroll = self.nav_scroll.min(rows.len().saturating_sub(viewport));
    }
}

/// The display lines above card `i`.
fn lines_before(heights: &[CardHeight], i: usize) -> usize {
    heights.iter().take(i).map(|h| h.lines).sum()
}

fn max_scroll(heights: &[CardHeight], viewport: usize) -> usize {
    lines_before(heights, heights.len()).saturating_sub(viewport)
}

/// The navigator's rows: each file holding comments, its comments listed under it.
pub fn nav_rows(store: &CommentStore) -> Vec<NavRow> {
    let mut rows: Vec<NavRow> = Vec::new();
    let mut file: Option<usize> = None;
    for (i, c) in store.iter().enumerate() {
        let same =
            file.is_some_and(|f| matches!(&rows[f], NavRow::File { path, .. } if *path == c.file));
        if same {
            if let Some(NavRow::File { count, .. }) = file.map(|f| &mut rows[f]) {
                *count += 1;
            }
        } else {
            file = Some(rows.len());
            rows.push(NavRow::File { path: c.file.clone(), count: 1, first: i });
        }
        rows.push(NavRow::Comment(i));
    }
    rows
}

/// The store index of the first comment in the file after (or, `forward` false, before) the
/// one holding comment `cursor`. `None` at the ends.
pub fn file_step(store: &CommentStore, cursor: usize, forward: bool) -> Option<usize> {
    let file = &store.get(cursor)?.file;
    if forward {
        store.iter().skip(cursor).position(|c| c.file != *file).map(|k| cursor + k)
    } else {
        let prev = (0..cursor).rev().find(|&i| store.get(i).is_some_and(|c| c.file != *file))?;
        let prev_file = &store.get(prev)?.file;
        (0..=prev).rev().take_while(|&i| store.get(i).is_some_and(|c| c.file == *prev_file)).last()
    }
}

#[cfg(test)]
mod tests {
    use super::{CardHeight, CommentsView, NavRow, Reveal, file_step, nav_rows};
    use crate::model::{Comment, CommentStore};

    fn comment(file: &str, line: u32, text: &str) -> Comment {
        Comment {
            file: file.into(),
            start: line,
            end: line,
            text: text.into(),
            deleted: None,
            anchor: Some("x".into()),
            before: Vec::new(),
            after: Vec::new(),
        }
    }

    fn store(items: &[(&str, u32, &str)]) -> CommentStore {
        let mut s = CommentStore::new();
        s.replace_all(items.iter().map(|&(f, l, t)| comment(f, l, t)).collect());
        s
    }

    fn cards(n: usize, lines: usize) -> Vec<CardHeight> {
        vec![CardHeight { lines, keep: lines.min(4) }; n]
    }

    #[test]
    fn the_navigator_lists_each_file_then_its_comments() {
        let s = store(&[("a.rs", 3, "one"), ("a.rs", 9, "two"), ("b.rs", 1, "three")]);
        assert_eq!(
            nav_rows(&s),
            [
                NavRow::File { path: "a.rs".into(), count: 2, first: 0 },
                NavRow::Comment(0),
                NavRow::Comment(1),
                NavRow::File { path: "b.rs".into(), count: 1, first: 2 },
                NavRow::Comment(2),
            ]
        );
    }

    #[test]
    fn a_file_step_lands_on_the_first_comment_of_the_next_or_previous_file() {
        let s =
            store(&[("a.rs", 1, "a1"), ("a.rs", 2, "a2"), ("b.rs", 1, "b1"), ("b.rs", 5, "b2")]);
        assert_eq!(file_step(&s, 0, true), Some(2));
        assert_eq!(file_step(&s, 3, true), None);
        assert_eq!(file_step(&s, 3, false), Some(0));
        assert_eq!(file_step(&s, 1, false), None);
    }

    #[test]
    fn a_rescan_keeps_the_selection_and_the_view_on_the_same_comments() {
        let old = store(&[("a.rs", 3, "one"), ("a.rs", 9, "two"), ("b.rs", 1, "three")]);
        let mut v = CommentsView { cursor: 2, top: 1, top_offset: 2, ..Default::default() };
        let (sel, top) = (old.get(2).cloned(), old.get(1).cloned());
        // An agent adds a comment above both.
        let new = store(&[
            ("a.rs", 1, "new"),
            ("a.rs", 4, "one"),
            ("a.rs", 10, "two"),
            ("b.rs", 1, "three"),
        ]);
        v.follow(&new, sel.as_ref(), top.as_ref());
        assert_eq!((v.cursor, v.top, v.top_offset), (3, 2, 2));
    }

    #[test]
    fn a_rescan_that_drops_the_selected_comment_falls_back_to_its_place() {
        let old = store(&[("a.rs", 3, "one"), ("a.rs", 9, "two"), ("b.rs", 1, "three")]);
        let mut v = CommentsView { cursor: 2, top: 2, top_offset: 1, ..Default::default() };
        let (sel, top) = (old.get(2).cloned(), old.get(2).cloned());
        let new = store(&[("a.rs", 3, "one"), ("a.rs", 9, "two")]);
        v.follow(&new, sel.as_ref(), top.as_ref());
        assert_eq!((v.cursor, v.top, v.top_offset), (1, 1, 0));
    }

    #[test]
    fn revealing_scrolls_only_as_far_as_the_card_needs() {
        let heights = cards(5, 10);
        let mut v = CommentsView::default();
        v.select(2, Reveal::Visible);
        v.settle(&heights, 12);
        // The card's kept lines (20..24) end at the pane's foot.
        assert_eq!((v.top, v.top_offset), (1, 2));
        v.select(0, Reveal::Visible);
        v.settle(&heights, 12);
        assert_eq!((v.top, v.top_offset), (0, 0));
        v.select(3, Reveal::Top);
        v.settle(&heights, 12);
        assert_eq!((v.top, v.top_offset), (3, 0));
    }

    #[test]
    fn the_scroll_stops_with_the_last_card_at_the_foot() {
        let heights = cards(3, 10);
        let mut v = CommentsView::default();
        v.select(2, Reveal::Top);
        v.settle(&heights, 15);
        assert_eq!((v.top, v.top_offset), (1, 5));
        v.scroll_by(100, &heights, 15);
        assert_eq!((v.top, v.top_offset), (1, 5));
        v.scroll_by(-7, &heights, 15);
        assert_eq!((v.top, v.top_offset), (0, 8));
        assert_eq!(v.cursor, 2, "the wheel leaves the selection alone");
    }

    #[test]
    fn a_page_selects_the_first_whole_card_and_never_moves_backwards() {
        let heights = cards(6, 10);
        let mut v = CommentsView::default();
        v.page(15, &heights, 12);
        assert_eq!((v.top, v.top_offset, v.cursor), (1, 5, 2));
        v.page(-10, &heights, 12);
        assert_eq!(v.cursor, 1);
        // At the end, a page down selects the last card.
        v.scroll_by(1000, &heights, 12);
        v.page(15, &heights, 12);
        assert_eq!(v.cursor, 5);
    }

    #[test]
    fn the_navigator_reveals_the_selected_row_with_its_file() {
        let s =
            store(&[("a.rs", 1, "a1"), ("a.rs", 2, "a2"), ("b.rs", 1, "b1"), ("b.rs", 5, "b2")]);
        let rows = nav_rows(&s);
        let mut v = CommentsView::default();
        v.select(3, Reveal::Visible);
        v.settle_nav(&rows, 3);
        assert_eq!(v.nav_scroll, 3, "b.rs's row and both its comments");
        v.select(0, Reveal::Visible);
        v.settle_nav(&rows, 3);
        assert_eq!(v.nav_scroll, 0);
    }
}
