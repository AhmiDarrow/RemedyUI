//! Themes as typed design tokens.

use std::collections::BTreeMap;
use std::time::Duration;

use remedy_ui_core::Color;

use crate::contrast::{CONTRAST_AA, CONTRAST_AAA, ensure_contrast};

/// Color roles. The named roles are what built-in widgets read; `custom` holds an app's own.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorRoles {
    /// The window background.
    pub bg: Color,
    /// Panels and side bars.
    pub panel: Color,
    /// Cards and raised surfaces.
    pub card: Color,
    /// Body text.
    pub text: Color,
    /// Secondary text.
    pub muted: Color,
    /// The accent: primary buttons, links, focus.
    pub accent: Color,
    /// Text and icons on the accent.
    pub on_accent: Color,
    /// Destructive actions and errors.
    pub danger: Color,
    /// Warnings.
    pub warn: Color,
    /// Success.
    pub ok: Color,
    /// Hairlines between surfaces. `None` derives it from the surfaces and text.
    pub line: Option<Color>,
    /// An app's own roles, by name.
    pub custom: BTreeMap<String, Color>,
}

/// Text sizes. Six sizes come from three, as FreeSpeak's skins define them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TypeScale {
    /// Help text under controls.
    pub small: f32,
    /// Everything people read.
    pub body: f32,
    /// Screen titles.
    pub title: f32,
}

/// Spacing: the base unit everything is a multiple of.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Spacing {
    /// The grid unit (4 dp by default); `s(n)` is n units.
    pub unit: f32,
    /// Height of buttons, inputs and rows of choices.
    pub control_height: f32,
}

impl Spacing {
    /// `n` units.
    pub fn s(&self, n: u8) -> f32 {
        self.unit * f32::from(n)
    }

    /// `v` rounded to the nearest whole unit, as skins' spacing tokens are.
    pub fn snap(&self, v: f32) -> f32 {
        if self.unit <= 0.0 { v } else { (v / self.unit).round() * self.unit }
    }
}

/// Corner radii.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Radii {
    /// Small controls.
    pub small: f32,
    /// Cards and dialogs.
    pub medium: f32,
    /// Sheets and large surfaces.
    pub large: f32,
}

/// Motion tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MotionTokens {
    /// Hover and press feedback.
    pub quick: Duration,
    /// Most transitions.
    pub standard: Duration,
    /// Large moves and page transitions.
    pub emphasized: Duration,
    /// Follow the OS or app request to reduce motion.
    pub reduced: bool,
}

/// How much space controls take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Density {
    /// Tighter rows and padding.
    Compact,
    /// The default.
    #[default]
    Comfortable,
    /// Roomier, for touch.
    Spacious,
}

impl Density {
    /// Every density, tightest first.
    pub const ALL: &'static [Density] = &[Density::Compact, Density::Comfortable, Density::Spacious];

    /// The density's name in theme files: `compact`, `comfortable` or `spacious`.
    pub fn name(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
            Density::Spacious => "spacious",
        }
    }

    /// The density a theme file names, if it names one.
    ///
    /// ```
    /// use remedy_ui_style::Density;
    ///
    /// assert_eq!(Density::from_name("spacious"), Some(Density::Spacious));
    /// assert_eq!(Density::from_name(Density::Compact.name()), Some(Density::Compact));
    /// assert_eq!(Density::from_name("roomy"), None);
    /// ```
    pub fn from_name(name: &str) -> Option<Density> {
        Density::ALL.iter().copied().find(|d| d.name() == name)
    }
}

/// A theme: every token built-in widgets read. Themes are data: an app maps its own format
/// (FreeSpeak's `.fskin`, W3C design-token files) onto this.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Theme {
    /// A name for menus.
    pub name: String,
    /// Colors.
    pub colors: ColorRoles,
    /// Families for text, in fallback order.
    pub font_families: Vec<String>,
    /// Families for numbers, clocks and codes (same-width figures).
    pub mono_families: Vec<String>,
    /// Text sizes.
    pub type_scale: TypeScale,
    /// Spacing.
    pub spacing: Spacing,
    /// Corner radii.
    pub radii: Radii,
    /// Motion.
    pub motion: MotionTokens,
    /// Density.
    pub density: Density,
    /// Hairline and border width.
    pub stroke: f32,
    /// Files the theme brings: a background image and its own fonts.
    pub assets: ThemeAssets,
}

/// Files a theme brings with it. They are references, not loaded data: a path (relative to
/// the folder of the theme file), or the bytes inline as a `data:` URI. Whoever applies the
/// theme loads them through the media and font safety paths: images are decoded only at the
/// size shown (in the sandboxed decoder where the app uses it), fonts are checked by the
/// font engine before use, and a theme from someone else carries none unless the person
/// allows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ThemeAssets {
    /// An image drawn behind each window's content, covering it.
    pub background: Option<String>,
    /// Font files by the family name the theme's font tokens use for them: a font token
    /// naming `Ocean` uses the faces in the file given for `Ocean`.
    pub fonts: BTreeMap<String, String>,
}

