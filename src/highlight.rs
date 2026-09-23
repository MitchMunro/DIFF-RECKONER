//! Syntax highlighting via `syntect`, themed by the active theme's paired syntax theme.
//!
//! The highlighter is rebuilt when the theme
//! changes and produces per-line foreground spans; the background is the palette's `base`,
//! painted by the renderer.

use std::fmt;
use std::io::Cursor;

use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SyntectColor, StyleModifier, Theme, ThemeItem, ThemeSet, ThemeSettings,
};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use std::sync::OnceLock;

use ratatui::style::Color;

use crate::diff::Span;
use crate::theme::{Palette, SyntaxChoice};

/// The default text color when a theme carries none, or its syntax theme fails to load.
const DEFAULT_FG: Color = Color::Rgb(0xcd, 0xd6, 0xf4);

/// The broad bat/two-face syntax set, built once per process (it is expensive to
/// deserialize) and shared across every `Highlighter`.
fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// The two-face embedded theme set, deserialized once and shared — like [`syntaxes`], so a
/// theme switch clones one theme out of the cached set instead of rebuilding the whole dump.
fn embedded_themes() -> &'static two_face::theme::EmbeddedLazyThemeSet {
    static THEMES: OnceLock<two_face::theme::EmbeddedLazyThemeSet> = OnceLock::new();
    THEMES.get_or_init(two_face::theme::extra)
}

/// Holds the active syntax theme (absent when it failed to load); highlights file content
/// into spans against the shared syntax set.
pub struct Highlighter {
    theme: Option<Theme>,
    default_fg: Color,
}

impl fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Highlighter").finish_non_exhaustive()
    }
}

impl Highlighter {
    /// Build from a theme's paired syntax source: a bundled `.tmTheme` (parsed from vendored
    /// bytes), or a theme from the `two-face` embedded set. A bundled theme that fails to
    /// parse leaves the highlighter theme-less, so highlighting degrades to plain spans
    /// rather than crashing. Most files color out of the box via the
    /// broad two-face syntax set.
    pub fn new(syntax: SyntaxChoice) -> Self {
        let theme = match syntax {
            SyntaxChoice::Bundled(bytes) => {
                match ThemeSet::load_from_reader(&mut Cursor::new(bytes)) {
                    Ok(theme) => Some(theme),
                    Err(e) => {
                        crate::logln!("bundled syntax theme failed to parse: {e}");
                        None
                    }
                }
            }
            SyntaxChoice::Embedded(name) => Some(embedded_themes().get(name).clone()),
            SyntaxChoice::Derived(palette) => Some(derived_theme(&palette)),
        };
        let default_fg =
            theme.as_ref().and_then(|t| t.settings.foreground).map_or(DEFAULT_FG, from_syntect);
        Self { theme, default_fg }
    }

    /// Highlight `content` line by line. Each inner `Vec` is one line's spans. With no
    /// known `language` — or no loaded theme — every line is a single plain span in the
    /// default color. `language` matches as an extension first (paths), then as a token
    /// name (markdown fence tags like `rust` or `python`).
    pub fn highlight(&self, content: &str, language: Option<&str>) -> Vec<Vec<Span>> {
        let syntaxes = syntaxes();
        let syntax = language.and_then(|lang| {
            syntaxes.find_syntax_by_extension(lang).or_else(|| syntaxes.find_syntax_by_token(lang))
        });
        let (Some(syntax), Some(theme)) = (syntax, self.theme.as_ref()) else {
            return content
                .lines()
                .map(|l| vec![Span { text: l.to_string(), color: self.default_fg }])
                .collect();
        };
        let mut h = HighlightLines::new(syntax, theme);
        let mut out = Vec::new();
        for line in LinesWithEndings::from(content) {
            let spans = match h.highlight_line(line, syntaxes) {
                Ok(regions) => regions
                    .into_iter()
                    .map(|(style, text)| Span {
                        text: text.trim_end_matches('\n').to_string(),
                        color: from_syntect(style.foreground),
                    })
                    .collect(),
                // A grammar error degrades to plain text rather than blocking the diff.
                Err(_) => vec![Span {
                    text: line.trim_end_matches('\n').to_string(),
                    color: self.default_fg,
                }],
            };
            out.push(spans);
        }
        out
    }
}

