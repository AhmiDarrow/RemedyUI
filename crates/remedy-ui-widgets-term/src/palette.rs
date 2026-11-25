//! The terminal's colors come from the theme. The 16 ANSI colors are theme tokens
//! `term-0` … `term-15`, the default foreground and background `term-fg` and `term-bg`, and
//! the cursor `term-cursor`. A theme that does not set them gets colors derived from its own
//! roles, balanced for contrast against the terminal's background.

use remedy_ui_core::Color;
use remedy_ui_core::style::{Theme, ensure_contrast};

use crate::cell::TermColor;
use crate::term::xterm_color;

/// The terminal's default background under `theme`: its `term-bg` token, else the window
/// background.
pub fn background(theme: &Theme) -> Color {
    theme.colors.custom.get("term-bg").copied().unwrap_or(theme.colors.bg)
}

/// The terminal's default foreground under `theme`: its `term-fg` token, else body text.
pub fn foreground(theme: &Theme) -> Color {
    theme.colors.custom.get("term-fg").copied().unwrap_or_else(|| theme.derive().text)
}

/// The cursor's color: the `term-cursor` token, else the accent made visible on the
/// background.
pub fn cursor(theme: &Theme) -> Color {
    let bg = background(theme);
    theme
        .colors
        .custom
        .get("term-cursor")
        .copied()
        .unwrap_or_else(|| ensure_contrast(theme.colors.accent, bg, 3.0))
}

fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap_or(Color::BLACK)
}

/// ANSI color `i` (0–15) under `theme`: its `term-<i>` token, else derived from the theme.
/// Colors past 15 are xterm's cube and grays.
pub fn ansi(theme: &Theme, i: u8) -> Color {
    if let Some(c) = theme.colors.custom.get(&format!("term-{i}")) {
        return *c;
    }
    if i > 15 {
        let (r, g, b) = xterm_color(i);
        return Color::rgb8(r, g, b);
    }
    let d = theme.derive();
    let c = &theme.colors;
    let bg = background(theme);
    let fg = foreground(theme);
    // The six hues, from the theme's own roles where it has them; blue, magenta and cyan
    // lean toward the accent so they sit in the palette.
    let hue = |k: u8| -> Color {
        match k {
            1 => c.danger,
            2 => c.ok,
            3 => c.warn,
            4 => hex("#4f7fd6").mix_oklab(c.accent, 0.2),
            5 => hex("#b066c4").mix_oklab(c.accent, 0.2),
            _ => hex("#3fa9b0").mix_oklab(c.accent, 0.2),
        }
    };
    match i {
        0 => {
            if d.dark {
                bg.mix_oklab(fg, 0.18)
            } else {
                fg
            }
        }
        7 => {
            if d.dark {
                ensure_contrast(d.muted, bg, 4.5)
            } else {
                bg.mix_oklab(fg, 0.3)
            }
        }
        8 => {
            if d.dark {
                ensure_contrast(bg.mix_oklab(fg, 0.45), bg, 3.0)
            } else {
                ensure_contrast(d.muted, bg, 3.0)
            }
        }
        15 => {
            if d.dark {
                fg
            } else {
                bg.mix_oklab(fg, 0.1)
            }
        }
        1..=6 => ensure_contrast(hue(i), bg, 4.5),
        _ => {
            // The bright six: their hue lifted toward the text color.
            let base = hue(i - 8);
            ensure_contrast(base.mix_oklab(fg, 0.3), bg, 4.5)
        }
    }
}

/// The colors a terminal draws with, resolved from the theme and what the program changed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Palette {
    pub(crate) colors: Vec<Color>,
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) cursor: Color,
    /// Text under a block cursor.
    pub(crate) cursor_text: Color,
    pub(crate) selection: Color,
    pub(crate) link: Color,
}

impl Palette {
    /// The palette under `theme`, with `fg` and `bg` the terminal element's own colors, and
    /// the program's changes (`overrides`: 0–255, then 256 fg, 257 bg, 258 cursor).
    pub(crate) fn new(theme: &Theme, fg: Color, bg: Color, overrides: &[(u16, (u8, u8, u8))]) -> Palette {
        let mut colors: Vec<Color> = (0..=255u8).map(|i| ansi(theme, i)).collect();
        let mut p = Palette {
            fg,
            bg,
            cursor: cursor(theme),
            cursor_text: bg,
            selection: theme.derive().selected,
            link: ensure_contrast(theme.colors.accent, bg, 4.5),
            colors: Vec::new(),
        };
        for (slot, (r, g, b)) in overrides {
            let c = Color::rgb8(*r, *g, *b);
            match slot {
                256 => p.fg = c,
                257 => p.bg = c,
                258 => p.cursor = c,
                i => {
                    if let Some(x) = colors.get_mut(usize::from(*i)) {
                        *x = c;
                    }
                }
            }
        }
        p.cursor_text = if p.cursor.contrast(p.bg) >= p.cursor.contrast(p.fg) { p.bg } else { p.fg };
        p.colors = colors;
        p
    }

    /// `c` as a color to draw, `default` standing for the terminal's default.
    pub(crate) fn resolve(&self, c: TermColor, default: Color) -> Color {
        match c {
            TermColor::Default => default,
            TermColor::Indexed(i) => self.colors.get(usize::from(i)).copied().unwrap_or(default),
            TermColor::Rgb(r, g, b) => Color::rgb8(r, g, b),
        }
    }

    /// The colors as RGB, for answering the program's color queries.
    pub(crate) fn to_rgb(&self) -> crate::term::Colors {
        let rgb = |c: Color| {
            let [r, g, b, _] = c.to_rgba8();
            (r, g, b)
        };
        crate::term::Colors {
            palette: self.colors.iter().map(|c| rgb(*c)).collect(),
            fg: rgb(self.fg),
            bg: rgb(self.bg),
            cursor: rgb(self.cursor),
        }
    }
}
