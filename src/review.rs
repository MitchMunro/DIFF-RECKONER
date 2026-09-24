//! Review comments written into the source file itself (design doc §3).
//!
//! A comment is a run of consecutive lines, each a line comment carrying [`TAG`]. The file is
//! the only store: [`scan`] finds every comment, and [`add`], [`rewrite`] and [`delete`] edit
//! the file on disk. Every edit re-reads the file first and refuses when it no longer holds
//! what the caller saw, so a write never lands on lines an agent has since moved.

use std::path::Path;

use anyhow::Result;

use crate::model::Comment;

/// The tag every comment line carries, greppable as a fixed string.
pub const TAG: &str = "[- REVIEW -]";

/// Lines of the file a comment carries from each side of its tag lines (§5.3).
pub const CONTEXT_LINES: usize = 5;

/// How many characters of a removed line a `[DELETED: (...)]` marker keeps.
const DELETED_LEN: usize = 16;

const DELETED_OPEN: &str = "[DELETED: (";
const DELETED_CLOSE: &str = ")]";

/// Extensions whose language has no line comment, where a written comment would break the
/// file (§3.4).
const NO_LINE_COMMENT: &[&str] =
    &["json", "jsonl", "rst", "css", "html", "htm", "xhtml", "xml", "svg", "csv", "tsv", "ipynb"];

/// Line-comment leaders by extension. Anything not listed, and a file with no extension,
/// takes `#` (§3.5). Prose files take the tag bare: they are just text, so a tag line is a
/// comment already.
const LINE_COMMENT: &[(&str, &[&str])] = &[
    ("", &["md", "markdown", "mdx", "txt", "text"]),
    (
        "//",
        &[
            "rs", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "hxx", "m", "mm", "cs", "java", "kt",
            "kts", "scala", "sc", "go", "swift", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts",
            "cts", "dart", "zig", "groovy", "gradle", "proto", "scss", "less", "jsonc", "json5",
            "fs", "fsx", "sol", "php", "v",
        ],
    ),
    ("--", &["sql", "lua", "hs", "elm", "ada", "adb", "ads", "vhd", "vhdl"]),
    (";", &["lisp", "cl", "el", "clj", "cljs", "cljc", "edn", "scm", "rkt", "asm", "s", "ini"]),
    ("%", &["tex", "sty", "cls", "erl", "hrl"]),
    ("\"", &["vim"]),
    ("'", &["vb", "vbs", "bas"]),
];

/// The line-comment leader for `path` (empty for a prose file), or `None` when its language has
/// none.
pub fn line_prefix(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if NO_LINE_COMMENT.contains(&ext.as_str()) {
        return None;
    }
    let listed = LINE_COMMENT.iter().find(|(_, exts)| exts.contains(&ext.as_str()));
    Some(listed.map_or("#", |(prefix, _)| prefix))
}

/// Why a comment write was refused. `status` is the one short line the status bar shows.
#[derive(Debug)]
pub enum WriteError {
    /// The file's language has no line comment; carries the extension.
    NoSyntax(String),
    /// The file is gone from the worktree.
    Missing,
    /// The file is not UTF-8 text, so rewriting it could corrupt it.
    NotText,
    /// The file no longer holds what the caller saw.
    Changed,
    /// The new comment would touch an existing one and merge into it.
    Merges,
    Io(std::io::Error),
}

impl WriteError {
    pub fn status(&self) -> String {
        match self {
            WriteError::NoSyntax(ext) if ext.is_empty() => "no line-comment syntax here".into(),
            WriteError::NoSyntax(ext) => format!("no line-comment syntax for .{ext}"),
            WriteError::Missing => "file is gone from the worktree".into(),
            WriteError::NotText => "not a UTF-8 text file".into(),
            WriteError::Changed => "file changed on disk; try again".into(),
            WriteError::Merges => "that line already has a comment".into(),
            WriteError::Io(_) => "could not write the file".into(),
        }
    }
}

