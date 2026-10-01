//! The diff-only themes: real syntax themes the diff pane can wear over the main theme.
//!
//! Each is a vendored `.tmTheme` (`assets/diff-themes/`, sources in its `MANIFEST.md`). Its
//! token colors highlight the diff, and its background and text become the pane's palette,
//! the rest derived by the main themes' rules from the syntax theme's own colors. Where the
//! source names diff fills of its own, they win while they keep the code legible. `main`, the
//! default, names none: the pane wears the main theme.

// This file is a color table; 6-digit `0xRRGGBB` literals read better grouped as one value.
#![allow(clippy::unreadable_literal)]

use std::io::Cursor;
use std::str::FromStr;

use ratatui::style::Color;
use syntect::highlighting::{Highlighter, ThemeSet};
use syntect::parsing::ScopeStack;

use crate::theme::{Appearance, Palette, SyntaxChoice, Theme};

/// The diff pane follows the main theme: the default, and the picker's top row.
pub const MAIN: &str = "main";

/// The diff pane wears the saved dark or light diff theme matching the terminal.
pub const AUTO: &str = "auto";

macro_rules! entry {
    ($name:literal, $appearance:ident) => {
        (
            $name,
            Appearance::$appearance,
            include_bytes!(concat!("../assets/diff-themes/", $name, ".tmTheme")) as &[u8],
        )
    };
}

/// Every diff theme, its appearance, and its `.tmTheme` bytes: twenty dark, then twenty light,
/// each side by popularity with the Xcode, VS Code, and GitHub defaults first.
pub const CATALOG: &[(&str, Appearance, &[u8])] = &[
    // Dark.
    entry!("xcode-dark", Dark),
    entry!("vscode-dark", Dark),
    entry!("github-dark", Dark),
    entry!("one-dark-pro", Dark),
    entry!("dracula", Dark),
    entry!("monokai", Dark),
    entry!("ayu-dark", Dark),
    entry!("winter-is-coming-dark", Dark),
    entry!("night-owl", Dark),
    entry!("tokyo-night", Dark),
    entry!("palenight", Dark),
    entry!("synthwave-84", Dark),
    entry!("shades-of-purple", Dark),
    entry!("cobalt2", Dark),
    entry!("solarized-dark", Dark),
    entry!("catppuccin-mocha", Dark),
    entry!("noctis", Dark),
    entry!("nord", Dark),
    entry!("gruvbox-dark", Dark),
    entry!("rose-pine", Dark),
    // Light.
    entry!("xcode-light", Light),
    entry!("vscode-light", Light),
    entry!("github-light", Light),
    entry!("ayu-light", Light),
    entry!("winter-is-coming-light", Light),
    entry!("light-owl", Light),
    entry!("tokyo-night-light", Light),
    entry!("one-light", Light),
    entry!("catppuccin-latte", Light),
    entry!("noctis-lux", Light),
    entry!("gruvbox-light", Light),
    entry!("solarized-light", Light),
    entry!("quiet-light", Light),
    entry!("min-light", Light),
    entry!("horizon-bright", Light),
    entry!("rose-pine-dawn", Light),
    entry!("gruvbox-material-light", Light),
    entry!("tomorrow", Light),
    entry!("everforest-light", Light),
    entry!("flexoki-light", Light),
];

/// The [`CATALOG`] themes of `appearance`, in catalog order — one side of the picker.
pub fn names(appearance: Appearance) -> Vec<&'static str> {
    CATALOG.iter().filter(|(_, a, _)| *a == appearance).map(|&(n, ..)| n).collect()
}

/// The appearance of diff theme `name`; `None` outside the catalog.
pub fn appearance_of(name: &str) -> Option<Appearance> {
    CATALOG.iter().find(|(n, ..)| *n == name).map(|&(_, a, _)| a)
}

/// Whether `name` is one of the [`CATALOG`] themes.
pub fn is_builtin(name: &str) -> bool {
    appearance_of(name).is_some()
}

