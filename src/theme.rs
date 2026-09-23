//! The color model: named palettes, derivation from anchors, and selection.
//!
//! A theme is a few anchor colors plus a paired syntax theme;
//! every other slot is derived from the anchors. One theme — `catppuccin` — instead
//! pins its whole palette as a literal, to stay byte-identical to the pre-theming
//! colors. One selection sets both the chrome `Palette` and the syntax theme, so they
//! never desync. The whole frame paints on the theme's `base`, not the terminal's background.

// This file is a color table; 6-digit `0xRRGGBB` literals read better grouped as one value.
#![allow(clippy::unreadable_literal)]

use std::sync::OnceLock;
use std::time::Duration;

use ratatui::style::Color;
use two_face::theme::EmbeddedThemeName;

/// The default theme name; the fallback for an unset CLI value. `auto` follows the
/// terminal: Catppuccin Mocha on a dark background, Latte on a light one.
pub const DEFAULT: &str = "auto";

/// The theme that follows the terminal: its own ANSI colors and default background and text,
/// in place of any palette. Outside [`CATALOG`], since it is neither dark nor light.
pub const TERMINAL: &str = "terminal";

/// The themes `auto` picks when the config names none.
pub const DEFAULT_DARK: &str = "catppuccin";
pub const DEFAULT_LIGHT: &str = "catppuccin-latte";

/// The terminal background's appearance, probed once at startup by [`detect_appearance`].
/// Unset (tests, a failed probe) reads as dark.
static DETECTED: OnceLock<Appearance> = OnceLock::new();

/// Every built-in theme and its appearance: twenty dark, then twenty light.
pub const CATALOG: &[(&str, Appearance)] = &[
    ("catppuccin", Appearance::Dark),
    ("dracula", Appearance::Dark),
    ("one-dark", Appearance::Dark),
    ("nord", Appearance::Dark),
    ("gruvbox", Appearance::Dark),
    ("tokyo-night", Appearance::Dark),
    ("monokai", Appearance::Dark),
    ("solarized", Appearance::Dark),
    ("github-dark", Appearance::Dark),
    ("rose-pine", Appearance::Dark),
    ("night-owl", Appearance::Dark),
    ("material-darker", Appearance::Dark),
    ("ayu-dark", Appearance::Dark),
    ("everforest-dark", Appearance::Dark),
    ("kanagawa", Appearance::Dark),
    ("vscode-dark", Appearance::Dark),
    ("xcode-dark", Appearance::Dark),
    ("cobalt2", Appearance::Dark),
    ("ciapre", Appearance::Dark),
    ("tomorrow-night", Appearance::Dark),
    ("catppuccin-latte", Appearance::Light),
    ("solarized-light", Appearance::Light),
    ("github-light", Appearance::Light),
    ("xcode-light", Appearance::Light),
    ("one-light", Appearance::Light),
    ("gruvbox-light", Appearance::Light),
    ("tokyo-night-day", Appearance::Light),
    ("rose-pine-dawn", Appearance::Light),
    ("ayu-light", Appearance::Light),
    ("everforest-light", Appearance::Light),
    ("light-owl", Appearance::Light),
    ("tomorrow", Appearance::Light),
    ("kanagawa-lotus", Appearance::Light),
    ("alabaster", Appearance::Light),
    ("bluloco-light", Appearance::Light),
    ("selenized-light", Appearance::Light),
    ("flexoki-light", Appearance::Light),
    ("dayfox", Appearance::Light),
    ("terminal-basic", Appearance::Light),
    ("iceberg-light", Appearance::Light),
];

/// A theme's intrinsic cast, which sets the derivation direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

/// The syntax theme paired with a palette: a bundled `.tmTheme`'s vendored bytes (for themes
/// `two-face` lacks, and for Catppuccin Mocha kept byte-identical to today's), a theme from
/// the `two-face` embedded set, or token colors derived from the palette.
#[derive(Clone, Copy, Debug)]
pub enum SyntaxChoice {
    Bundled(&'static [u8]),
    Embedded(EmbeddedThemeName),
    /// Token colors derived from the theme's own palette, for a theme with no syntax theme.
    Derived(Palette),
}

/// A resolved theme: its name, the chrome `Palette`, and its paired syntax theme.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub palette: Palette,
    pub syntax: SyntaxChoice,
}

/// The resolved colors every UI element paints — one source for chrome and diff fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The theme's background: every cell without a fill of its own paints on it, and the
    /// modal scrim blends receding cells toward it.
    pub base: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface2: Color,
    pub dim2: Color,
    pub dim1: Color,
    pub dim0: Color,
    pub text: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub orange: Color,
    pub purple: Color,
    pub blue: Color,
    pub del_bg: Color,
    pub ins_bg: Color,
    pub emph_del_bg: Color,
    pub emph_ins_bg: Color,
    /// The search match highlight: a warm fill behind a matched substring, legible over a
    /// plain row, a syntax-colored row, and the preview's banded hit line alike.
    pub match_hl: Color,
    /// The text-selection highlight, live and settled: a cool fill distinct by hue from the
    /// `surface1`/`surface2` row fills, so a selection reads inside a cursor row in any pane
    pub sel_bg: Color,
}

impl Palette {
    /// The cursor-row fill: the strongest-contrast surface (`surface2`) in the focused pane, a
    /// step softer (`surface1`) when not, so which pane holds the cursor reads at a glance.
    /// ("Strongest", not "brightest": light themes step surfaces toward black, not white.)
    pub fn cursor_bg(&self, focused: bool) -> Color {
        if focused { self.surface2 } else { self.surface1 }
    }

