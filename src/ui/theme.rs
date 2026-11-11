//! The dashboard's colors, by role. `theme = "..."` in config.toml picks one;
//! the default is Grove's own.

use std::cell::Cell;

use ratatui::style::Color;

#[derive(Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    // Each pane's title word keeps its color whatever has focus, so green
    // and red stay free for focus and needs-you.
    /// Focus, selection markers, the Projects pane.
    pub accent: Color,
    /// Key names in the help and the Pinned pane.
    pub key: Color,
    pub sessions: Color,
    pub todo: Color,
    pub fincas: Color,
    pub shell: Color,
    /// The ticker's gains and losses.
    pub up: Color,
    pub down: Color,
    pub needs_you: Color,
    pub your_turn: Color,
    pub working: Color,
    pub starting: Color,
    /// Borders, hints, ended sessions: anything that should recede.
    pub dim: Color,
    /// Secondary text a step brighter than `dim`.
    pub muted: Color,
    /// Text that should stand out from the terminal's default.
    pub bright: Color,
    pub error: Color,
    /// Warnings and passing messages.
    pub warn: Color,
    /// Text on a `needs_you` badge, and on a `warn` one.
    pub on_alert: Color,
    pub ink: Color,
    pub draft: Color,
    /// The selected row in the search results.
    pub selected_bg: Color,
    /// The cursor row, in a focused and an unfocused pane.
    pub cursor_bg: Color,
    pub cursor_dim_bg: Color,
    /// The tree in the corner.
    pub leaf: Color,
    pub sage: Color,
    pub stem: Color,
    pub fruit: Color,
    /// The dashboard's background and plain text. Grove's are the
    /// terminal's own (`Reset`); the others paint theirs.
    pub bg: Color,
    pub fg: Color,
    /// What color names in config.toml (`account_color = "blue"`) mean here.
    pub hues: Hues,
}

/// Named colors, as 0xrrggbb. Every theme has every name, using its
/// nearest shade where it has nothing closer.
#[derive(Debug, PartialEq)]
pub struct Hues {
    pub red: u32,
    pub maroon: u32,
    pub flamingo: u32,
    pub rosewater: u32,
    pub orange: u32,
    pub yellow: u32,
    pub green: u32,
    pub teal: u32,
    pub cyan: u32,
    pub sapphire: u32,
    pub blue: u32,
    pub lavender: u32,
    pub purple: u32,
    pub pink: u32,
}

impl Hues {
    /// A color by name; Catppuccin's names work in every theme.
    pub fn get(&self, name: &str) -> Option<u32> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "red" => self.red,
            "maroon" => self.maroon,
            "flamingo" => self.flamingo,
            "rosewater" => self.rosewater,
            "orange" | "peach" => self.orange,
            "yellow" => self.yellow,
            "green" => self.green,
            "teal" => self.teal,
            "cyan" | "sky" => self.cyan,
            "sapphire" => self.sapphire,
            "blue" => self.blue,
            "lavender" => self.lavender,
            "purple" | "mauve" | "magenta" => self.purple,
            "pink" => self.pink,
            _ => return None,
        })
    }
}

/// Tokyo Night's named colors; Grove's are the same but for its green.
const TOKYO_HUES: Hues = Hues {
    red: 0xf7768e,
    maroon: 0xdb4b4b,
    flamingo: 0xf7768e,
    rosewater: 0xc0caf5,
    orange: 0xff9e64,
    yellow: 0xe0af68,
    green: 0x9ece6a,
    teal: 0x73daca,
    cyan: 0x7dcfff,
    sapphire: 0x2ac3de,
    blue: 0x7aa2f7,
    lavender: 0x7aa2f7,
    purple: 0xbb9af7,
    pink: 0xbb9af7,
};