/// Where a new comment goes and what it annotates.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Placement {
    /// The 1-based line the comment is written directly above; one past the last line
    /// writes it at the end of the file.
    pub before: u32,
    /// What the caller saw at `before`: the line's text, or `None` for the end of the file.
    pub expect: Option<String>,
    /// The removed line a comment on a deletion carries (§3.3).
    pub deleted: Option<String>,
}

/// The first [`DELETED_LEN`] characters of a removed line, its indentation dropped.
pub fn deleted_snippet(line: &str) -> String {
    line.trim_start().chars().take(DELETED_LEN).collect()
}

/// Every comment in the worktree: the tracked and untracked files `git grep` finds the tag in,
/// plus `extra`, the ignored files a write reached, which `git grep` cannot see.
pub fn scan<'a>(repo: &Path, extra: impl IntoIterator<Item = &'a String>) -> Result<Vec<Comment>> {
    let mut paths = crate::git::files_containing(repo, TAG)?;
    for path in extra {
        if !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    let mut out = Vec::new();
    for path in &paths {
        if let Ok(bytes) = std::fs::read(repo.join(path)) {
            out.extend(parse(path, &String::from_utf8_lossy(&bytes)));
        }
    }
    Ok(out)
}

/// The comments in `content`, the text of `path`. Where the language has line comments, only
/// lines led by its own leader count, so a string literal quoting the tag is not a comment.
/// Where it has none, a line the tag starts is one, as in prose.
pub fn parse(path: &str, content: &str) -> Vec<Comment> {
    let prefix = read_prefix(path);
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(first) = tag_body(lines[i], prefix) else {
            i += 1;
            continue;
        };
        let start = i;
        let mut body = vec![first];
        while let Some(next) = lines.get(i + 1).and_then(|l| tag_body(l, prefix)) {
            body.push(next);
            i += 1;
        }
        let (deleted, head) = split_deleted(body[0]);
        body[0] = head;
        out.push(Comment {
            file: path.to_string(),
            start: line_no(start),
            end: line_no(i),
            text: body.join("\n"),
            deleted,
            anchor: lines.get(i + 1).map(|l| strip_eol(l).to_string()),
            before: context_above(&lines, start),
            after: context_below(&lines, i),
        });
        i += 1;
    }
    out
}

/// Up to [`CONTEXT_LINES`] lines above index `start`, verbatim.
fn context_above(lines: &[&str], start: usize) -> Vec<String> {
    lines[start.saturating_sub(CONTEXT_LINES)..start].iter().map(|l| strip_eol(l).into()).collect()
}

/// Up to [`CONTEXT_LINES`] lines below index `end`, verbatim.
fn context_below(lines: &[&str], end: usize) -> Vec<String> {
    let to = (end + 1 + CONTEXT_LINES).min(lines.len());
    lines[(end + 1).min(to)..to].iter().map(|l| strip_eol(l).into()).collect()
}

/// Write a new comment into `path` at `at`.
pub fn add(repo: &Path, path: &str, at: &Placement, text: &str) -> Result<(), WriteError> {
    let prefix = prefix_for(path)?;
    let content = read_text(repo, path)?;
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let seen = match at.expect.as_deref() {
        Some(expect) => lines.get(index(at.before)).is_some_and(|l| strip_eol(l) == expect),
        None => at.before as usize == lines.len() + 1,
    };
    if !seen {
        return Err(WriteError::Changed);
    }
    // Tag lines touching the new ones would read back as one comment.
    let at_index = index(at.before);
    let touches = |i: Option<usize>| {
        i.and_then(|i| lines.get(i)).is_some_and(|l| tag_body(l, prefix).is_some())
    };
    if touches(at_index.checked_sub(1)) || touches(Some(at_index)) {
        return Err(WriteError::Merges);
    }
    let updated = insert(&content, prefix, at.before, text, at.deleted.as_deref());
    write_text(repo, path, &updated)
}