    /// Lift a painted color onto a selection fill. The dim role (`dim2`) sits one surface
    /// step above the fill and all but vanishes on it, so it rises to `dim0` and the
    /// secondary parts of a selected row stay readable. Every other color
    /// already reads there and passes through. Each theme names both ends, so the mapping means
    /// the same thing in all of them.
    pub fn on_fill(&self, color: Color) -> Color {
        if color == self.dim2 { self.dim0 } else { color }
    }

    /// Recede a painted color behind an open modal: halfway to `base`, so the modal owns the
    /// eye while the page behind stays recognizable. Non-RGB colors are the
    /// terminal's own defaults, which have no known distance to `base`; they pass through.
    pub fn scrim(&self, color: Color) -> Color {
        match color {
            Color::Rgb(..) => blend(color, self.base, 0.5),
            other => other,
        }
    }

    /// Text on an accent fill (the caret block, a find match): `surface0` — but ANSI black under
    /// the `terminal` theme, whose `surface0` may be no color at all.
    pub fn ink(&self) -> Color {
        if self.follows_terminal() { Color::Black } else { self.surface0 }
    }

    /// Whether the bars (tab bar, footer, fold rows) paint a `surface0` fill. The `terminal`
    /// theme's dark surface is none, so its bars sit on the terminal's own background.
    pub fn fills_bars(&self) -> bool {
        self.surface0 != Color::Reset
    }

    /// Whether this is the `terminal` theme's palette — the only one on the terminal's own
    /// background — whose [`FAINT`] text the renderer paints at faint intensity and whose
    /// [`INVERSE`] fill it paints as reverse video.
    pub fn follows_terminal(&self) -> bool {
        self.base == Color::Reset
    }

    /// Step `color` toward `text` until it reads on `base` at [`MIN_TOKEN_CONTRAST`], so a pale
    /// accent (an ANSI yellow on a light background) stays legible as a token color.
    /// An ANSI color, or any color on the terminal's own background, has no known contrast and
    /// passes through.
    pub fn legible(&self, color: Color) -> Color {
        if !matches!((color, self.base), (Color::Rgb(..), Color::Rgb(..))) {
            return color;
        }
        (0..=10)
            .map(|step| blend(color, self.text, f64::from(step) / 10.0))
            .find(|c| contrast(*c, self.base) >= MIN_TOKEN_CONTRAST)
            .unwrap_or(self.text)
    }
}

/// The terminal appearance [`detect_appearance`] recorded; dark when it has not run.
pub fn detected() -> Appearance {
    DETECTED.get().copied().unwrap_or(Appearance::Dark)
}

/// Resolve a theme name to a `Theme` for the detected terminal appearance.
pub fn resolve(name: Option<&str>) -> Theme {
    resolve_for(name, detected())
}

/// Resolve a theme name to a `Theme`. `None`, `auto`, or an unknown name falls back to the
/// default for `appearance` and logs; never a half-palette. `terminal` takes its fills'
/// direction from `appearance`.
pub fn resolve_for(name: Option<&str>, appearance: Appearance) -> Theme {
    let auto = || match appearance {
        Appearance::Dark => catppuccin(),
        Appearance::Light => catppuccin_latte(),
    };
    match name {
        None | Some(DEFAULT) => auto(),
        Some(TERMINAL) => follow_terminal(appearance),
        Some(n) => build(n).unwrap_or_else(|| {
            logln!("unknown theme {n:?}; using {DEFAULT}");
            auto()
        }),
    }
}

/// Whether `name` selects a complete built-in theme. Plugin configuration validates against
/// this same catalog before a snapshot is applied.
pub fn is_known(name: &str) -> bool {
    name == DEFAULT || name == TERMINAL || is_builtin(name)
}

/// The [`CATALOG`] themes of `appearance`, alphabetical — one side of the theme picker.
pub fn names(appearance: Appearance) -> Vec<&'static str> {
    let mut names: Vec<_> =
        CATALOG.iter().filter(|(_, a)| *a == appearance).map(|&(n, _)| n).collect();
    names.sort_unstable();
    names
}

/// The appearance of catalog theme `name`; `None` outside the catalog.
pub fn appearance_of(name: &str) -> Option<Appearance> {
    CATALOG.iter().find(|(n, _)| *n == name).map(|&(_, a)| a)
}

/// Whether `name` is one of the [`CATALOG`] themes — `auto` excluded, since it names none.
pub fn is_builtin(name: &str) -> bool {
    build(name).is_some()
}

/// How long the startup probe waits for the terminal to answer an OSC 11 query.
const PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Probe the terminal background once and record it for `auto`: `COLORFGBG` first, then an
/// OSC 11 query, dark when neither answers.
///
/// Must run before raw mode and the event loop: the probe reads the tty, and a read racing
/// the loop steals keypresses (design doc §7).
///
/// NOTE: startup-only. A light/dark switch mid-session (DEC mode 2031) is not followed yet.
pub fn detect_appearance() {
    let colorfgbg = std::env::var("COLORFGBG").ok();
    let appearance = colorfgbg
        .as_deref()
        .and_then(appearance_from_colorfgbg)
        .or_else(query_appearance)
        .unwrap_or(Appearance::Dark);
    logln!("terminal appearance={appearance:?} COLORFGBG={colorfgbg:?}");
    let _ = DETECTED.set(appearance);
}

