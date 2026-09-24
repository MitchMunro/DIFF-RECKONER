//! Formatting comments and exporting them to the clipboard or the export file.
//!
//! The export is a note to an agent: a preamble saying what the comments are and what to do with
//! them, then each file's comments as their tag lines in a fenced block of the file around them.
//! Export never consumes: a comment lasts until something removes it from its file (design doc
//! §8).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::model::Comment;
use crate::review::TAG;

/// One comment as its export block: the tag lines' location, then the tag lines with the file's
/// context either side, verbatim, fenced in the file's language.
pub fn format_comment(comment: &Comment) -> String {
    let label = if comment.start == comment.end {
        format!("Line {}:", comment.start)
    } else {
        format!("Lines {}-{}:", comment.start, comment.end)
    };
    let body = [&comment.before, &comment.lines, &comment.after]
        .into_iter()
        .flatten()
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>()
        .join("\n");
    let language = Path::new(&comment.file)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let fence = fence_for(&body);
    format!("{label}\n{fence}{language}\n{body}\n{fence}")
}

/// A backtick fence one longer than any run in `body`, so no line of the file can close it.
fn fence_for(body: &str) -> String {
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

/// What the agent is asked to do. The `[DELETED]` line shows only when a comment carries one.
fn preamble(comments: &[&Comment]) -> String {
    let mut out = match comments.len() {
        1 => format!(
            "Address the review comment below. Each comment line starts with `{TAG}`.\n\
             Once you've addressed it, delete its lines. If you won't act on it, leave it and say why."
        ),
        n => format!(
            "Address the {n} review comments below. Each comment line starts with `{TAG}`.\n\
             Once you've addressed a comment, delete its lines. If you won't act on one, leave it \
             and say why."
        ),
    };
    if comments.iter().any(|c| c.deleted.is_some()) {
        out.push_str("\n`[DELETED: (...)]` quotes the start of a removed line.");
    }
    out
}

/// Every comment: the preamble, then a `## path` section per file, its comments in line order.
pub fn format_all(comments: &[&Comment]) -> String {
    let mut sorted = comments.to_vec();
    sorted.sort_by(|a, b| a.file.cmp(&b.file).then(a.start.cmp(&b.start)));
    let mut parts = vec![preamble(&sorted)];
    for (i, c) in sorted.iter().enumerate() {
        if i == 0 || sorted[i - 1].file != c.file {
            parts.push(format!("## {}", c.file));
        }
        parts.push(format_comment(c));
    }
    parts.join("\n\n")
}

/// A destination comments can be exported to. Export succeeds or errors as a whole.
pub trait ExportTarget {
    fn export(&self, text: &str) -> Result<()>;
    fn label(&self) -> &'static str;
    /// Destination-specific confirmation shown after a successful export.
    fn success_message(&self, count: usize) -> String;
    /// Destination-specific line shown after a failed one. It is the whole status, so it is one
    /// short sentence a reviewer can read, never the underlying error. The cause goes to the log.
    fn failure_message(&self) -> String;
}

fn counted_comments(count: usize) -> String {
    let noun = if count == 1 { "comment" } else { "comments" };
    format!("{count} {noun}")
}

/// A clipboard tool and the args that make it read stdin into the system clipboard. Tried in
/// order — the first one present on `PATH` wins. macOS ships `pbcopy`; Linux needs one of these
/// installed (Wayland `wl-copy`, or X11 `xclip`/`xsel`). OSC 52 and Windows are roadmap.
const CLIPBOARD_TOOLS: &[(&str, &[&str])] = &[
    ("pbcopy", &[]),
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

/// The system clipboard, via the first available platform clipboard tool.
#[derive(Debug)]
pub struct Clipboard;

impl ExportTarget for Clipboard {
    fn label(&self) -> &'static str {
        "clipboard"
    }

    fn success_message(&self, count: usize) -> String {
        format!("copied {}", counted_comments(count))
    }

    fn failure_message(&self) -> String {
        "clipboard failed".to_string()
    }

    fn export(&self, text: &str) -> Result<()> {
        let (cmd, args) = select_tool(CLIPBOARD_TOOLS, crate::proc::on_path)
            .context("no clipboard tool found (install wl-clipboard, xclip, or xsel)")?;
        let mut child = crate::proc::command(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {cmd}"))?;
        child
            .stdin
            .as_mut()
            .with_context(|| format!("{cmd} stdin unavailable"))?
            .write_all(text.as_bytes())
            .with_context(|| format!("writing to {cmd}"))?;
        if !child.wait().with_context(|| format!("waiting for {cmd}"))?.success() {
            bail!("{cmd} exited non-zero");
        }
        Ok(())
    }
}

/// The export file, relative to the worktree root: short enough to type into an agent's prompt
/// where there is no clipboard to paste the comments themselves (design doc §8).
pub const EXPORT_FILE: &str = ".diff-reckoner/review.md";

/// The export file under one worktree, overwritten whole on every export. Its directory
/// carries a `*` `.gitignore`, so neither the file nor the ignore file is ever a change.
#[derive(Debug)]
pub struct ReviewFile {
    pub repo: PathBuf,
}

impl ExportTarget for ReviewFile {
    fn label(&self) -> &'static str {
        "file"
    }

    fn success_message(&self, count: usize) -> String {
        format!("wrote {} to {EXPORT_FILE}", counted_comments(count))
    }

    fn failure_message(&self) -> String {
        format!("could not write {EXPORT_FILE}")
    }

    fn export(&self, text: &str) -> Result<()> {
        let path = self.repo.join(EXPORT_FILE);
        let dir = path.parent().context("export path has a directory")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        // Written once and then left alone, so a reviewer's own edit to it stands.
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, "*\n")
                .with_context(|| format!("writing {}", ignore.display()))?;
        }
        std::fs::write(&path, format!("{text}\n"))
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// The first clipboard tool the `present` predicate accepts, preserving list order.
fn select_tool(
    tools: &'static [(&'static str, &'static [&'static str])],
    present: impl Fn(&str) -> bool,
) -> Option<(&'static str, &'static [&'static str])> {
    tools.iter().copied().find(|(cmd, _)| present(cmd))
}

#[cfg(test)]
mod tests {
    use super::{
        CLIPBOARD_TOOLS, Clipboard, EXPORT_FILE, ExportTarget, ReviewFile, format_all,
        format_comment, select_tool,
    };
    use crate::model::Comment;

    #[test]
    fn clipboard_tool_selection_prefers_list_order_and_can_be_empty() {
        // None present -> no tool (the caller surfaces the "install one" error).
        assert!(select_tool(CLIPBOARD_TOOLS, |_| false).is_none());
        // Only an X11 tool present -> it's chosen, with its selection args.
        assert_eq!(
            select_tool(CLIPBOARD_TOOLS, |c| c == "xclip"),
            Some(("xclip", &["-selection", "clipboard"][..]))
        );
        // When several are present, earlier in the list wins (pbcopy over xclip).
        assert_eq!(
            select_tool(CLIPBOARD_TOOLS, |c| c == "pbcopy" || c == "xclip").map(|(cmd, _)| cmd),
            Some("pbcopy")
        );
    }

    #[test]
    fn export_confirmations_name_the_actual_result_and_pluralize_comments() {
        assert_eq!(Clipboard.success_message(1), "copied 1 comment");
        assert_eq!(Clipboard.success_message(2), "copied 2 comments");
    }

    fn parsed(path: &str, content: &str) -> Vec<Comment> {
        crate::review::parse(path, content)
    }

    #[test]
    fn a_block_fences_the_tag_lines_in_their_context() {
        let src = "fn a() {\n    one();\n    // [- REVIEW -] held across an await\n    // [- REVIEW -] deadlocks on retry\n    two().await;\n}\n";
        let c = &parsed("src/a.rs", src)[0];
        assert_eq!(
            format_comment(c),
            "Lines 3-4:\n```rs\nfn a() {\n    one();\n    // [- REVIEW -] held across an await\n    // [- REVIEW -] deadlocks on retry\n    two().await;\n}\n```"
        );
    }

    #[test]
    fn context_stops_at_five_lines_and_the_files_edges() {
        let src: String = (1..=20).map(|n| format!("l{n}\n")).collect::<Vec<_>>().concat();
        let src = src.replace("l10\n", "# [- REVIEW -] here\nl10\n");
        let block = format_comment(&parsed("x.py", &src)[0]);
        assert!(block.starts_with("Line 10:\n```py\nl5\n"), "{block}");
        assert!(block.ends_with("l14\n```"), "{block}");
        let block = format_comment(&parsed("x", "# [- REVIEW -] top\nonly\n")[0]);
        assert_eq!(block, "Line 1:\n```\n# [- REVIEW -] top\nonly\n```");
    }

    #[test]
    fn a_fence_outruns_any_backticks_in_the_file() {
        let block = format_comment(&parsed("n.md", "[- REVIEW -] fix\n```sh\nls\n```\n")[0]);
        assert!(block.starts_with("Line 1:\n````md\n"), "{block}");
        assert!(block.ends_with("\n````"), "{block}");
    }

    #[test]
    fn crlf_files_export_without_carriage_returns() {
        let block = format_comment(&parsed("a.rs", "// [- REVIEW -] x\r\nfoo();\r\n")[0]);
        assert!(!block.contains('\r'), "{block:?}");
    }

    #[test]
    fn all_leads_with_the_preamble_and_groups_by_file_in_line_order() {
        let b = parsed("b.rs", "// [- REVIEW -] two\nb();\n");
        let a = parsed("a.rs", "// [- REVIEW -] earlier\na();\n\n// [- REVIEW -] later\nz();\n");
        let out = format_all(&[&b[0], &a[1], &a[0]]);
        let (preamble, rest) = out.split_once("\n\n").unwrap();
        assert_eq!(
            preamble,
            "Address the 3 review comments below. Each comment line starts with `[- REVIEW -]`.\n\
             Once you've addressed a comment, delete its lines. If you won't act on one, leave it \
             and say why."
        );
        let headings: Vec<&str> = rest.lines().filter(|l| l.starts_with("## ")).collect();
        assert_eq!(headings, ["## a.rs", "## b.rs"]);
        assert!(rest.find("earlier").unwrap() < rest.find("Line 4:").unwrap());
        assert!(!out.contains("DELETED"), "no deleted-line comment, no note about one");
    }

    #[test]
    fn one_comment_reads_in_the_singular_and_a_deleted_one_adds_its_note() {
        let c = parsed("a.rs", "// [- REVIEW -] [DELETED: (cleanup();)] still needed\nfinish();\n");
        let out = format_all(&[&c[0]]);
        assert!(out.starts_with("Address the review comment below."), "{out}");
        assert!(out.contains("Once you've addressed it, delete its lines."), "{out}");
        assert!(out.contains("\n`[DELETED: (...)]` quotes the start of a removed line.\n\n"));
        assert!(out.contains("// [- REVIEW -] [DELETED: (cleanup();)] still needed\n"), "{out}");
    }

    #[test]
    fn the_export_file_is_overwritten_and_ignores_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = ReviewFile { repo: dir.path().to_path_buf() };
        target.export("first").unwrap();
        target.export("a.rs:1\nx\nsecond").unwrap();
        let read = |p: &str| std::fs::read_to_string(dir.path().join(p)).unwrap();
        assert_eq!(read(EXPORT_FILE), "a.rs:1\nx\nsecond\n");
        assert_eq!(read(".diff-reckoner/.gitignore"), "*\n");
        assert_eq!(target.success_message(2), "wrote 2 comments to .diff-reckoner/review.md");
    }
}