/// A syntax theme built from a palette, for a theme with none of its own: each token role takes
/// one accent, lifted to stay legible on the palette's base. Unlisted scopes (operators,
/// punctuation, plain identifiers) keep the palette's text color.
fn derived_theme(p: &Palette) -> Theme {
    let roles = [
        ("comment, punctuation.definition.comment, markup.quote", p.dim1),
        ("keyword, storage, keyword.control", p.purple),
        ("string, markup.raw, markup.inline.raw, markup.inserted", p.green),
        ("constant.numeric, constant.language, constant.character, constant.other", p.orange),
        ("entity.name.function, support.function, markup.heading, entity.name.section", p.blue),
        ("markup.underline.link, string.other.link", p.blue),
        (
            "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, \
             support.type, support.class, entity.other.attribute-name, markup.changed",
            p.yellow,
        ),
        ("entity.name.tag, variable.language, support.constant, markup.deleted, invalid", p.red),
    ];
    let scopes = roles
        .into_iter()
        .map(|(selector, color)| ThemeItem {
            scope: selector.parse().expect("a static scope selector parses"),
            style: StyleModifier {
                foreground: Some(syntect_color(p.legible(color))),
                background: None,
                font_style: None,
            },
        })
        .collect();
    Theme {
        settings: ThemeSettings {
            foreground: Some(syntect_color(p.text)),
            ..ThemeSettings::default()
        },
        scopes,
        ..Theme::default()
    }
}

/// A palette color as syntect's. syntect has only RGBA, so the `terminal` theme's colors ride
/// in the alpha channel as bat encodes them: alpha 0 carries an ANSI index in `r`, alpha 1 the
/// terminal default.
fn syntect_color(color: Color) -> SyntectColor {
    let ansi = |r| SyntectColor { r, g: 0, b: 0, a: 0 };
    match color {
        Color::Rgb(r, g, b) => SyntectColor { r, g, b, a: 0xff },
        Color::Reset => SyntectColor { r: 0, g: 0, b: 0, a: 1 },
        Color::Indexed(i) => ansi(i),
        Color::Black => ansi(0),
        Color::Red => ansi(1),
        Color::Green => ansi(2),
        Color::Yellow => ansi(3),
        Color::Blue => ansi(4),
        Color::Magenta => ansi(5),
        Color::Cyan => ansi(6),
        Color::Gray => ansi(7),
        Color::DarkGray => ansi(8),
        Color::LightRed => ansi(9),
        Color::LightGreen => ansi(10),
        Color::LightYellow => ansi(11),
        Color::LightBlue => ansi(12),
        Color::LightMagenta => ansi(13),
        Color::LightCyan => ansi(14),
        Color::White => ansi(15),
    }
}

/// A syntect color as the renderer's, decoding [`syntect_color`]'s alpha convention.
fn from_syntect(c: SyntectColor) -> Color {
    match c.a {
        0 => Color::Indexed(c.r),
        1 => Color::Reset,
        _ => Color::Rgb(c.r, c.g, c.b),
    }
}

#[cfg(test)]
mod tests {
    use super::Highlighter;
    use crate::theme;

    /// The bundled Catppuccin Mocha syntax, the default theme's pairing.
    fn mocha() -> super::SyntaxChoice {
        theme::resolve(Some("catppuccin")).syntax
    }

    #[test]
    fn highlights_rust_into_colored_spans() {
        let h = Highlighter::new(mocha());
        let lines = h.highlight("let x = 1;\n", Some("rs"));
        assert_eq!(lines.len(), 1);
        let spans = &lines[0];
        assert!(spans.len() > 1, "rust tokenizes into several spans");
        assert_eq!(spans.iter().map(|s| s.text.as_str()).collect::<String>(), "let x = 1;");
        // The Catppuccin keyword color (purple) differs from the default text color.
        assert!(spans.iter().any(|s| s.text == "let" && s.color != super::DEFAULT_FG));
    }

    #[test]
    fn unknown_language_is_one_plain_span_per_line() {
        let h = Highlighter::new(mocha());
        let lines = h.highlight("alpha\nbeta\n", None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], vec![super::Span { text: "alpha".into(), color: super::DEFAULT_FG }]);
    }

    #[test]
    fn bundled_syntax_themes_all_parse() {
        // Each bundled `.tmTheme` must load, or highlighting silently degrades to plain spans
        // A loaded theme tokenizes rust into more than one span; a failed
        // load would yield a single plain span — so this guards the parse path for every
        // bundled theme, the only `SyntaxChoice` that can fail.
        for name in ["catppuccin", "tokyo-night", "tokyo-night-day", "rose-pine", "rose-pine-dawn"]
        {
            let h = Highlighter::new(theme::resolve(Some(name)).syntax);
            let spans = h.highlight("let x = 1;\n", Some("rs"));
            assert!(spans[0].len() > 1, "{name}: bundled syntax theme failed to load");
        }
    }

    #[test]
    fn derived_syntax_colors_tokens_from_the_palette() {
        let t = theme::resolve(Some("xcode-light"));
        assert!(matches!(t.syntax, theme::SyntaxChoice::Derived(_)));
        let spans = Highlighter::new(t.syntax).highlight("let x = \"s\";\n", Some("rs"));
        let color_of = |text: &str| spans[0].iter().find(|s| s.text.trim() == text).unwrap().color;
        assert_eq!(color_of("let"), t.palette.legible(t.palette.purple));
        assert_ne!(color_of("let"), color_of("x"), "keywords and identifiers differ");
    }
}