/// The appearance a `COLORFGBG` value (`fg;bg` or `fg;default;bg`) names: light for a white
/// or light-gray background index, dark for any other index, `None` when the last field is
/// not an index.
fn appearance_from_colorfgbg(value: &str) -> Option<Appearance> {
    let bg: u8 = value.rsplit(';').next()?.parse().ok()?;
    Some(if matches!(bg, 7 | 15) { Appearance::Light } else { Appearance::Dark })
}

/// Ask the terminal for its colors over OSC 10/11; `None` when it does not answer in time.
fn query_appearance() -> Option<Appearance> {
    let mut options = terminal_colorsaurus::QueryOptions::default();
    options.timeout = PROBE_TIMEOUT;
    match terminal_colorsaurus::theme_mode(options) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Appearance::Light),
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Appearance::Dark),
        Err(error) => {
            logln!("terminal color query failed: {error}");
            None
        }
    }
}

/// The built theme for `name`, or `None` when it is not a known palette. Names match herdr's
/// so the value a user copies from their herdr config resolves to the same palette.
fn build(name: &str) -> Option<Theme> {
    use Appearance::{Dark, Light};
    use EmbeddedThemeName as E;
    Some(match name {
        // Dark.
        "catppuccin" => catppuccin(),
        "dracula" => derived("dracula", Dark, E::Dracula, DRACULA),
        "one-dark" => derived("one-dark", Dark, E::TwoDark, ONE_DARK),
        "nord" => derived("nord", Dark, E::Nord, NORD),
        "gruvbox" => derived("gruvbox", Dark, E::GruvboxDark, GRUVBOX),
        "tokyo-night" => bundled("tokyo-night", Dark, TOKYO_NIGHT_TM, TOKYO_NIGHT),
        "monokai" => derived("monokai", Dark, E::MonokaiExtended, MONOKAI),
        "solarized" => derived("solarized", Dark, E::SolarizedDark, SOLARIZED),
        "github-dark" => terminal("github-dark", Dark, GITHUB_DARK),
        "rose-pine" => bundled("rose-pine", Dark, ROSE_PINE_TM, ROSE_PINE),
        "night-owl" => terminal("night-owl", Dark, NIGHT_OWL),
        "material-darker" => terminal("material-darker", Dark, MATERIAL_DARKER),
        "ayu-dark" => terminal("ayu-dark", Dark, AYU_DARK),
        "everforest-dark" => terminal("everforest-dark", Dark, EVERFOREST_DARK),
        "kanagawa" => terminal("kanagawa", Dark, KANAGAWA),
        "vscode-dark" => terminal("vscode-dark", Dark, VSCODE_DARK),
        "xcode-dark" => terminal("xcode-dark", Dark, XCODE_DARK),
        "cobalt2" => terminal("cobalt2", Dark, COBALT2),
        "ciapre" => terminal("ciapre", Dark, CIAPRE),
        "tomorrow-night" => terminal("tomorrow-night", Dark, TOMORROW_NIGHT),
        // Light.
        "catppuccin-latte" => catppuccin_latte(),
        "solarized-light" => derived("solarized-light", Light, E::SolarizedLight, SOLARIZED_LIGHT),
        "github-light" => derived("github-light", Light, E::Github, GITHUB_LIGHT),
        "xcode-light" => terminal("xcode-light", Light, XCODE_LIGHT),
        "one-light" => derived("one-light", Light, E::OneHalfLight, ONE_LIGHT),
        "gruvbox-light" => derived("gruvbox-light", Light, E::GruvboxLight, GRUVBOX_LIGHT),
        "tokyo-night-day" => bundled("tokyo-night-day", Light, TOKYO_NIGHT_DAY_TM, TOKYO_NIGHT_DAY),
        "rose-pine-dawn" => bundled("rose-pine-dawn", Light, ROSE_PINE_DAWN_TM, ROSE_PINE_DAWN),
        "ayu-light" => terminal("ayu-light", Light, AYU_LIGHT),
        "everforest-light" => terminal("everforest-light", Light, EVERFOREST_LIGHT),
        "light-owl" => terminal("light-owl", Light, LIGHT_OWL),
        "tomorrow" => terminal("tomorrow", Light, TOMORROW),
        "kanagawa-lotus" => terminal("kanagawa-lotus", Light, KANAGAWA_LOTUS),
        "alabaster" => terminal("alabaster", Light, ALABASTER),
        "bluloco-light" => terminal("bluloco-light", Light, BLULOCO_LIGHT),
        "selenized-light" => terminal("selenized-light", Light, SELENIZED_LIGHT),
        "flexoki-light" => terminal("flexoki-light", Light, FLEXOKI_LIGHT),
        "dayfox" => terminal("dayfox", Light, DAYFOX),
        "terminal-basic" => terminal("terminal-basic", Light, TERMINAL_BASIC),
        "iceberg-light" => terminal("iceberg-light", Light, ICEBERG_LIGHT),
        _ => return None,
    })
}

/// A theme ported from a terminal color scheme: its palette is derived from `anchors`, and with
/// no syntax theme of its own, its token colors are derived from that palette too.
fn terminal(name: &'static str, appearance: Appearance, anchors: Anchors) -> Theme {
    let palette = derive(anchors, appearance);
    Theme { name, palette, syntax: SyntaxChoice::Derived(palette) }
}

/// A derived theme: its palette is computed from `anchors`, paired with a `two-face` syntax theme.
fn derived(
    name: &'static str,
    appearance: Appearance,
    syntax: EmbeddedThemeName,
    anchors: Anchors,
) -> Theme {
    Theme { name, palette: derive(anchors, appearance), syntax: SyntaxChoice::Embedded(syntax) }
}