/// Diff theme `name` as a [`Theme`]: its palette, and its `.tmTheme` as the syntax theme.
/// `None` outside the catalog, or when its file fails to parse.
pub fn resolve(name: &str) -> Option<Theme> {
    let &(name, appearance, bytes) = CATALOG.iter().find(|(n, ..)| *n == name)?;
    let theme = match ThemeSet::load_from_reader(&mut Cursor::new(bytes)) {
        Ok(theme) => theme,
        Err(e) => {
            crate::logln!("diff theme {name:?} failed to parse: {e}");
            return None;
        }
    };
    let rgb = |c: syntect::highlighting::Color| Color::Rgb(c.r, c.g, c.b);
    let base = theme.settings.background.map(rgb)?;
    let text = theme.settings.foreground.map(rgb)?;
    let highlighter = Highlighter::new(&theme);
    // The first scope the theme colors apart from its text and visibly off its background (a
    // scope painted as white on a red fill names no red), else a stock hue.
    let accent = |scopes: &[&str], fallback: u32| {
        scopes
            .iter()
            .filter_map(|s| ScopeStack::from_str(s).ok())
            .map(|stack| rgb(highlighter.style_for_stack(stack.as_slice()).foreground))
            .find(|&c| c != text && crate::theme::contrast(c, base) >= 1.5)
            .unwrap_or_else(|| hex(fallback))
    };
    let accents = [
        accent(&["markup.deleted", "invalid"], 0xe0556b),
        accent(&["markup.inserted", "string"], 0x5fa860),
        accent(&["entity.name.type", "support.type", "storage.type"], 0xd7a54a),
        accent(&["constant.numeric", "constant"], 0xe08a4a),
        accent(&["keyword", "storage"], 0xa877d8),
        accent(&["entity.name.function", "support.function"], 0x4f8fe0),
    ];
    let mut palette = crate::theme::derive_palette(base, text, accents, appearance);
    fills(&mut palette, bytes, appearance);
    Some(Theme { name, palette, syntax: SyntaxChoice::Bundled(bytes) })
}

/// The diff fills: the theme's own where its source names them — `diffInserted`/`diffDeleted`
/// for a changed line, `diffInsertedText`/`diffDeletedText` laid over that for an edited word,
/// as VS Code paints them — else `palette`'s derived ones. A derived fill that gave up its
/// tint to stay legible is tinted again against [`fill_floor`], and an own fill below it
/// keeps the derived one.
fn fills(palette: &mut Palette, bytes: &[u8], appearance: Appearance) {
    let p = *palette;
    let floor = fill_floor(&p);
    let legible = |fill: Color| crate::theme::contrast(p.text, fill) >= floor;
    let (line_start, word_start) = match appearance {
        Appearance::Dark => (0.20, 0.38),
        Appearance::Light => (0.12, 0.22),
    };
    let text = String::from_utf8_lossy(bytes);
    for (line_key, word_key, accent, line, word) in [
        (
            "diffInserted",
            "diffInsertedText",
            p.green,
            &mut palette.ins_bg,
            &mut palette.emph_ins_bg,
        ),
        ("diffDeleted", "diffDeletedText", p.red, &mut palette.del_bg, &mut palette.emph_del_bg),
    ] {
        for (fill, start) in [(&mut *line, line_start), (&mut *word, word_start)] {
            if *fill == p.base {
                *fill = tint(accent, &p, start, floor);
            }
        }
        let Some(line_fill) = setting(&text, line_key).map(|c| over(c, p.base)) else { continue };
        if !legible(line_fill) {
            continue;
        }
        *line = line_fill;
        if let Some(word_fill) = setting(&text, word_key).map(|c| over(c, line_fill))
            && legible(word_fill)
            && word_fill != line_fill
        {
            *word = word_fill;
        }
    }
}

/// The contrast a diff fill keeps against the text: [`crate::theme::MIN_FILL_CONTRAST`], or
/// for a theme whose text already sits below it on its own background (Solarized Light's),
/// nine tenths of that, so its rows still tint.
fn fill_floor(p: &Palette) -> f64 {
    crate::theme::MIN_FILL_CONTRAST.min(crate::theme::contrast(p.text, p.base) * 0.9)
}