pub const GROVE: Theme = Theme {
    name: "grove",
    accent: Color::Rgb(102, 187, 106),
    key: Color::Rgb(0xe0, 0xaf, 0x68),
    sessions: Color::Rgb(0xbb, 0x9a, 0xf7),
    todo: Color::Rgb(0x7d, 0xcf, 0xff),
    fincas: Color::Rgb(0xf7, 0x76, 0x8e),
    shell: Color::Rgb(0x7a, 0xa2, 0xf7),
    up: Color::Rgb(0x9e, 0xce, 0x6a),
    down: Color::Rgb(0xf7, 0x76, 0x8e),
    needs_you: Color::Red,
    your_turn: Color::Yellow,
    working: Color::Green,
    starting: Color::Cyan,
    dim: Color::DarkGray,
    muted: Color::Gray,
    bright: Color::White,
    error: Color::Red,
    warn: Color::Yellow,
    on_alert: Color::White,
    ink: Color::Black,
    draft: Color::Cyan,
    selected_bg: Color::Rgb(0x2a, 0x2f, 0x3a),
    cursor_bg: Color::Rgb(46, 74, 52),
    cursor_dim_bg: Color::Rgb(40, 40, 40),
    leaf: Color::Rgb(0x8f, 0xb3, 0x5a),
    sage: Color::Rgb(0x6f, 0x9a, 0x6a),
    stem: Color::Rgb(0x8d, 0x6e, 0x4c),
    fruit: Color::Rgb(0x6b, 0x4a, 0x7a),
    bg: Color::Reset,
    fg: Color::Reset,
    hues: Hues {
        green: 0x66bb6a,
        ..TOKYO_HUES
    },
};

/// The Catppuccin colors a theme draws on (catppuccin.com/palette).
struct Flavor {
    rosewater: u32,
    flamingo: u32,
    pink: u32,
    mauve: u32,
    red: u32,
    maroon: u32,
    peach: u32,
    yellow: u32,
    green: u32,
    teal: u32,
    sky: u32,
    sapphire: u32,
    blue: u32,
    lavender: u32,
    text: u32,
    subtext0: u32,
    overlay0: u32,
    surface0: u32,
    surface1: u32,
    base: u32,
}