/// The anchor colors a derived theme lists; the rest of its palette is computed from these.
#[derive(Clone, Copy, Debug)]
struct Anchors {
    base: Color,
    text: Color,
    red: Color,
    green: Color,
    yellow: Color,
    orange: Color,
    purple: Color,
    blue: Color,
}

/// The `terminal` theme's dim text, as its palette carries it: the renderer paints it as the
/// terminal's default text at faint intensity ([`Palette::follows_terminal`]). ANSI has no dim
/// color — bright black is near-black in some schemes (Solarized's is its darkest tone) — so the
/// faint attribute is the one dim that reads the same under every scheme.
pub const FAINT: Color = Color::DarkGray;

/// The `terminal` theme's strongest fill on a dark terminal, as its palette carries it: the
/// renderer paints it as reverse video over default text ([`Palette::follows_terminal`]). No
/// ANSI color is reliably off every dark background — Solarized's bright black is its
/// background — but reverse video always is. Never painted as itself.
pub const INVERSE: Color = Color::Indexed(255);

/// The `terminal` theme: the terminal's default background and text, and its sixteen ANSI
/// colors for everything else, so the terminal's own scheme decides how it looks. ANSI colors
/// cannot be blended, so there are no tinted diff rows — the `▌` bars mark them — the dim
/// roles are [`FAINT`], and the surfaces are the grays, ordered by `appearance` — on a dark
/// terminal the strongest is [`INVERSE`].
fn follow_terminal(appearance: Appearance) -> Theme {
    let (surface, soft, strong) = match appearance {
        // No ANSI color is a quiet bar on every dark background (black is a hole on a gray
        // one), so the dark bars take no fill.
        Appearance::Dark => (Color::Reset, Color::Black, INVERSE),
        Appearance::Light => (Color::White, Color::Gray, Color::Gray),
    };
    let palette = Palette {
        base: Color::Reset,
        surface0: surface,
        surface1: soft,
        surface2: strong,
        dim2: FAINT,
        dim1: FAINT,
        // The dim role lifted onto a fill is full-strength text.
        dim0: Color::Reset,
        text: Color::Reset,
        red: Color::Red,
        green: Color::Green,
        yellow: Color::Yellow,
        orange: Color::LightRed,
        purple: Color::Magenta,
        blue: Color::Blue,
        del_bg: Color::Reset,
        ins_bg: Color::Reset,
        emph_del_bg: Color::Red,
        emph_ins_bg: Color::Green,
        match_hl: Color::Yellow,
        sel_bg: Color::Blue,
    };
    Theme { name: TERMINAL, palette, syntax: SyntaxChoice::Derived(palette) }
}

/// Catppuccin Mocha: pinned to its canonical values so it renders identically to the
/// pre-theming palette.
fn catppuccin() -> Theme {
    Theme {
        name: "catppuccin",
        palette: Palette {
            base: Color::Rgb(0x1e, 0x1e, 0x2e),
            surface0: Color::Rgb(0x31, 0x32, 0x44),
            surface1: Color::Rgb(0x45, 0x47, 0x5a),
            surface2: Color::Rgb(0x58, 0x5b, 0x70),
            dim2: Color::Rgb(0x6c, 0x70, 0x86),
            dim1: Color::Rgb(0x7f, 0x84, 0x9c),
            dim0: Color::Rgb(0xa6, 0xad, 0xc8),
            text: Color::Rgb(0xcd, 0xd6, 0xf4),
            red: Color::Rgb(0xf3, 0x8b, 0xa8),
            green: Color::Rgb(0xa6, 0xe3, 0xa1),
            yellow: Color::Rgb(0xf9, 0xe2, 0xaf),
            orange: Color::Rgb(0xfa, 0xb3, 0x87),
            purple: Color::Rgb(0xcb, 0xa6, 0xf7),
            blue: Color::Rgb(0xb4, 0xbe, 0xfe),
            del_bg: Color::Rgb(0x45, 0x23, 0x2f),
            ins_bg: Color::Rgb(0x1f, 0x3a, 0x2a),
            emph_del_bg: Color::Rgb(0x6e, 0x34, 0x46),
            emph_ins_bg: Color::Rgb(0x30, 0x55, 0x3f),
            match_hl: Color::Rgb(0x5c, 0x51, 0x2b),
            sel_bg: Color::Rgb(0x35, 0x3d, 0x7d),
        },
        syntax: SyntaxChoice::Bundled(MOCHA_TM),
    }
}

/// A theme whose palette is derived from `anchors`, paired with a bundled `.tmTheme`'s bytes.
fn bundled(
    name: &'static str,
    appearance: Appearance,
    syntax: &'static [u8],
    anchors: Anchors,
) -> Theme {
    Theme { name, palette: derive(anchors, appearance), syntax: SyntaxChoice::Bundled(syntax) }
}

/// Vendored `.tmTheme` assets for the syntax themes `two-face` does not carry (and Mocha,
/// kept as the byte-identical source of today's highlighting). Licenses listed in the
/// README's License section.
const MOCHA_TM: &[u8] = include_bytes!("../assets/Catppuccin Mocha.tmTheme");
const TOKYO_NIGHT_TM: &[u8] = include_bytes!("../assets/tokyo-night.tmTheme");
const TOKYO_NIGHT_DAY_TM: &[u8] = include_bytes!("../assets/tokyo-night-day.tmTheme");
const ROSE_PINE_TM: &[u8] = include_bytes!("../assets/rose-pine.tmTheme");
const ROSE_PINE_DAWN_TM: &[u8] = include_bytes!("../assets/rose-pine-dawn.tmTheme");