/// Replace comment `c`'s text in place, keeping its `[DELETED: (...)]` marker.
pub fn rewrite(repo: &Path, c: &Comment, text: &str) -> Result<(), WriteError> {
    let prefix = read_prefix(&c.file);
    let content = read_text(repo, &c.file)?;
    let c = locate(&content, c)?;
    let removed = remove(&content, c.start, c.end);
    let updated = insert(&removed, prefix, c.start, text, c.deleted.as_deref());
    write_text(repo, &c.file, &updated)
}

/// Remove comment `c`'s lines, restoring the file as it was before the comment.
pub fn delete(repo: &Path, c: &Comment) -> Result<(), WriteError> {
    let content = read_text(repo, &c.file)?;
    let c = locate(&content, c)?;
    write_text(repo, &c.file, &remove(&content, c.start, c.end))
}

/// The comments `path` holds on disk now: a write's own re-read.
pub fn reparse(repo: &Path, path: &str) -> Vec<Comment> {
    read_text(repo, path).map(|content| parse(path, &content)).unwrap_or_default()
}

/// `content` with `text` written as tag lines directly above line `before`, indented like
/// that line. Blank text lines keep their tag, so the comment stays one run.
fn insert(content: &str, prefix: &str, before: u32, text: &str, deleted: Option<&str>) -> String {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let eol = if content.contains("\r\n") { "\r\n" } else { "\n" };
    let at = index(before).min(lines.len());
    let indent: String = lines
        .get(at)
        .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
        .unwrap_or_default();
    let tagged: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let line = line.trim_end();
            let body = match deleted.filter(|_| i == 0) {
                Some(d) => format!("{DELETED_OPEN}{d}{DELETED_CLOSE} {line}"),
                None => line.to_string(),
            };
            let body = body.trim_end();
            let lead = if prefix.is_empty() { String::new() } else { format!("{prefix} ") };
            if body.is_empty() {
                format!("{indent}{lead}{TAG}")
            } else {
                format!("{indent}{lead}{TAG} {body}")
            }
        })
        .collect();
    let mut out = String::with_capacity(content.len() + tagged.len() * 32);
    for line in &lines[..at] {
        out.push_str(line);
    }
    if at == lines.len() {
        // The end of the file: a last line with no newline keeps none, so removing the
        // comment again restores the file byte for byte (`remove`).
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(eol);
            out.push_str(&tagged.join(eol));
        } else {
            for line in &tagged {
                out.push_str(line);
                out.push_str(eol);
            }
        }
        return out;
    }
    for line in &tagged {
        out.push_str(line);
        out.push_str(eol);
    }
    for line in &lines[at..] {
        out.push_str(line);
    }
    out
}

/// `content` without lines `start..=end`. Removing the file's unterminated last line takes the
/// newline before it too, undoing `insert` at the end of the file.
fn remove(content: &str, start: u32, end: u32) -> String {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let (lo, hi) = (index(start), (end as usize).min(lines.len()));
    let mut kept: Vec<&str> = lines[..lo].to_vec();
    let unterminated_tail = hi == lines.len() && lines.last().is_some_and(|l| !l.ends_with('\n'));
    if unterminated_tail && let Some(last) = kept.pop() {
        kept.push(strip_eol(last));
    }
    kept.extend_from_slice(&lines[hi..]);
    kept.concat()
}

/// The text after the tag when `line` is a tag line led by `prefix`.
fn tag_body<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = strip_eol(line).trim_start().strip_prefix(prefix)?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest).strip_prefix(TAG)?;
    Some(rest.strip_prefix(' ').unwrap_or(rest).trim_end())
}

