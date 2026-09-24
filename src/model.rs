//! Review model: scopes, changed files, and comments.
//!
//! Comments live in the source files as tag lines (`review.rs`); the store here only holds
//! the last scan of them.

/// Which set of changes the Changes view shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Uncommitted,
    Branch,
    /// A picked run of commits, diffed `A^` against `B`.
    Commits,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Uncommitted => "uncommitted",
            Scope::Branch => "branch",
            Scope::Commits => "commits",
        }
    }

    /// The scope's name in the specs and in config values (`default_scope`): kebab-case,
    /// unlike the header chip's spaced `label`.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Uncommitted => "uncommitted",
            Scope::Branch => "branch",
            Scope::Commits => "commits",
        }
    }

    /// Cycle to the next scope, for the header chip click: uncommitted → branch → commits.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Scope::Uncommitted => Scope::Branch,
            Scope::Branch => Scope::Commits,
            Scope::Commits => Scope::Uncommitted,
        }
    }
}

/// The `commits` scope's pick: a contiguous run from `oldest` to `newest`, both full commit
/// ids, equal for a run of one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitPick {
    pub oldest: String,
    pub newest: String,
}

impl CommitPick {
    pub fn single(sha: &str) -> Self {
        Self { oldest: sha.to_string(), newest: sha.to_string() }
    }

    pub fn is_single(&self) -> bool {
        self.oldest == self.newest
    }
}

/// How a file changed within a scope.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

impl ChangeKind {
    pub fn marker(self) -> char {
        match self {
            ChangeKind::Added => 'A',
            ChangeKind::Modified => 'M',
            ChangeKind::Deleted => 'D',
            ChangeKind::Renamed => 'R',
            ChangeKind::Untracked => '?',
        }
    }
}

/// A row in the Changes list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
    /// The old path of a renamed file; `None` for every other kind. Its old content lives
    /// at this path, so a rename diffs real content instead of reading as all-insertion.
    pub previous_path: Option<String>,
}

/// Which side of the diff a line lives on. Only the PR-snippet leftover (`snippet.rs`) reads
/// it: a comment in the file always sits on the new side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    New,
    Old,
}

/// A review comment: one run of consecutive tag lines in a worktree file (design doc §3.1).
/// The file is the only store, so a `Comment` is only ever a parse of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comment {
    pub file: String,
    /// The tag lines' 1-based line span in the file.
    pub start: u32,
    pub end: u32,
    /// The text after the tag, one line per tag line.
    pub text: String,
    /// The start of the removed line a `[DELETED: (...)]` comment was left on (§3.3).
    pub deleted: Option<String>,
    /// The line the comment annotates, the first below its tag lines, verbatim; `None` when
    /// the comment ends the file.
    pub anchor: Option<String>,
    /// Up to [`crate::review::CONTEXT_LINES`] lines above the tag lines, and below them,
    /// verbatim: the Comments tab's context window (§5.3), read by the scan that found the
    /// comment. Not identity: an edit nearby changes it without making another comment.
    pub before: Vec<String>,
    pub after: Vec<String>,
    /// The tag lines themselves, verbatim: what the export shows an agent to delete.
    pub lines: Vec<String>,
}

impl Comment {
    /// The `path:start-end` (or `path:line`) span of the tag lines.
    pub fn location(&self) -> String {
        if self.start == self.end {
            format!("{}:{}", self.file, self.start)
        } else {
            format!("{}:{}-{}", self.file, self.start, self.end)
        }
    }

    /// The text as the list and the export show it: the `[DELETED: (...)]` marker, when
    /// there is one, leads the first line.
    pub fn display_text(&self) -> String {
        match &self.deleted {
            Some(d) => format!("[DELETED: ({d})] {}", self.text),
            None => self.text.clone(),
        }
    }

    /// Whether `other` is the same comment for reconciling place state: same file, text,
    /// and annotated line, wherever a refresh moved it (Continuity).
    pub fn same_identity(&self, other: &Comment) -> bool {
        self.file == other.file
            && self.text == other.text
            && self.deleted == other.deleted
            && self.anchor == other.anchor
    }
}