/// Catppuccin Latte: a light theme, derived from its anchors to exercise the derivation
/// path (and paired with `two-face`'s Latte syntax theme).
fn catppuccin_latte() -> Theme {
    derived(
        "catppuccin-latte",
        Appearance::Light,
        EmbeddedThemeName::CatppuccinLatte,
        CATPPUCCIN_LATTE,
    )
}

const CATPPUCCIN_LATTE: Anchors =
    anchors(0xeff1f5, 0x4c4f69, 0xd20f39, 0x40a02b, 0xdf8e1d, 0xfe640b, 0x8839ef, 0x7287fd);

/// Canonical anchors for the derived themes. base, text, then the six accents
/// (red, green, yellow, orange, purple, blue); surfaces and diff fills are derived.
const DRACULA: Anchors =
    anchors(0x282a36, 0xf8f8f2, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xffb86c, 0xbd93f9, 0x8be9fd);
const NORD: Anchors =
    anchors(0x2e3440, 0xd8dee9, 0xbf616a, 0xa3be8c, 0xebcb8b, 0xd08770, 0xb48ead, 0x81a1c1);
const GRUVBOX: Anchors =
    anchors(0x282828, 0xebdbb2, 0xfb4934, 0xb8bb26, 0xfabd2f, 0xfe8019, 0xd3869b, 0x83a598);
const GRUVBOX_LIGHT: Anchors =
    anchors(0xfbf1c7, 0x3c3836, 0x9d0006, 0x79740e, 0xb57614, 0xaf3a03, 0x8f3f71, 0x076678);
const ONE_DARK: Anchors =
    anchors(0x282c34, 0xabb2bf, 0xe06c75, 0x98c379, 0xe5c07b, 0xd19a66, 0xc678dd, 0x61afef);
const ONE_LIGHT: Anchors =
    anchors(0xfafafa, 0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x986801, 0xa626a4, 0x4078f2);
const SOLARIZED: Anchors =
    anchors(0x002b36, 0x93a1a1, 0xdc322f, 0x859900, 0xb58900, 0xcb4b16, 0x6c71c4, 0x268bd2);
const SOLARIZED_LIGHT: Anchors =
    anchors(0xfdf6e3, 0x586e75, 0xdc322f, 0x859900, 0xb58900, 0xcb4b16, 0x6c71c4, 0x268bd2);
const GITHUB_LIGHT: Anchors =
    anchors(0xffffff, 0x1f2328, 0xcf222e, 0x1a7f37, 0x9a6700, 0xbc4c00, 0x8250df, 0x0969da);
const MONOKAI: Anchors =
    anchors(0x272822, 0xf8f8f2, 0xf92672, 0xa6e22e, 0xe6db74, 0xfd971f, 0xae81ff, 0x66d9ef);
const TOKYO_NIGHT: Anchors =
    anchors(0x1a1b26, 0xc0caf5, 0xf7768e, 0x9ece6a, 0xe0af68, 0xff9e64, 0xbb9af7, 0x7aa2f7);
const TOKYO_NIGHT_DAY: Anchors =
    anchors(0xe1e2e7, 0x3760bf, 0xf52a65, 0x587539, 0x8c6c3e, 0xb15c00, 0x9854f1, 0x2e7de9);
const ROSE_PINE: Anchors =
    anchors(0x191724, 0xe0def4, 0xeb6f92, 0x9ccfd8, 0xf6c177, 0xebbcba, 0xc4a7e7, 0x31748f);
const ROSE_PINE_DAWN: Anchors =
    anchors(0xfaf4ed, 0x575279, 0xb4637a, 0x56949f, 0xea9d34, 0xd7827e, 0x907aa9, 0x286983);

// Ported from terminal color schemes (iTerm2-Color-Schemes, MIT): the scheme's background,
// foreground and ANSI red, green, yellow, magenta and blue. ANSI has no orange, so orange is
// the midpoint of red and yellow.
const GITHUB_DARK: Anchors =
    anchors(0x0d1117, 0xe6edf3, 0xff7b72, 0x3fb950, 0xd29922, 0xe98a4a, 0xbc8cff, 0x58a6ff);
const NIGHT_OWL: Anchors =
    anchors(0x011627, 0xd6deeb, 0xef5350, 0x22da6e, 0xaddb67, 0xce975c, 0xc792ea, 0x82aaff);
const MATERIAL_DARKER: Anchors =
    anchors(0x212121, 0xeeffff, 0xff5370, 0xc3e88d, 0xffcb6b, 0xff8f6e, 0xc792ea, 0x82aaff);
const AYU_DARK: Anchors =
    anchors(0x0b0e14, 0xbfbdb6, 0xea6c73, 0x7fd962, 0xf9af4f, 0xf28e61, 0xcda1fa, 0x53bdfa);
const EVERFOREST_DARK: Anchors =
    anchors(0x232a2e, 0xd3c6aa, 0xe67e80, 0xa7c080, 0xdbbc7f, 0xe19d80, 0xd699b6, 0x7fbbb3);
const KANAGAWA: Anchors =
    anchors(0x1f1f28, 0xdcd7ba, 0xc34043, 0x76946a, 0xc0a36e, 0xc27259, 0x957fb8, 0x7e9cd8);
const VSCODE_DARK: Anchors =
    anchors(0x1e1e1e, 0xcccccc, 0xcd3131, 0x0dbc79, 0xe5e510, 0xd98b21, 0xbc3fbc, 0x2472c8);