/// A first tag line's `[DELETED: (...)]` marker and the text after it. The snippet may itself
/// hold `)]`, so a close exactly [`DELETED_LEN`] characters in wins over an earlier one.
fn split_deleted(body: &str) -> (Option<String>, &str) {
    let Some(rest) = body.strip_prefix(DELETED_OPEN) else { return (None, body) };
    let closes: Vec<usize> = rest.match_indices(DELETED_CLOSE).map(|(i, _)| i).collect();
    let Some(&close) =
        closes.iter().find(|&&i| rest[..i].chars().count() == DELETED_LEN).or(closes.first())
    else {
        return (None, body);
    };
    let after = &rest[close + DELETED_CLOSE.len()..];
    (Some(rest[..close].to_string()), after.strip_prefix(' ').unwrap_or(after))
}

/// Where `c` sits in `content` now: exactly where it was parsed, else the one comment with its
/// identity (`Comment::same_identity`), since an edit above may have moved it.
fn locate(content: &str, c: &Comment) -> Result<Comment, WriteError> {
    let on_disk = parse(&c.file, content);
    if let Some(d) = on_disk.iter().find(|d| *d == c) {
        return Ok(d.clone());
    }
    let mut same = on_disk.into_iter().filter(|d| d.same_identity(c));
    match (same.next(), same.next()) {
        (Some(d), None) => Ok(d),
        _ => Err(WriteError::Changed),
    }
}

fn prefix_for(path: &str) -> Result<&'static str, WriteError> {
    line_prefix(path).ok_or_else(|| {
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().into_owned());
        WriteError::NoSyntax(ext.unwrap_or_default())
    })
}

/// The leader a tag line in `path` is read with: its line comment, else none, so in a file with
/// no line-comment syntax a line the tag starts is a comment. A new comment is still refused
/// there ([`prefix_for`]), but one already in the file can be rewritten and removed.
fn read_prefix(path: &str) -> &'static str {
    line_prefix(path).unwrap_or("")
}

fn read_text(repo: &Path, path: &str) -> Result<String, WriteError> {
    let bytes = std::fs::read(repo.join(path)).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => WriteError::Missing,
        _ => WriteError::Io(e),
    })?;
    String::from_utf8(bytes).map_err(|_| WriteError::NotText)
}

fn write_text(repo: &Path, path: &str, content: &str) -> Result<(), WriteError> {
    std::fs::write(repo.join(path), content).map_err(WriteError::Io)
}

