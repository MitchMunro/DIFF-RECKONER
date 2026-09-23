//! Formatting comments and exporting them to the clipboard.
//!
//! A comment becomes a block of `location`, the line it annotates, then the text. Export
//! never consumes: a comment lasts until something removes it from its file (design doc §8).

use std::io::Write;
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::model::Comment;

/// One comment as its export block: location, the annotated line, then text. A comment that
/// ends its file annotates no line, so its block has no snippet.
pub fn format_comment(comment: &Comment) -> String {
    let text = normalize_text(&comment.display_text());
    match &comment.anchor {
        Some(line) => format!("{}\n{line}\n{text}", comment.location()),
        None => format!("{}\n{text}", comment.location()),
    }
}

/// Comment text for export: drop `\r`, trim trailing space per line, and drop blank
/// lines so a multi-line comment can never introduce the blank-line block separator.
fn normalize_text(text: &str) -> String {
    text.replace('\r', "")
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Many comments, sorted by file then start line, one blank line between blocks.
pub fn format_all(comments: &[&Comment]) -> String {
    let mut sorted = comments.to_vec();
    sorted.sort_by(|a, b| a.file.cmp(&b.file).then(a.start.cmp(&b.start)));
    sorted.iter().map(|c| format_comment(c)).collect::<Vec<_>>().join("\n\n")
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
        CLIPBOARD_TOOLS, Clipboard, ExportTarget, format_all, format_comment, select_tool,
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

    fn comment(file: &str, start: u32, end: u32, anchor: Option<&str>, text: &str) -> Comment {
        Comment {
            file: file.into(),
            start,
            end,
            text: text.into(),
            deleted: None,
            anchor: anchor.map(Into::into),
        }
    }

    #[test]
    fn block_is_location_annotated_line_text() {
        let c = comment(
            "extruct/core/llm_registry.py",
            40,
            41,
            Some("from .x import y"),
            "this import path\nlooks wrong",
        );
        assert_eq!(
            format_comment(&c),
            "extruct/core/llm_registry.py:40-41\nfrom .x import y\nthis import path\nlooks wrong"
        );
    }

    #[test]
    fn a_deleted_line_comment_leads_with_its_marker() {
        let mut c = comment("a.rs", 38, 38, Some("    finish();"), "still needed");
        c.deleted = Some("cleanup();".into());
        assert_eq!(
            format_comment(&c),
            "a.rs:38\n    finish();\n[DELETED: (cleanup();)] still needed"
        );
    }

    #[test]
    fn a_comment_ending_the_file_has_no_snippet() {
        let c = comment("a.rs", 9, 9, None, "trailing");
        assert_eq!(format_comment(&c), "a.rs:9\ntrailing");
    }

    #[test]
    fn multiline_text_keeps_breaks_but_drops_blank_lines() {
        let c = comment("a.rs", 1, 1, Some("x"), "first line\n\n  \nsecond line\n");
        assert_eq!(format_comment(&c), "a.rs:1\nx\nfirst line\nsecond line");
    }

    #[test]
    fn all_sorts_by_file_then_start_with_blank_separator() {
        let b = comment("b.rs", 5, 5, Some("x"), "two");
        let a2 = comment("a.rs", 20, 20, Some("y"), "later");
        let a1 = comment("a.rs", 3, 3, Some("z"), "earlier");
        let out = format_all(&[&b, &a2, &a1]);
        assert_eq!(out, "a.rs:3\nz\nearlier\n\na.rs:20\ny\nlater\n\nb.rs:5\nx\ntwo");
    }
}