const XCODE_DARK: Anchors =
    anchors(0x292a30, 0xdfdfe0, 0xff8170, 0x78c2b3, 0xd9c97c, 0xeca576, 0xff7ab2, 0x4eb0cc);
const COBALT2: Anchors =
    anchors(0x132738, 0xffffff, 0xff0000, 0x38de21, 0xffe50a, 0xff7305, 0xff005d, 0x1460d2);
const CIAPRE: Anchors =
    anchors(0x191c27, 0xaea47a, 0x8e0d16, 0x48513b, 0xcc8b3f, 0xad4c2b, 0x724d7c, 0x576d8c);
const TOMORROW_NIGHT: Anchors =
    anchors(0x1d1f21, 0xc5c8c6, 0xcc6666, 0xb5bd68, 0xf0c674, 0xde966d, 0xb294bb, 0x81a2be);
const XCODE_LIGHT: Anchors =
    anchors(0xffffff, 0x262626, 0xd12f1b, 0x3e8087, 0x78492a, 0xa53c23, 0xad3da4, 0x0f68a0);
const AYU_LIGHT: Anchors =
    anchors(0xf8f9fa, 0x5c6166, 0xea6c6d, 0x6cbf43, 0xeca944, 0xeb8b59, 0x9e75c7, 0x3199e1);
const EVERFOREST_LIGHT: Anchors =
    anchors(0xefebd4, 0x5c6a72, 0xe67e80, 0x9ab373, 0xc1a266, 0xd49073, 0xd699b6, 0x7fbbb3);
const LIGHT_OWL: Anchors =
    anchors(0xfbfbfb, 0x403f53, 0xde3d3b, 0x08916a, 0xe0af02, 0xdf761f, 0xd6438a, 0x288ed7);
const TOMORROW: Anchors =
    anchors(0xffffff, 0x4d4d4c, 0xc82829, 0x718c00, 0xeab700, 0xd97015, 0x8959a8, 0x4271ae);
const KANAGAWA_LOTUS: Anchors =
    anchors(0xf2ecbc, 0x545464, 0xc84053, 0x6f894e, 0x77713f, 0xa05949, 0xb35b79, 0x4d699b);
const ALABASTER: Anchors =
    anchors(0xf7f7f7, 0x000000, 0xaa3731, 0x448c27, 0xcb9000, 0xbb6419, 0x7a3e9d, 0x325cc0);
const BLULOCO_LIGHT: Anchors =
    anchors(0xf9f9f9, 0x373a41, 0xd52753, 0x23974a, 0xdf631c, 0xda4538, 0x823ff1, 0x275fe4);
const SELENIZED_LIGHT: Anchors =
    anchors(0xfbf3db, 0x53676d, 0xd2212d, 0x489100, 0xad8900, 0xc05517, 0xca4898, 0x0072d4);
const FLEXOKI_LIGHT: Anchors =
    anchors(0xfffcf0, 0x100f0f, 0xaf3029, 0x66800b, 0xad8301, 0xae5a15, 0xa02f6f, 0x205ea6);
const DAYFOX: Anchors =
    anchors(0xf6f2ee, 0x3d2b5a, 0xa5222f, 0x396847, 0xac5402, 0xa93b19, 0x6e33ce, 0x2848a9);
const TERMINAL_BASIC: Anchors =
    anchors(0xffffff, 0x000000, 0x990000, 0x00a600, 0x999900, 0x994d00, 0xb200b2, 0x0000b2);
const ICEBERG_LIGHT: Anchors =
    anchors(0xe8e9ec, 0x33374c, 0xcc517a, 0x668e3d, 0xc57339, 0xc9625a, 0x7759b4, 0x2d539e);

/// Build `Anchors` from `0xRRGGBB` hex literals, so a palette reads as one compact row.
/// One argument per anchor slot — the count is the palette's shape, not accidental.
#[allow(clippy::too_many_arguments)]
const fn anchors(
    base: u32,
    text: u32,
    red: u32,
    green: u32,
    yellow: u32,
    orange: u32,
    purple: u32,
    blue: u32,
) -> Anchors {
    Anchors {
        base: hex(base),
        text: hex(text),
        red: hex(red),
        green: hex(green),
        yellow: hex(yellow),
        orange: hex(orange),
        purple: hex(purple),
        blue: hex(blue),
    }
}