/// Tokens computed from a theme once per change, not per widget.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DerivedTokens {
    /// Body text, balanced to at least 7:1 on the background.
    pub text: Color,
    /// Secondary text (labels, help under controls, unselected options), balanced to at
    /// least 7:1 on the background, panels and cards: quieter than body text, never faint.
    pub muted: Color,
    /// Text on the accent, balanced to at least 4.5:1.
    pub on_accent: Color,
    /// Fill for something hovered.
    pub hover: Color,
    /// Fill for something pressed.
    pub pressed: Color,
    /// Fill for something selected.
    pub selected: Color,
    /// Fill behind marked text (what a search found).
    pub mark: Color,
    /// Accent fill when hovered.
    pub accent_hover: Color,
    /// Accent fill when pressed.
    pub accent_pressed: Color,
    /// Hairlines.
    pub line: Color,
    /// The focus ring.
    pub focus: Color,
    /// True when the background is dark.
    pub dark: bool,
    /// Six text sizes: tiny, small, body, heading, title, display.
    pub sizes: [f32; 6],
    /// Control height after density.
    pub control_height: f32,
}

impl Theme {
    /// Dark Forest, the default Remedy theme: dark green-greys with a moss accent. The colors
    /// are the `forest` palette of RemedyAI's `desktop/src/themes.ts`, which Portal, Hearth and
    /// FreeSpeak's skins share. Text on the accent is not set there; it uses the primary text
    /// color, balanced for contrast by [`Theme::derive`].
    pub fn dark_forest() -> Theme {
        let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::BLACK);
        Theme {
            name: "Dark Forest".into(),
            colors: ColorRoles {
                bg: hex("#0a0e0b"),
                panel: hex("#121812"),
                card: hex("#1a221c"),
                text: hex("#e6ebe7"),
                muted: hex("#9aa89e"),
                accent: hex("#4d7a5a"),
                on_accent: hex("#e6ebe7"),
                danger: hex("#b87a7a"),
                warn: hex("#a89058"),
                ok: hex("#5a8f6a"),
                line: Some(hex("#2a352c")),
                custom: BTreeMap::new(),
            },
            font_families: vec!["Remedy Sans UI".into()],
            mono_families: vec!["AhmiPrint".into(), "Remedy Sans UI".into()],
            type_scale: TypeScale { small: 12.0, body: 13.0, title: 18.0 },
            spacing: Spacing { unit: 4.0, control_height: 28.0 },
            radii: Radii { small: 3.0, medium: 6.0, large: 12.0 },
            motion: MotionTokens {
                quick: Duration::from_millis(100),
                standard: Duration::from_millis(200),
                emphasized: Duration::from_millis(350),
                reduced: false,
            },
            density: Density::Comfortable,
            stroke: 1.0,
            assets: ThemeAssets::default(),
        }
    }

    /// Computes the derived tokens.
    pub fn derive(&self) -> DerivedTokens {
        let c = &self.colors;
        let dark = c.bg.luminance() < 0.18;
        let toward = if dark { Color::WHITE } else { Color::BLACK };
        let tint = |base: Color, amount: f32| base.mix_oklab(toward, amount);
        let accent_toward = if c.accent.luminance() < 0.18 { Color::WHITE } else { Color::BLACK };
        let line = c.line.unwrap_or_else(|| c.card.mix_oklab(c.text, 0.12));
        let ts = self.type_scale;
        let control_height = match self.density {
            Density::Compact => self.spacing.control_height - self.spacing.unit,
            Density::Comfortable => self.spacing.control_height,
            Density::Spacious => self.spacing.control_height + 2.0 * self.spacing.unit,
        };
        // Readable on every surface text sits on: cards, panels and the background. The
        // background comes last, so when no color suits them all (light cards in a dark
        // theme) the background, where most text sits, keeps it.
        let readable = |fg: Color, ratio: f32| {
            let surfaces = [c.card, c.panel, c.bg];
            let once = surfaces.iter().fold(fg, |f, s| ensure_contrast(f, *s, ratio));
            surfaces.iter().fold(once, |f, s| ensure_contrast(f, *s, ratio))
        };
        DerivedTokens {
            text: readable(c.text, CONTRAST_AAA),
            muted: readable(c.muted, CONTRAST_AAA),
            on_accent: ensure_contrast(c.on_accent, c.accent, CONTRAST_AA),
            hover: tint(c.card, 0.06),
            pressed: tint(c.card, 0.12),
            selected: c.card.mix_oklab(c.accent, 0.25),
            mark: c.card.mix_oklab(c.warn, 0.35),
            accent_hover: c.accent.mix_oklab(accent_toward, 0.08),
            accent_pressed: c.accent.mix_oklab(accent_toward, 0.16),
            line,
            focus: ensure_contrast(c.accent, c.bg, 3.0),
            dark,
            sizes: [ts.small - 1.0, ts.small, ts.body, ts.body + 2.0, ts.title, ts.title + 6.0],
            control_height,
        }
    }
}