fn strip_eol(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

/// A 1-based line number as a slice index.
fn index(line: u32) -> usize {
    (line as usize).saturating_sub(1)
}

fn line_no(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{TAG, deleted_snippet, insert, line_prefix, parse, remove, split_deleted};

    fn tagged(prefix: &str, text: &str) -> String {
        format!("{prefix} {TAG} {text}")
    }

    #[test]
    fn prefix_follows_the_extension_with_hash_as_the_fallback() {
        assert_eq!(line_prefix("src/a.rs"), Some("//"));
        assert_eq!(line_prefix("q.SQL"), Some("--"));
        assert_eq!(line_prefix("a.py"), Some("#"));
        assert_eq!(line_prefix("Makefile"), Some("#"));
        assert_eq!(line_prefix("a.unknown"), Some("#"));
        assert_eq!(line_prefix("README.md"), Some(""));
        assert_eq!(line_prefix("notes.txt"), Some(""));
        assert_eq!(line_prefix("package.json"), None);
    }

    #[test]
    fn consecutive_tag_lines_parse_as_one_comment_with_its_anchor() {
        let content = format!(
            "fn a() {{}}\n    {}\n    {}\n    let x = lock();\n{}\n",
            tagged("//", "held across an await"),
            tagged("//", "which deadlocks"),
            tagged("//", "trailing"),
        );
        let found = parse("a.rs", &content);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].start, found[0].end), (2, 3));
        assert_eq!(found[0].text, "held across an await\nwhich deadlocks");
        assert_eq!(found[0].anchor.as_deref(), Some("    let x = lock();"));
        assert_eq!((found[1].start, found[1].anchor.as_deref()), (5, None));
    }

    #[test]
    fn a_comment_carries_up_to_five_lines_of_context_each_side() {
        let above = (1..=7).map(|n| format!("a{n}\n")).collect::<Vec<_>>().concat();
        let content = format!("{above}{}\nb1\nb2\n", tagged("#", "note"));
        let found = parse("x.py", &content);
        assert_eq!(found[0].before, ["a3", "a4", "a5", "a6", "a7"]);
        // Short of five below: the file's end cuts the window.
        assert_eq!(found[0].after, ["b1", "b2"]);
        let top = parse("x.py", &format!("{}\nb1\n", tagged("#", "first line")));
        assert!(top[0].before.is_empty());
    }

    #[test]
    fn a_file_without_line_comments_takes_a_line_the_tag_starts() {
        let content = format!("{{\n{TAG} check this key\n  \"k\": \"{TAG}\"\n}}\n");
        let found = parse("a.json", &content);
        assert_eq!(found.len(), 1, "the tag inside a value is not a comment");
        assert_eq!((found[0].start, found[0].text.as_str()), (2, "check this key"));
        assert!(parse("a.html", &format!("<!-- {TAG} fix -->\n")).is_empty());
    }

    #[test]
    fn only_the_files_own_leader_makes_a_tag_line() {
        let content =
            format!("let t = \"{TAG}\";\n# {TAG} wrong leader\n\"{TAG} quoted\"\n/// {TAG} doc\n");
        assert!(parse("a.rs", &content).is_empty());
        assert!(parse("a.json", &tagged("//", "x")).is_empty());
    }

    #[test]
    fn insert_then_remove_restores_the_file_exactly() {
        for content in ["a\n  b\nc\n", "a\r\nb\r\n", "a\nb", "", "only"] {
            let lines = content.split_inclusive('\n').count() as u32;
            for before in 1..=lines + 1 {
                let written = insert(content, "#", before, "one\n\ntwo", None);
                let found = parse("x.py", &written);
                assert_eq!(found.len(), 1, "{content:?} @ {before}: {written:?}");
                assert_eq!(found[0].text, "one\n\ntwo");
                assert_eq!(remove(&written, found[0].start, found[0].end), content);
            }
        }
    }

    #[test]
    fn a_prose_file_takes_the_tag_bare() {
        let written = insert("# Title\n\nSome prose.\n", "", 3, "reword this", None);
        assert_eq!(written, format!("# Title\n\n{TAG} reword this\nSome prose.\n"));
        let found = parse("README.md", &written);
        assert_eq!((found[0].start, found[0].text.as_str()), (3, "reword this"));
        // A code fence quoting another language's tag line is not a comment here.
        assert!(parse("README.md", &format!("```rust\n// {TAG} quoted\n```\n")).is_empty());
    }

    #[test]
    fn insert_matches_the_anchor_indent_and_line_ending() {
        let written = insert("fn a() {\r\n\tx();\r\n}\r\n", "//", 2, "why", None);
        assert_eq!(written, format!("fn a() {{\r\n\t// {TAG} why\r\n\tx();\r\n}}\r\n"));
    }

    #[test]
    fn deleted_marker_round_trips_and_prefers_a_full_length_snippet() {
        let snippet = deleted_snippet("    let ok = validate_token(t)?;");
        assert_eq!(snippet, "let ok = validat");
        let written = insert("a\n", "//", 1, "load-bearing\nmore", Some(&snippet));
        let c = &parse("a.rs", &written)[0];
        assert_eq!(c.deleted.as_deref(), Some("let ok = validat"));
        assert_eq!(c.text, "load-bearing\nmore");
        // A snippet holding the close keeps it when the full-length close follows.
        let (d, rest) = split_deleted("[DELETED: (x[f(a)] + g(b) -)] note");
        assert_eq!((d.as_deref(), rest), (Some("x[f(a)] + g(b) -"), "note"));
        let (d, rest) = split_deleted("[DELETED: (x)] y)] z");
        assert_eq!((d.as_deref(), rest), (Some("x"), "y)] z"));
    }
}
