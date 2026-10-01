//! Tree-sitter grammars: the languages that parse, their highlight queries, and the paint that
//! turns query captures into runs.
//!
//! Each query is Helix's `highlights.scm` (MPL-2.0), then diff-reckoner's `diff-reckoner.scm`.
//! For one node the later pattern wins. A nested node wins over its parent.

use std::sync::OnceLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

/// One parseable language: the names that select it and its grammar and query.
pub(crate) struct Grammar {
    /// The `TextMate` root scope the theme sees under every capture, e.g. `source.swift`.
    pub(crate) root: &'static str,
    /// File extensions and markdown fence tags that select it.
    names: &'static [&'static str],
    language: fn() -> Language,
    /// Helix's query then ours, concatenated so ours comes last.
    queries: &'static [&'static str],
    query: OnceLock<Option<Query>>,
}

impl Grammar {
    /// The compiled query, built on first use; `None` (logged) when it fails to compile
    /// against the grammar, so the language degrades to plain text.
    pub(crate) fn query(&self) -> Option<&Query> {
        self.query
            .get_or_init(|| match Query::new(&(self.language)(), &self.queries.concat()) {
                Ok(q) => Some(q),
                Err(e) => {
                    crate::logln!("{} highlight query failed to compile: {e}", self.root);
                    None
                }
            })
            .as_ref()
    }
}

macro_rules! queries {
    ($lang:literal) => {
        &[
            include_str!(concat!("../assets/queries/", $lang, "/highlights.scm")),
            "\n",
            include_str!(concat!("../assets/queries/", $lang, "/diff-reckoner.scm")),
        ]
    };
}

static GRAMMARS: [Grammar; 2] = [
    Grammar {
        root: "source.swift",
        names: &["swift"],
        language: || tree_sitter_swift::LANGUAGE.into(),
        queries: queries!("swift"),
        query: OnceLock::new(),
    },
    Grammar {
        root: "source.kotlin",
        names: &["kt", "kts", "kotlin"],
        language: || tree_sitter_kotlin::LANGUAGE.into(),
        queries: queries!("kotlin"),
        query: OnceLock::new(),
    },
];

/// The grammar `language` names, as an extension or a fence tag (case-insensitive).
pub(crate) fn find(language: &str) -> Option<&'static Grammar> {
    GRAMMARS.iter().find(|g| g.names.iter().any(|n| n.eq_ignore_ascii_case(language)))
}

/// A run of `content` bytes `start..end` under one capture (an index into the query's
/// capture names), or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) end: usize,
    pub(crate) capture: Option<u32>,
}

/// Parse `content` and paint its captures: runs covering every byte in order. `None` when the
/// grammar or its query fails to load.
pub(crate) fn paint(grammar: &Grammar, content: &str) -> Option<Vec<Run>> {
    const NONE: u32 = u32::MAX;
    let query = grammar.query()?;
    let mut parser = Parser::new();
    parser.set_language(&(grammar.language)()).ok()?;
    let tree = parser.parse(content, None)?;

    let hidden: Vec<bool> =
        query.capture_names().iter().map(|n| n.starts_with('_') || *n == "none").collect();
    // (start, end, pattern, capture) per capture, painted outer before inner and, for one
    // node, pattern order: each paint overwrites the ones before it.
    let mut captures = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), content.as_bytes());
    while let Some(m) = matches.next() {
        for c in m.captures() {
            let r = c.node.byte_range();
            captures.push((r.start, r.end, m.pattern_index, c.index));
        }
    }
    captures.sort_unstable_by_key(|&(s, e, p, _)| (s, std::cmp::Reverse(e), p));

    let mut paint = vec![NONE; content.len()];
    for (s, e, _, c) in captures {
        let c = if hidden[c as usize] { NONE } else { c };
        paint[s..e.min(content.len())].fill(c);
    }

    let mut runs: Vec<Run> = Vec::new();
    for (i, &c) in paint.iter().enumerate() {
        let capture = (c != NONE).then_some(c);
        match runs.last_mut() {
            Some(last) if last.capture == capture => last.end = i + 1,
            _ => runs.push(Run { end: i + 1, capture }),
        }
    }
    Some(runs)
}