/// The comments the last scan found, sorted by file then line. Derived state: only a scan
/// (`replace_all`) or a write that re-read its own file (`replace_file`) changes it.
#[derive(Default, Debug)]
pub struct CommentStore {
    items: Vec<Comment>,
}

impl CommentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Comment> {
        self.items.iter()
    }

    pub fn get(&self, index: usize) -> Option<&Comment> {
        self.items.get(index)
    }

    /// Adopt a whole scan.
    pub fn replace_all(&mut self, mut comments: Vec<Comment>) {
        sort(&mut comments);
        self.items = comments;
    }

    /// Adopt one file's fresh parse, so a write shows before the next scan lands.
    pub fn replace_file(&mut self, file: &str, comments: Vec<Comment>) {
        self.items.retain(|c| c.file != file);
        self.items.extend(comments);
        sort(&mut self.items);
    }

    /// The index of the comment `c` names by identity (`Comment::same_identity`).
    pub fn position_of(&self, c: &Comment) -> Option<usize> {
        self.items.iter().position(|it| it.same_identity(c))
    }
}

fn sort(comments: &mut [Comment]) {
    comments.sort_by(|a, b| a.file.cmp(&b.file).then(a.start.cmp(&b.start)));
}

#[cfg(test)]
mod tests {
    use super::{Comment, CommentStore, Scope};

    fn comment(file: &str, start: u32, end: u32, text: &str) -> Comment {
        Comment {
            file: file.into(),
            start,
            end,
            text: text.into(),
            deleted: None,
            anchor: Some("x".into()),
            before: Vec::new(),
            after: Vec::new(),
            lines: Vec::new(),
        }
    }

    #[test]
    fn scope_cycles_and_labels() {
        // The chip click cycles through all three scopes and wraps.
        assert_eq!(Scope::Uncommitted.cycle(), Scope::Branch);
        assert_eq!(Scope::Branch.cycle(), Scope::Commits);
        assert_eq!(Scope::Commits.cycle(), Scope::Uncommitted);
        assert_eq!(Scope::Uncommitted.label(), "uncommitted");
        assert_eq!(Scope::Commits.label(), "commits");
        assert_eq!(Scope::Commits.name(), "commits");
    }

    #[test]
    fn a_pick_of_one_is_single() {
        let pick = super::CommitPick::single("abc");
        assert!(pick.is_single());
        assert!(!super::CommitPick { oldest: "a".into(), newest: "b".into() }.is_single());
    }

    #[test]
    fn location_formats_range_and_single_line() {
        let mut c = comment("a.rs", 40, 52, "x");
        assert_eq!(c.location(), "a.rs:40-52");
        c.end = 40;
        assert_eq!(c.location(), "a.rs:40");
    }

    #[test]
    fn display_text_leads_with_the_deleted_marker() {
        let mut c = comment("a.rs", 1, 1, "load-bearing");
        assert_eq!(c.display_text(), "load-bearing");
        c.deleted = Some("let ok = validat".into());
        assert_eq!(c.display_text(), "[DELETED: (let ok = validat)] load-bearing");
    }

    #[test]
    fn replace_file_swaps_one_file_and_keeps_the_sort() {
        let mut s = CommentStore::new();
        s.replace_all(vec![comment("b.rs", 5, 5, "b"), comment("a.rs", 9, 9, "a9")]);
        assert_eq!(s.get(0).unwrap().file, "a.rs");
        s.replace_file("a.rs", vec![comment("a.rs", 2, 2, "a2"), comment("a.rs", 7, 7, "a7")]);
        let texts: Vec<&str> = s.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["a2", "a7", "b"]);
        s.replace_file("a.rs", Vec::new());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn position_of_matches_by_identity_not_line() {
        let mut s = CommentStore::new();
        s.replace_all(vec![comment("a.rs", 3, 3, "one"), comment("a.rs", 8, 8, "two")]);
        // Lines shifted by an edit above: still the same comment.
        assert_eq!(s.position_of(&comment("a.rs", 12, 12, "two")), Some(1));
        assert_eq!(s.position_of(&comment("a.rs", 8, 8, "gone")), None);
    }
}
