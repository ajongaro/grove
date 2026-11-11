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

pub const LATTE: Theme = catppuccin(
    "catppuccin-latte",
    Flavor {
        rosewater: 0xdc8a78,
        flamingo: 0xdd7878,
        pink: 0xea76cb,
        mauve: 0x8839ef,
        red: 0xd20f39,
        maroon: 0xe64553,
        peach: 0xfe640b,
        yellow: 0xdf8e1d,
        green: 0x40a02b,
        teal: 0x179299,
        sky: 0x04a5e5,
        sapphire: 0x209fb5,
        blue: 0x1e66f5,
        lavender: 0x7287fd,
        text: 0x4c4f69,
        subtext0: 0x6c6f85,
        overlay0: 0x9ca0b0,
        surface0: 0xccd0da,
        surface1: 0xbcc0cc,
        base: 0xeff1f5,
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

/// Gruvbox (dark, medium contrast): github.com/morhetz/gruvbox.
pub const GRUVBOX: Theme = Theme {
    name: "gruvbox",
    accent: rgb(0xb8bb26),
    key: rgb(0xfabd2f),
    sessions: rgb(0xd3869b),
    todo: rgb(0x8ec07c),
    fincas: rgb(0xfe8019),
    shell: rgb(0x83a598),
    up: rgb(0xb8bb26),
    down: rgb(0xfb4934),
    needs_you: rgb(0xfb4934),
    your_turn: rgb(0xfabd2f),
    working: rgb(0xb8bb26),
    starting: rgb(0x8ec07c),
    dim: rgb(0x7c6f64),
    muted: rgb(0xa89984),
    bright: rgb(0xebdbb2),
    error: rgb(0xfb4934),
    warn: rgb(0xfe8019),
    on_alert: rgb(0x282828),
    ink: rgb(0x282828),
    draft: rgb(0x8ec07c),
    selected_bg: rgb(0x3c3836),
    cursor_bg: mix(0x282828, 0xb8bb26, 22),
    cursor_dim_bg: rgb(0x3c3836),
    leaf: rgb(0xb8bb26),
    sage: rgb(0x8ec07c),
    stem: mix(0xfe8019, 0x504945, 55),
    fruit: rgb(0xd3869b),
    bg: rgb(0x282828),
    fg: rgb(0xebdbb2),
    hues: Hues {
        red: 0xfb4934,
        maroon: 0xcc241d,
        flamingo: 0xd3869b,
        rosewater: 0xebdbb2,
        orange: 0xfe8019,
        yellow: 0xfabd2f,
        green: 0xb8bb26,
        teal: 0x8ec07c,
        cyan: 0x8ec07c,
        sapphire: 0x83a598,
        blue: 0x83a598,
        lavender: 0x83a598,
        purple: 0xd3869b,
        pink: 0xd3869b,
    },
};

/// Dracula: draculatheme.com/contribute. It has no blue; `blue` is its purple.
pub const DRACULA: Theme = Theme {
    name: "dracula",
    accent: rgb(0xbd93f9),
    key: rgb(0xf1fa8c),
    sessions: rgb(0xff79c6),
    todo: rgb(0x8be9fd),
    fincas: rgb(0xffb86c),
    shell: rgb(0x50fa7b),
    up: rgb(0x50fa7b),
    down: rgb(0xff5555),
    needs_you: rgb(0xff5555),
    your_turn: rgb(0xf1fa8c),
    working: rgb(0x50fa7b),
    starting: rgb(0x8be9fd),
    dim: rgb(0x6272a4),
    muted: mix(0xf8f8f2, 0x6272a4, 40),
    bright: rgb(0xf8f8f2),
    error: rgb(0xff5555),
    warn: rgb(0xffb86c),
    on_alert: rgb(0x282a36),
    ink: rgb(0x282a36),
    draft: rgb(0x8be9fd),
    selected_bg: rgb(0x44475a),
    cursor_bg: mix(0x282a36, 0xbd93f9, 25),
    cursor_dim_bg: rgb(0x44475a),
    leaf: rgb(0x50fa7b),
    sage: rgb(0x8be9fd),
    stem: mix(0xffb86c, 0x44475a, 55),
    fruit: rgb(0xff79c6),
    bg: rgb(0x282a36),
    fg: rgb(0xf8f8f2),
    hues: Hues {
        red: 0xff5555,
        maroon: 0xff5555,
        flamingo: 0xff79c6,
        rosewater: 0xf8f8f2,
        orange: 0xffb86c,
        yellow: 0xf1fa8c,
        green: 0x50fa7b,
        teal: 0x8be9fd,
        cyan: 0x8be9fd,
        sapphire: 0x8be9fd,
        blue: 0xbd93f9,
        lavender: 0xbd93f9,
        purple: 0xbd93f9,
        pink: 0xff79c6,
    },
};

/// A theme from a palette's colors, each Grove role taking the usual one:
/// green for work and focus (`accent` unless the palette says otherwise),
/// red for needs-you, yellow for your turn.
struct Palette {
    bg: u32,
    /// The selected row; a step off `bg`.
    surface: u32,
    dim: u32,
    muted: u32,
    fg: u32,
    bright: u32,
    red: u32,
    orange: u32,
    yellow: u32,
    green: u32,
    teal: u32,
    cyan: u32,
    blue: u32,
    purple: u32,
    pink: u32,
    accent: u32,
    sessions: u32,
    fincas: u32,
}

const fn from_palette(name: &'static str, p: Palette) -> Theme {
    Theme {
        name,
        accent: rgb(p.accent),
        key: rgb(p.yellow),
        sessions: rgb(p.sessions),
        todo: rgb(p.cyan),
        fincas: rgb(p.fincas),
        shell: rgb(p.blue),
        up: rgb(p.green),
        down: rgb(p.red),
        needs_you: rgb(p.red),
        your_turn: rgb(p.yellow),
        working: rgb(p.green),
        starting: rgb(p.cyan),
        dim: rgb(p.dim),
        muted: rgb(p.muted),
        bright: rgb(p.bright),
        error: rgb(p.red),
        warn: rgb(p.orange),
        on_alert: rgb(p.bg),
        ink: rgb(p.bg),
        draft: rgb(p.teal),
        selected_bg: rgb(p.surface),
        cursor_bg: mix(p.bg, p.accent, 22),
        cursor_dim_bg: rgb(p.surface),
        leaf: rgb(p.green),
        sage: rgb(p.teal),
        stem: mix(p.orange, p.surface, 55),
        fruit: rgb(p.purple),
        bg: rgb(p.bg),
        fg: rgb(p.fg),
        hues: Hues {
            red: p.red,
            maroon: p.red,
            flamingo: p.pink,
            rosewater: p.fg,
            orange: p.orange,
            yellow: p.yellow,
            green: p.green,
            teal: p.teal,
            cyan: p.cyan,
            sapphire: p.blue,
            blue: p.blue,
            lavender: p.purple,
            purple: p.purple,
            pink: p.pink,
        },
    }
}

/// Gruvbox light, in its darker "faded" accents so they read on cream.
pub const GRUVBOX_LIGHT: Theme = from_palette(
    "gruvbox-light",
    Palette {
        bg: 0xfbf1c7,
        surface: 0xebdbb2,
        dim: 0xa89984,
        muted: 0x7c6f64,
        fg: 0x3c3836,
        bright: 0x282828,
        red: 0x9d0006,
        orange: 0xaf3a03,
        yellow: 0xb57614,
        green: 0x79740e,
        teal: 0x427b58,
        cyan: 0x427b58,
        blue: 0x076678,
        purple: 0x8f3f71,
        pink: 0x8f3f71,
        accent: 0x79740e,
        sessions: 0x8f3f71,
        fincas: 0xaf3a03,
    },
);

/// Nord: nordtheme.com/docs/colors-and-palettes.
pub const NORD: Theme = from_palette(
    "nord",
    Palette {
        bg: 0x2e3440,
        surface: 0x3b4252,
        dim: 0x616e88,
        muted: 0xd8dee9,
        fg: 0xd8dee9,
        bright: 0xeceff4,
        red: 0xbf616a,
        orange: 0xd08770,
        yellow: 0xebcb8b,
        green: 0xa3be8c,
        teal: 0x8fbcbb,
        cyan: 0x88c0d0,
        blue: 0x81a1c1,
        purple: 0xb48ead,
        pink: 0xb48ead,
        accent: 0x88c0d0,
        sessions: 0xb48ead,
        fincas: 0xd08770,
    },
);

/// Rosé Pine (rosepinetheme.com/palette). It has no green: foam (its
/// cyan) stands in for work, and rose leads.
/// One Rosé Pine variant's palette, by its own names.
struct RosePine {
    base: u32,
    overlay: u32,
    muted: u32,
    subtle: u32,
    text: u32,
    love: u32,
    gold: u32,
    rose: u32,
    pine: u32,
    foam: u32,
    iris: u32,
}

const fn rose_pine(name: &'static str, r: RosePine) -> Theme {
    from_palette(
        name,
        Palette {
            bg: r.base,
            surface: r.overlay,
            dim: r.muted,
            muted: r.subtle,
            fg: r.text,
            bright: r.text,
            red: r.love,
            orange: r.rose,
            yellow: r.gold,
            green: r.foam,
            teal: r.pine,
            cyan: r.foam,
            blue: r.pine,
            purple: r.iris,
            pink: r.rose,
            accent: r.rose,
            sessions: r.iris,
            fincas: r.gold,
        },
    )
}

pub const ROSE_PINE: Theme = rose_pine(
    "rose-pine",
    RosePine {
        base: 0x191724,
        overlay: 0x26233a,
        muted: 0x6e6a86,
        subtle: 0x908caa,
        text: 0xe0def4,
        love: 0xeb6f92,
        gold: 0xf6c177,
        rose: 0xebbcba,
        pine: 0x31748f,
        foam: 0x9ccfd8,
        iris: 0xc4a7e7,
    },
);
pub const ROSE_PINE_MOON: Theme = rose_pine(
    "rose-pine-moon",
    RosePine {
        base: 0x232136,
        overlay: 0x393552,
        muted: 0x6e6a86,
        subtle: 0x908caa,
        text: 0xe0def4,
        love: 0xeb6f92,
        gold: 0xf6c177,
        rose: 0xea9a97,
        pine: 0x3e8fb0,
        foam: 0x9ccfd8,
        iris: 0xc4a7e7,
    },
);
pub const ROSE_PINE_DAWN: Theme = rose_pine(
    "rose-pine-dawn",
    RosePine {
        base: 0xfaf4ed,
        overlay: 0xf2e9e1,
        muted: 0x9893a5,
        subtle: 0x797593,
        text: 0x575279,
        love: 0xb4637a,
        gold: 0xea9d34,
        rose: 0xd7827e,
        pine: 0x286983,
        foam: 0x56949f,
        iris: 0x907aa9,
    },
);

/// Everforest dark, medium: github.com/sainnhe/everforest.
pub const EVERFOREST: Theme = from_palette(
    "everforest",
    Palette {
        bg: 0x2d353b,
        surface: 0x3d484d,
        dim: 0x7a8478,
        muted: 0x9da9a0,
        fg: 0xd3c6aa,
        bright: 0xd3c6aa,
        red: 0xe67e80,
        orange: 0xe69875,
        yellow: 0xdbbc7f,
        green: 0xa7c080,
        teal: 0x83c092,
        cyan: 0x83c092,
        blue: 0x7fbbb3,
        purple: 0xd699b6,
        pink: 0xd699b6,
        accent: 0xa7c080,
        sessions: 0xd699b6,
        fincas: 0xe69875,
    },
);

/// Kanagawa wave: github.com/rebelot/kanagawa.nvim.
pub const KANAGAWA: Theme = from_palette(
    "kanagawa",
    Palette {
        bg: 0x1f1f28,
        surface: 0x2a2a37,
        dim: 0x727169,
        muted: 0xc8c093,
        fg: 0xdcd7ba,
        bright: 0xdcd7ba,
        red: 0xe46876,
        orange: 0xffa066,
        yellow: 0xe6c384,
        green: 0x98bb6c,
        teal: 0x7aa89f,
        cyan: 0x7fb4ca,
        blue: 0x7e9cd8,
        purple: 0x957fb8,
        pink: 0xd27e99,
        accent: 0x98bb6c,
        sessions: 0x957fb8,
        fincas: 0xffa066,
    },
);

/// Atom's One Dark.
pub const ONE_DARK: Theme = from_palette(
    "one-dark",
    Palette {
        bg: 0x282c34,
        surface: 0x3e4451,
        dim: 0x5c6370,
        muted: 0x9da5b4,
        fg: 0xabb2bf,
        bright: 0xd7dae0,
        red: 0xe06c75,
        orange: 0xd19a66,
        yellow: 0xe5c07b,
        green: 0x98c379,
        teal: 0x56b6c2,
        cyan: 0x56b6c2,
        blue: 0x61afef,
        purple: 0xc678dd,
        pink: 0xc678dd,
        accent: 0x98c379,
        sessions: 0xc678dd,
        fincas: 0xd19a66,
    },
);

/// Solarized (ethanschoonover.com/solarized): the same accents on dark or
/// light, the base tones swapped.
const fn solarized(
    name: &'static str,
    bg: u32,
    surface: u32,
    dim: u32,
    fg: u32,
    bright: u32,
) -> Theme {
    from_palette(
        name,
        Palette {
            bg,
            surface,
            dim,
            muted: fg,
            fg,
            bright,
            red: 0xdc322f,
            orange: 0xcb4b16,
            yellow: 0xb58900,
            green: 0x859900,
            teal: 0x2aa198,
            cyan: 0x2aa198,
            blue: 0x268bd2,
            purple: 0x6c71c4,
            pink: 0xd33682,
            accent: 0x859900,
            sessions: 0x6c71c4,
            fincas: 0xcb4b16,
        },
    )
}

pub const SOLARIZED_DARK: Theme = solarized(
    "solarized-dark",
    0x002b36,
    0x073642,
    0x586e75,
    0x839496,
    0x93a1a1,
);
pub const SOLARIZED_LIGHT: Theme = solarized(
    "solarized-light",
    0xfdf6e3,
    0xeee8d5,
    0x93a1a1,
    0x657b83,
    0x586e75,
);

static THEMES: [&Theme; 18] = [
    &GROVE,
    &MOCHA,
    &MACCHIATO,
    &FRAPPE,
    &LATTE,
    &TOKYO_NIGHT,
    &GRUVBOX,
    &GRUVBOX_LIGHT,
    &DRACULA,
    &NORD,
    &ROSE_PINE,
    &ROSE_PINE_MOON,
    &ROSE_PINE_DAWN,
    &EVERFOREST,
    &KANAGAWA,
    &ONE_DARK,
    &SOLARIZED_DARK,
    &SOLARIZED_LIGHT,
];

/// The theme called `name`; `catppuccin` alone means Mocha.
pub fn named(name: &str) -> Option<&'static Theme> {
    let name = name.trim().to_ascii_lowercase();
    let name = match name.as_str() {
        "" => "grove",
        "catppuccin" => "catppuccin-mocha",
        "tokyonight" | "tokyo night" => "tokyo-night",
        "gruvbox-dark" | "gruvbox dark" => "gruvbox",
        "latte" => "catppuccin-latte",
        "rosepine" | "rose-pine-main" | "rosé-pine" | "rosé pine" | "rose pine" => "rose-pine",
        "everforest-dark" => "everforest",
        "kanagawa-wave" => "kanagawa",
        "onedark" | "one dark" => "one-dark",
        "solarized" => "solarized-dark",
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
        assert_eq!(named("solarized").unwrap().name, "solarized-dark");
        assert_eq!(named("Rosé Pine").unwrap().name, "rose-pine");
        assert_eq!(named("onedark").unwrap().name, "one-dark");
        assert_eq!(named("latte").unwrap().name, "catppuccin-latte");
        assert!(named("monokai").is_none());
        // Every theme is reachable by its own name, and names are unique.
        let names: std::collections::BTreeSet<_> = names().collect();
        assert_eq!(names.len(), all().len());
        assert!(all().iter().all(|t| named(t.name).unwrap().name == t.name));
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
        assert_eq!(hex("blue", "gruvbox"), "#83a598");
        assert_eq!(hex("blue", "dracula"), "#bd93f9");
        assert_eq!(named("Gruvbox Dark").unwrap().name, "gruvbox");
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