const fn rgb(c: u32) -> Color {
    Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// `a` moved `pct` percent of the way to `b`.
const fn mix(a: u32, b: u32, pct: u32) -> Color {
    const fn ch(a: u32, b: u32, shift: u32, pct: u32) -> u8 {
        let (a, b) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
        ((a * (100 - pct) + b * pct) / 100) as u8
    }
    Color::Rgb(ch(a, b, 16, pct), ch(a, b, 8, pct), ch(a, b, 0, pct))
}

const fn catppuccin(name: &'static str, f: Flavor) -> Theme {
    Theme {
        name,
        accent: rgb(f.green),
        key: rgb(f.peach),
        sessions: rgb(f.mauve),
        todo: rgb(f.sky),
        fincas: rgb(f.maroon),
        shell: rgb(f.blue),
        up: rgb(f.green),
        down: rgb(f.red),
        needs_you: rgb(f.red),
        your_turn: rgb(f.yellow),
        working: rgb(f.green),
        starting: rgb(f.teal),
        dim: rgb(f.overlay0),
        muted: rgb(f.subtext0),
        bright: rgb(f.text),
        error: rgb(f.red),
        warn: rgb(f.yellow),
        on_alert: rgb(f.base),
        ink: rgb(f.base),
        draft: rgb(f.teal),
        selected_bg: rgb(f.surface0),
        cursor_bg: mix(f.base, f.green, 22),
        cursor_dim_bg: rgb(f.surface0),
        leaf: rgb(f.green),
        sage: rgb(f.teal),
        stem: mix(f.flamingo, f.surface1, 45),
        fruit: rgb(f.mauve),
        bg: rgb(f.base),
        fg: rgb(f.text),
        hues: Hues {
            red: f.red,
            maroon: f.maroon,
            flamingo: f.flamingo,
            rosewater: f.rosewater,
            orange: f.peach,
            yellow: f.yellow,
            green: f.green,
            teal: f.teal,
            cyan: f.sky,
            sapphire: f.sapphire,
            blue: f.blue,
            lavender: f.lavender,
            purple: f.mauve,
            pink: f.pink,
        },
    }
}

pub const MOCHA: Theme = catppuccin(
    "catppuccin-mocha",
    Flavor {
        rosewater: 0xf5e0dc,
        flamingo: 0xf2cdcd,
        pink: 0xf5c2e7,
        mauve: 0xcba6f7,
        red: 0xf38ba8,
        maroon: 0xeba0ac,
        peach: 0xfab387,
        yellow: 0xf9e2af,
        green: 0xa6e3a1,
        teal: 0x94e2d5,
        sky: 0x89dceb,
        sapphire: 0x74c7ec,
        blue: 0x89b4fa,
        lavender: 0xb4befe,
        text: 0xcdd6f4,
        subtext0: 0xa6adc8,
        overlay0: 0x6c7086,
        surface0: 0x313244,
        surface1: 0x45475a,
        base: 0x1e1e2e,
    },
);

pub const MACCHIATO: Theme = catppuccin(
    "catppuccin-macchiato",
    Flavor {
        rosewater: 0xf4dbd6,
        flamingo: 0xf0c6c6,
        pink: 0xf5bde6,
        mauve: 0xc6a0f6,
        red: 0xed8796,
        maroon: 0xee99a0,
        peach: 0xf5a97f,
        yellow: 0xeed49f,
        green: 0xa6da95,
        teal: 0x8bd5ca,
        sky: 0x91d7e3,
        sapphire: 0x7dc4e4,
        blue: 0x8aadf4,
        lavender: 0xb7bdf8,
        text: 0xcad3f5,
        subtext0: 0xa5adcb,
        overlay0: 0x6e738d,
        surface0: 0x363a4f,
        surface1: 0x494d64,
        base: 0x24273a,
    },
);

pub const FRAPPE: Theme = catppuccin(
    "catppuccin-frappe",
    Flavor {
        rosewater: 0xf2d5cf,
        flamingo: 0xeebebe,
        pink: 0xf4b8e4,
        mauve: 0xca9ee6,
        red: 0xe78284,
        maroon: 0xea999c,
        peach: 0xef9f76,
        yellow: 0xe5c890,
        green: 0xa6d189,
        teal: 0x81c8be,
        sky: 0x99d1db,
        sapphire: 0x85c1dc,
        blue: 0x8caaee,
        lavender: 0xbabbf1,
        text: 0xc6d0f5,
        subtext0: 0xa5adce,
        overlay0: 0x737994,
        surface0: 0x414559,
        surface1: 0x51576d,
        base: 0x303446,
    },
);

/// Tokyo Night (github.com/folke/tokyonight.nvim), the Night style.
pub const TOKYO_NIGHT: Theme = Theme {
    name: "tokyo-night",
    accent: rgb(0x9ece6a),
    key: rgb(0xe0af68),
    sessions: rgb(0xbb9af7),
    todo: rgb(0x7dcfff),
    fincas: rgb(0xf7768e),
    shell: rgb(0x7aa2f7),
    up: rgb(0x9ece6a),
    down: rgb(0xf7768e),
    needs_you: rgb(0xf7768e),
    your_turn: rgb(0xe0af68),
    working: rgb(0x9ece6a),
    starting: rgb(0x7dcfff),
    dim: rgb(0x565f89),
    muted: rgb(0xa9b1d6),
    bright: rgb(0xc0caf5),
    error: rgb(0xdb4b4b),
    warn: rgb(0xff9e64),
    on_alert: rgb(0x1a1b26),
    ink: rgb(0x1a1b26),
    draft: rgb(0x73daca),
    selected_bg: rgb(0x292e42),
    cursor_bg: mix(0x1a1b26, 0x9ece6a, 22),
    cursor_dim_bg: rgb(0x292e42),
    leaf: rgb(0x9ece6a),
    sage: rgb(0x73daca),
    stem: mix(0xff9e64, 0x414868, 55),
    fruit: rgb(0x9d7cd8),
    bg: rgb(0x1a1b26),
    fg: rgb(0xc0caf5),
    hues: TOKYO_HUES,
};

static THEMES: [&Theme; 5] = [&GROVE, &MOCHA, &MACCHIATO, &FRAPPE, &TOKYO_NIGHT];

/// The theme called `name`; `catppuccin` alone means Mocha.
pub fn named(name: &str) -> Option<&'static Theme> {
    let name = name.trim().to_ascii_lowercase();
    let name = match name.as_str() {
        "" => "grove",
        "catppuccin" => "catppuccin-mocha",
        "tokyonight" | "tokyo night" => "tokyo-night",
        n => n,
    };
    THEMES.iter().copied().find(|t| t.name == name)
}