/// Capture name prefix to `TextMate` scope. The longest matching prefix is replaced; a capture
/// with no match is its own scope. An identity entry stops a shorter prefix from matching.
const SCOPES: &[(&str, &str)] = &[
    ("attribute", "storage.modifier.attribute"),
    ("constant.builtin.boolean", "constant.language.boolean"),
    ("constant.builtin", "constant.language"),
    ("constant", "variable.other.constant"),
    ("constant.character", "constant.character"),
    ("constant.numeric", "constant.numeric"),
    ("constructor", "entity.name.function.constructor"),
    ("function.builtin", "support.function"),
    ("function.declaration", "entity.name.function"),
    ("function.macro", "support.macro"),
    ("function", "variable.function"),
    ("keyword.function", "keyword.declaration.function"),
    ("keyword.storage.modifier", "storage.modifier"),
    ("keyword.storage.type", "storage.type"),
    ("label", "entity.name.label"),
    ("namespace", "entity.name.namespace"),
    ("operator", "keyword.operator"),
    ("punctuation.bracket", "punctuation.section"),
    ("punctuation.delimiter", "punctuation.separator"),
    ("punctuation.special", "punctuation.section.interpolation"),
    ("special", "keyword.operator"),
    ("string.special.url", "markup.underline.link"),
    ("string.special.symbol", "constant.other.symbol"),
    ("string.special", "string.other"),
    ("tag", "entity.name.tag"),
    ("type.builtin", "support.type"),
    ("type.declaration", "entity.name.type.class"),
    ("type.enum.variant", "variable.other.enummember"),
    ("type", "entity.name.type"),
    ("variable.builtin", "variable.language"),
    ("variable.declaration", "entity.name.variable"),
    ("variable.parameter", "variable.parameter"),
    ("variable", "variable.other"),
    ("variable.other", "variable.other"),
];

/// The `TextMate` scope `capture` colors as.
pub(crate) fn scope_for(capture: &str) -> String {
    let hit = SCOPES
        .iter()
        .filter(|(name, _)| {
            capture.strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
        .max_by_key(|(name, _)| name.len());
    match hit {
        Some((name, scope)) => format!("{scope}{}", &capture[name.len()..]),
        None => capture.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_compiles_against_its_grammar() {
        for g in &GRAMMARS {
            assert!(g.query().is_some(), "{} query failed to compile", g.root);
        }
    }

    #[test]
    fn captures_name_their_textmate_scope() {
        assert_eq!(scope_for("type"), "entity.name.type");
        assert_eq!(scope_for("type.builtin"), "support.type");
        assert_eq!(scope_for("typealias"), "typealias", "a prefix matches whole segments");
        assert_eq!(scope_for("keyword.control.return"), "keyword.control.return");
        assert_eq!(scope_for("constant.builtin.boolean"), "constant.language.boolean");
        assert_eq!(scope_for("function.method"), "variable.function.method");
        assert_eq!(scope_for("variable.other.member"), "variable.other.member");
        assert_eq!(scope_for("constant.numeric.integer"), "constant.numeric.integer");
    }

    #[test]
    fn the_later_pattern_wins_and_inner_nodes_win() {
        let g = find("swift").unwrap();
        let src = "let a = f(b.c)\n";
        let runs = paint(g, src).unwrap();
        let names = g.query().unwrap().capture_names();
        let at = |needle: &str| {
            let i = src.find(needle).unwrap();
            let run = runs.iter().find(|r| r.end > i).unwrap();
            run.capture.map(|c| names[c as usize])
        };
        assert_eq!(at("f("), Some("function"), "the call pattern beats (simple_identifier)");
        assert_eq!(at("c)"), Some("variable.other.member"));
        assert_eq!(at("let"), Some("keyword"));
        assert_eq!(runs.last().unwrap().end, src.len(), "runs cover every byte");
    }
}