/// A `Color::Rgb` from a `0xRRGGBB` literal.
const fn hex(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Build a full palette from anchors: surfaces step `base` toward the contrast pole
/// (lighter for a dark theme, darker for a light one); diff fills tint `base` with the
/// add/remove accent, kept legible against `text`.
fn derive(a: Anchors, appearance: Appearance) -> Palette {
    let pole = match appearance {
        Appearance::Dark => WHITE,
        Appearance::Light => BLACK,
    };
    let surface = |t: f64| blend(a.base, pole, t);
    Palette {
        base: a.base,
        surface0: surface(0.045),
        surface1: surface(0.09),
        surface2: surface(0.14),
        dim2: surface(0.26),
        dim1: surface(0.34),
        dim0: blend(a.text, a.base, 0.18),
        text: a.text,
        red: a.red,
        green: a.green,
        yellow: a.yellow,
        orange: a.orange,
        purple: a.purple,
        blue: a.blue,
        del_bg: readable_tint(a.red, a.base, a.text, appearance, false),
        ins_bg: readable_tint(a.green, a.base, a.text, appearance, false),
        emph_del_bg: readable_tint(a.red, a.base, a.text, appearance, true),
        emph_ins_bg: readable_tint(a.green, a.base, a.text, appearance, true),
        match_hl: readable_tint(a.yellow, a.base, a.text, appearance, true),
        sel_bg: readable_tint(saturated(a.blue), a.base, a.text, appearance, true),
    }
}

const WHITE: Color = Color::Rgb(0xff, 0xff, 0xff);
const BLACK: Color = Color::Rgb(0x00, 0x00, 0x00);

/// The lowest contrast a diff fill keeps against the row's text, so code on a fill stays
/// legible on any base.
const MIN_FILL_CONTRAST: f64 = 4.5;

/// The lowest contrast a derived syntax color keeps against `base`: below body text's 4.5, so
/// accents keep their hue, but clear of the pale ANSI yellows some light schemes carry.
const MIN_TOKEN_CONTRAST: f64 = 3.0;

/// A diff-row fill: tint `base` with `accent`, stepping the tint down from its start strength
/// until the row's `fg` clears [`MIN_FILL_CONTRAST`]. `strong` is the brighter word-emphasis
/// fill. When even a faint tint can't clear the floor (a light theme with light text), the
/// bare `base` wins — legibility over a visible tint.
fn readable_tint(
    accent: Color,
    base: Color,
    fg: Color,
    appearance: Appearance,
    strong: bool,
) -> Color {
    let start = match (appearance, strong) {
        (Appearance::Dark, false) => 0.20,
        (Appearance::Dark, true) => 0.38,
        (Appearance::Light, false) => 0.12,
        (Appearance::Light, true) => 0.22,
    };
    let mut t = start;
    while t > 0.0 {
        let fill = blend(base, accent, t);
        if contrast(fg, fill) >= MIN_FILL_CONTRAST {
            return fill;
        }
        t -= 0.02;
    }
    base
}

/// Halfway between an accent and its colorful core — the shared gray component removed and
/// the remainder rescaled to full range. A pastel anchor (Catppuccin's periwinkle `blue`)
/// tints `base` into the same gray family as the surface fills; the saturated version tints
/// it into an unmistakable hue instead, which is what lets the selection fill read inside a
/// cursor row. A gray anchor has no hue to amplify and passes through.
fn saturated(c: Color) -> Color {
    let (r, g, b) = channels(c);
    let lo = r.min(g).min(b);
    let span = r.max(g).max(b) - lo;
    if span == 0 {
        return c;
    }
    let core = |ch: u8| (f64::from(ch - lo) * 255.0 / f64::from(span)).round() as u8;
    blend(c, Color::Rgb(core(r), core(g), core(b)), 0.5)
}

/// Linear per-channel blend: `t` of the way from `from` to `to` (0.0 = `from`, 1.0 = `to`).
fn blend(from: Color, to: Color, t: f64) -> Color {
    let (fr, fg, fb) = channels(from);
    let (tr, tg, tb) = channels(to);
    let mix = |lhs: u8, rhs: u8| (f64::from(lhs) * (1.0 - t) + f64::from(rhs) * t).round() as u8;
    Color::Rgb(mix(fr, tr), mix(fg, tg), mix(fb, tb))
}

/// The WCAG contrast ratio between two colors (1.0 .. 21.0).
fn contrast(fg: Color, bg: Color) -> f64 {
    let (lf, lb) = (luminance(fg), luminance(bg));
    let (hi, lo) = if lf >= lb { (lf, lb) } else { (lb, lf) };
    (hi + 0.05) / (lo + 0.05)
}

/// WCAG relative luminance, with sRGB linearization.
fn luminance(color: Color) -> f64 {
    let (r, g, b) = channels(color);
    let lin = |channel: u8| {
        let srgb = f64::from(channel) / 255.0;
        if srgb <= 0.03928 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// The RGB channels of a color; anchors are always `Rgb`, so the fallback never fires.
fn channels(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Appearance, CATALOG, CATPPUCCIN_LATTE, MIN_FILL_CONTRAST, Palette, contrast, derive,
        resolve, resolve_for,
    };
    use ratatui::style::Color;

    #[test]
    fn contrast_black_white_is_max() {
        let r = contrast(Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255));
        assert!((r - 21.0).abs() < 0.01, "black vs white is ~21:1, got {r}");
    }

    #[test]
    fn catppuccin_is_the_unchanged_mocha_palette() {
        let p = resolve(Some("catppuccin")).palette;
        assert_eq!(p.surface0, Color::Rgb(0x31, 0x32, 0x44));
        assert_eq!(p.text, Color::Rgb(0xcd, 0xd6, 0xf4));
        assert_eq!(p.del_bg, Color::Rgb(0x45, 0x23, 0x2f));
        assert_eq!(p.ins_bg, Color::Rgb(0x1f, 0x3a, 0x2a));
        // The renamed slots keep their Mocha values: orange was peach, purple mauve,
        // blue lavender, and dim0/1/2 were subtext0/overlay1/overlay0.
        assert_eq!(p.orange, Color::Rgb(0xfa, 0xb3, 0x87));
        assert_eq!(p.purple, Color::Rgb(0xcb, 0xa6, 0xf7));
        assert_eq!(p.blue, Color::Rgb(0xb4, 0xbe, 0xfe));
        assert_eq!(p.dim0, Color::Rgb(0xa6, 0xad, 0xc8));
        assert_eq!(p.dim1, Color::Rgb(0x7f, 0x84, 0x9c));
        assert_eq!(p.dim2, Color::Rgb(0x6c, 0x70, 0x86));
        // The selection fill: saturated `blue` tinted over `base` at emphasis strength — a
        // real hue, nothing near the gray `surface1`/`surface2` cursor fills.
        assert_eq!(p.sel_bg, Color::Rgb(0x35, 0x3d, 0x7d));
    }

    #[test]
    fn unknown_falls_back_to_default_and_terminal_is_its_own() {
        assert_eq!(resolve(Some("nope")).name, "catppuccin");
        let terminal = resolve(Some("terminal"));
        assert_eq!(terminal.name, "terminal");
        assert_eq!((terminal.palette.base, terminal.palette.text), (Color::Reset, Color::Reset));
        assert!(super::is_known("terminal") && !super::is_builtin("terminal"));
        assert_eq!(resolve(None).name, "catppuccin");
    }

    #[test]
    fn auto_follows_the_terminal_appearance() {
        assert_eq!(resolve_for(Some("auto"), Appearance::Dark).name, "catppuccin");
        assert_eq!(resolve_for(Some("auto"), Appearance::Light).name, "catppuccin-latte");
        assert_eq!(resolve_for(None, Appearance::Light).name, "catppuccin-latte");
        assert_eq!(resolve_for(Some("nope"), Appearance::Light).name, "catppuccin-latte");
        // An explicit theme wins over the terminal.
        assert_eq!(resolve_for(Some("nord"), Appearance::Light).name, "nord");
        assert!(super::is_known("auto"));
    }

    #[test]
    fn names_split_the_catalog_by_appearance_alphabetically() {
        use super::{appearance_of, names};
        let dark = names(Appearance::Dark);
        let light = names(Appearance::Light);
        assert_eq!((dark[0], dark.len()), ("ayu-dark", 20));
        assert_eq!((light[0], light.len()), ("alabaster", 20));
        assert!(dark.is_sorted() && light.is_sorted(), "each side lists alphabetically");
        assert_eq!(appearance_of("iceberg-light"), Some(Appearance::Light));
        assert_eq!(appearance_of("auto"), None);
    }

    #[test]
    fn colorfgbg_names_the_background_index() {
        use super::appearance_from_colorfgbg as parse;
        assert_eq!(parse("15;0"), Some(Appearance::Dark));
        assert_eq!(parse("0;15"), Some(Appearance::Light));
        assert_eq!(parse("0;default;7"), Some(Appearance::Light));
        assert_eq!(parse("12;8"), Some(Appearance::Dark));
        assert_eq!(parse("15;default"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn latte_is_a_selectable_light_theme() {
        assert_eq!(resolve(Some("catppuccin-latte")).name, "catppuccin-latte");
    }

    #[test]
    fn light_derivation_keeps_diff_fills_legible() {
        // Exercise the shipped catppuccin-latte anchors, so a real retune that breaks the
        // contrast floor or the surface ramp is caught here.
        let anchors = CATPPUCCIN_LATTE;
        let p: Palette = derive(anchors, Appearance::Light);
        // Text stays readable on every derived fill, on a light base.
        for fill in [p.del_bg, p.ins_bg, p.emph_del_bg, p.emph_ins_bg] {
            assert!(
                contrast(p.text, fill) >= MIN_FILL_CONTRAST,
                "fill {fill:?} drops below the legibility floor",
            );
        }
        // A light theme steps its surfaces darker than the base, deepening along the ramp, so
        // the fills read against the light canvas.
        let base_lum = super::luminance(anchors.base);
        assert!(super::luminance(p.surface0) < base_lum, "surface0 is darker than the base");
        assert!(
            super::luminance(p.surface2) < super::luminance(p.surface0),
            "the surface ramp keeps darkening",
        );
    }

    #[test]
    fn the_catalog_is_twenty_dark_then_twenty_light() {
        let dark = CATALOG.iter().filter(|(_, a)| *a == Appearance::Dark).count();
        assert_eq!((dark, CATALOG.len() - dark), (20, 20));
        let mut names: Vec<_> = CATALOG.iter().map(|(n, _)| n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CATALOG.len(), "catalog names are unique");
    }

    #[test]
    fn derived_token_colors_clear_the_floor() {
        for &(name, _) in CATALOG {
            let p = resolve(Some(name)).palette;
            for accent in [p.red, p.green, p.yellow, p.orange, p.purple, p.blue, p.dim1] {
                let c = p.legible(accent);
                assert!(
                    contrast(c, p.base) >= super::MIN_TOKEN_CONTRAST || c == p.text,
                    "{name}: token {c:?} is illegible on {:?}",
                    p.base,
                );
            }
        }
    }

    #[test]
    fn every_named_theme_resolves_to_itself() {
        for &(name, _) in CATALOG {
            assert_eq!(resolve(Some(name)).name, name, "{name} should resolve to its own palette");
        }
    }

    #[test]
    fn every_theme_keeps_diff_fills_legible() {
        for &(name, _) in CATALOG {
            let p = resolve(Some(name)).palette;
            for fill in [p.del_bg, p.ins_bg, p.emph_del_bg, p.emph_ins_bg, p.sel_bg] {
                assert!(
                    contrast(p.text, fill) >= MIN_FILL_CONTRAST,
                    "{name}: fill {fill:?} drops below the legibility floor",
                );
            }
        }
    }

    #[test]
    fn appearance_orients_text_against_surface() {
        for &(name, appearance) in CATALOG {
            let light = appearance == Appearance::Light;
            let p = resolve(Some(name)).palette;
            // Light theme: dark text on a lighter surface. Dark theme: the reverse.
            let text_darker = super::luminance(p.text) < super::luminance(p.surface0);
            assert_eq!(text_darker, light, "{name}: text/surface contrast points the wrong way");
        }
    }
}