/// `#rrggbb`, or a color name looked up in `theme`.
pub fn lookup(color: &str, theme: &Theme) -> Option<(u8, u8, u8)> {
    let c = match color.trim().strip_prefix('#') {
        Some(h) if h.len() == 6 => u32::from_str_radix(h, 16).ok()?,
        Some(_) => return None,
        None => theme.hues.get(color)?,
    };
    Some(((c >> 16) as u8, (c >> 8) as u8, c as u8))
}

/// `color` as `#rrggbb` for tmux and the status line, names resolved in
/// the theme called `theme`. Anything Grove can't resolve passes through
/// untouched, for tmux to make what it can of.
pub fn hex(color: &str, theme: &str) -> String {
    match lookup(color, named(theme).unwrap_or(&GROVE)) {
        Some((r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
        None => color.to_string(),
    }
}

/// Every theme, in the order the picker lists them.
pub fn all() -> &'static [&'static Theme] {
    &THEMES
}

pub fn names() -> impl Iterator<Item = &'static str> {
    THEMES.iter().map(|t| t.name)
}

thread_local! {
    // Per thread so tests that switch themes don't leak into each other.
    static CURRENT: Cell<&'static Theme> = const { Cell::new(&GROVE) };
}

/// The theme the dashboard is drawing with.
pub fn pal() -> &'static Theme {
    CURRENT.with(Cell::get)
}

/// Draws with `name` from now on. False (and no change) for a name Grove
/// doesn't know.
pub fn set(name: &str) -> bool {
    match named(name) {
        Some(theme) => {
            CURRENT.with(|c| c.set(theme));
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_with_aliases_and_case() {
        assert_eq!(named("").unwrap().name, "grove");
        assert_eq!(named("Catppuccin").unwrap().name, "catppuccin-mocha");
        assert_eq!(named("tokyonight").unwrap().name, "tokyo-night");
        assert_eq!(
            named("catppuccin-frappe").unwrap().name,
            "catppuccin-frappe"
        );
        assert!(named("solarized").is_none());
    }

    #[test]
    fn mocha_uses_the_catppuccin_palette() {
        assert_eq!(MOCHA.accent, Color::Rgb(0xa6, 0xe3, 0xa1));
        assert_eq!(MOCHA.sessions, Color::Rgb(0xcb, 0xa6, 0xf7));
        assert_eq!(MOCHA.dim, Color::Rgb(0x6c, 0x70, 0x86));
    }

    #[test]
    fn color_names_resolve_per_theme_and_hex_passes_through() {
        assert_eq!(hex("blue", "catppuccin-mocha"), "#89b4fa");
        assert_eq!(hex("Peach", "catppuccin-frappe"), "#ef9f76");
        assert_eq!(hex("peach", "tokyo-night"), "#ff9e64");
        assert_eq!(hex("green", "grove"), "#66bb6a");
        assert_eq!(hex("green", "no-such-theme"), "#66bb6a");
        assert_eq!(hex("#E8A33D", "catppuccin-mocha"), "#e8a33d");
        assert_eq!(hex("colour42", "grove"), "colour42");
        assert_eq!(lookup("chartreuse", &MOCHA), None);
    }

    #[test]
    fn set_switches_this_thread_and_rejects_unknown_names() {
        assert_eq!(pal().name, "grove");
        assert!(set("catppuccin"));
        assert_eq!(pal().name, "catppuccin-mocha");
        assert!(!set("nope"));
        assert_eq!(pal().name, "catppuccin-mocha");
        set("grove");
    }
}