/// Tint `p.base` with `accent`, stepping down from `start` until the text clears `floor`.
fn tint(accent: Color, p: &Palette, start: f64, floor: f64) -> Color {
    let mut t = start;
    while t > 0.0 {
        let fill = crate::theme::blend(p.base, accent, t);
        if crate::theme::contrast(p.text, fill) >= floor {
            return fill;
        }
        t -= 0.02;
    }
    p.base
}

/// An RGBA color: its channels and opacity.
type Rgba = (u8, u8, u8, f64);

/// The value of global setting `key` in a `.tmTheme`'s text: the `<string>` after its `<key>`.
fn setting(text: &str, key: &str) -> Option<Rgba> {
    let after = &text[text.find(&format!("<key>{key}</key>"))?..];
    let value = &after[after.find("<string>")? + "<string>".len()..];
    parse_hex(&value[..value.find("</string>")?])
}

/// `#RGB`, `#RRGGBB`, or `#RRGGBBAA`.
fn parse_hex(s: &str) -> Option<Rgba> {
    let digits = s.trim().strip_prefix('#')?;
    let expanded: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => digits.to_owned(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(expanded.get(i..i + 2)?, 16).ok();
    let alpha = if expanded.len() == 8 { f64::from(byte(6)?) / 255.0 } else { 1.0 };
    Some((byte(0)?, byte(2)?, byte(4)?, alpha))
}

/// `fill` composited over the opaque `under`.
fn over((r, g, b, a): Rgba, under: Color) -> Color {
    crate::theme::blend(under, Color::Rgb(r, g, b), a)
}

const fn hex(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_a_side_with_the_editor_defaults_first() {
        let dark = names(Appearance::Dark);
        let light = names(Appearance::Light);
        assert_eq!((dark.len(), light.len()), (20, 20));
        assert_eq!(dark[..3], ["xcode-dark", "vscode-dark", "github-dark"]);
        assert_eq!(light[..3], ["xcode-light", "vscode-light", "github-light"]);
    }

    #[test]
    fn every_theme_resolves_with_tinted_legible_fills() {
        for &(name, appearance, _) in CATALOG {
            let theme = resolve(name).unwrap_or_else(|| panic!("{name} should resolve"));
            assert_eq!(theme.name, name);
            let p = theme.palette;
            let lum = |c: Color| crate::theme::contrast(c, Color::Rgb(0, 0, 0));
            let light_base = lum(p.base) > lum(p.text);
            assert_eq!(
                light_base,
                appearance == Appearance::Light,
                "{name}: listed on the wrong side"
            );
            for fill in [p.del_bg, p.ins_bg, p.emph_del_bg, p.emph_ins_bg] {
                assert_ne!(fill, p.base, "{name}: a diff fill with no tint");
                let c = crate::theme::contrast(p.text, fill);
                assert!(c >= fill_floor(&p), "{name}: fill {fill:?} at {c:.2}");
            }
        }
    }

    #[test]
    fn xcode_paints_its_comparison_colors() {
        let p = resolve("xcode-dark").unwrap().palette;
        // The changed-words fills sit over their line's band: orange at 25%, blue at 50%.
        let orange = over((0x74, 0x52, 0x38, 64.0 / 255.0), hex(0x38393f));
        let blue = over((0x1c, 0x48, 0x72, 128.0 / 255.0), hex(0x282d35));
        assert_eq!((p.del_bg, p.emph_del_bg), (hex(0x38393f), orange), "grey, orange");
        assert_eq!((p.ins_bg, p.emph_ins_bg), (hex(0x282d35), blue), "slate, blue");
        assert_eq!((p.red, p.green), (hex(0xe8935a), hex(0x4c9bff)), "the gutter signs");
    }

    #[test]
    fn parses_a_translucent_fill_and_composites_it() {
        assert_eq!(parse_hex("#ff000080"), Some((255, 0, 0, 128.0 / 255.0)));
        assert_eq!(parse_hex("#abc"), Some((0xaa, 0xbb, 0xcc, 1.0)));
        assert_eq!(parse_hex("red"), None);
        assert_eq!(over((255, 255, 255, 0.5), Color::Rgb(0, 0, 0)), Color::Rgb(128, 128, 128));
    }
}
