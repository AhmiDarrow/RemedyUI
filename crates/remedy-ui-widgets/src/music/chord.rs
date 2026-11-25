//! Chord diagrams: the box with strings, frets, fingers, barres, and open and muted strings.

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::layout::Length;
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::{CornerRadii, Point, Rect, Vec2};
use remedy_ui_engine::paint::{Brush, Item, Path, Stroke, StrokeCap};
use remedy_ui_engine::view::{El, TextPaint, Ui};

/// What a string does in a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StringState {
    /// Not played (an X above the nut).
    Muted,
    /// Played open (an O above the nut).
    Open,
    /// Pressed at a fret, with a finger if the diagram shows fingering.
    Fretted {
        /// The fret as numbered on the neck (not relative to the diagram's top).
        fret: u32,
        /// The finger: 1 index, 2 middle, 3 ring, 4 little, 5 thumb.
        finger: Option<u8>,
    },
}

/// A finger laid across several strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Barre {
    /// The fret as numbered on the neck.
    pub fret: u32,
    /// The lowest string it covers (0 is the lowest string).
    pub from: usize,
    /// The highest string it covers.
    pub to: usize,
    /// The finger.
    pub finger: Option<u8>,
}

/// A chord's fingering.
#[derive(Clone, Debug, PartialEq)]
pub struct ChordShape {
    /// The chord's name ("Am7"), if it has one.
    pub name: Option<String>,
    /// What each string does, from the lowest string (drawn at the left) up.
    pub strings: Vec<StringState>,
    /// The fret number of the diagram's first row: 1 draws the nut, higher numbers a plain
    /// line and a "3fr" label.
    pub base_fret: u32,
    /// Barres.
    pub barres: Vec<Barre>,
    /// How many fret rows to draw; `None` takes enough for the chord, at least four.
    pub rows: Option<u32>,
    /// The strings that sound the chord's root: their dot (or their O) is drawn in the root
    /// colour.
    pub roots: Vec<usize>,
    /// What each string's dot says in place of its finger number (a scale degree, "R"), by
    /// string; `None`, or no entry, leaves the finger.
    pub labels: Vec<Option<String>>,
    /// A name under each string (the tuning, low to high); empty for no row.
    pub string_names: Vec<String>,
}

impl ChordShape {
    /// A chord of `strings`, with the diagram's top row chosen to show it: the nut if every
    /// fret is within the first four, else the lowest fret used.
    pub fn new(name: Option<&str>, strings: Vec<StringState>) -> ChordShape {
        let mut c = ChordShape {
            name: name.map(str::to_owned),
            strings,
            base_fret: 1,
            barres: Vec::new(),
            rows: None,
            roots: Vec::new(),
            labels: Vec::new(),
            string_names: Vec::new(),
        };
        c.base_fret = c.suggested_base_fret();
        c
    }

    /// Adds a barre (and keeps the top row where the chord fits).
    pub fn with_barre(mut self, barre: Barre) -> ChordShape {
        self.barres.push(barre);
        self.base_fret = self.suggested_base_fret();
        self
    }

    /// Marks the strings that sound the root.
    pub fn with_roots(mut self, strings: Vec<usize>) -> ChordShape {
        self.roots = strings;
        self
    }

    /// Writes `labels` on the dots, by string, in place of finger numbers.
    pub fn with_labels(mut self, labels: Vec<Option<String>>) -> ChordShape {
        self.labels = labels;
        self
    }

    /// Names the strings under the box, lowest first.
    pub fn with_string_names(mut self, names: Vec<String>) -> ChordShape {
        self.string_names = names;
        self
    }

    /// Sets the number of the first row.
    pub fn with_base_fret(mut self, base_fret: u32) -> ChordShape {
        self.base_fret = base_fret.max(1);
        self
    }

    fn fretted(&self) -> impl Iterator<Item = u32> + '_ {
        let strings = self.strings.iter().filter_map(|s| match s {
            StringState::Fretted { fret, .. } => Some(*fret),
            _ => None,
        });
        strings.chain(self.barres.iter().map(|b| b.fret))
    }

    /// The first row that shows the chord: 1 if every fret is within the first four.
    pub fn suggested_base_fret(&self) -> u32 {
        match (self.fretted().min(), self.fretted().max()) {
            (Some(lo), Some(hi)) if hi > 4 => lo.max(1),
            _ => 1,
        }
    }

    /// The rows the diagram draws.
    pub fn row_count(&self) -> u32 {
        let top = self.fretted().max().map_or(0, |m| (m + 1).saturating_sub(self.base_fret));
        self.rows.unwrap_or_else(|| top.max(4)).max(1)
    }
}

/// How a [`chord_diagram`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChordOptions {
    /// Scale of the whole diagram; 1 is four theme units per string gap and a bit more per fret.
    pub scale: f32,
    /// Show the chord's name above the box.
    pub show_name: bool,
    /// Show finger numbers on the dots.
    pub fingers: bool,
    /// Mirror the box for left-handed players (the lowest string at the right).
    pub left_handed: bool,
}

impl ChordOptions {
    /// Sets `scale`: scale of the whole diagram; 1 is four theme units per string gap and a bit more per fret.
    pub fn scale(mut self, value: f32) -> Self {
        self.scale = value;
        self
    }

    /// Sets `show_name`: show the chord's name above the box.
    pub fn show_name(mut self, value: bool) -> Self {
        self.show_name = value;
        self
    }

    /// Sets `fingers`: show finger numbers on the dots.
    pub fn fingers(mut self, value: bool) -> Self {
        self.fingers = value;
        self
    }

    /// Sets `left_handed`: mirror the box for left-handed players (the lowest string at the right).
    pub fn left_handed(mut self, value: bool) -> Self {
        self.left_handed = value;
        self
    }
}

impl Default for ChordOptions {
    fn default() -> Self {
        ChordOptions { scale: 1.0, show_name: true, fingers: true, left_handed: false }
    }
}

/// How a chord is said aloud in English: each string from the lowest, barres, and where the
/// diagram starts ([`describe_chord_in`] for another language).
pub fn describe_chord(chord: &ChordShape) -> String {
    describe_chord_in(chord, "en")
}

/// How a chord is said aloud in `locale`: each string from the lowest, barres, and where the
/// diagram starts.
pub fn describe_chord_in(chord: &ChordShape, locale: &str) -> String {
    use crate::music::say;
    let n = chord.strings.len();
    let mut parts: Vec<String> = Vec::new();
    for (i, s) in chord.strings.iter().enumerate() {
        let string = ("string", (n - i).into());
        parts.push(match s {
            StringState::Muted => say(locale, "chord-string-muted", &[string]),
            StringState::Open => say(locale, "chord-string-open", &[string]),
            StringState::Fretted { fret, finger: Some(f) } => say(
                locale,
                "chord-string-fret-finger",
                &[string, ("fret", (*fret).into()), ("finger", u32::from(*f).into())],
            ),
            StringState::Fretted { fret, finger: None } => {
                say(locale, "chord-string-fret", &[string, ("fret", (*fret).into())])
            }
        });
    }
    for b in &chord.barres {
        let args = [
            ("fret", b.fret.into()),
            ("from", n.saturating_sub(b.from).into()),
            ("to", n.saturating_sub(b.to).into()),
        ];
        parts.push(match b.finger {
            Some(f) => {
                let mut args = args.to_vec();
                args.push(("finger", u32::from(f).into()));
                say(locale, "chord-barre-finger", &args)
            }
            None => say(locale, "chord-barre", &args),
        });
    }
    if chord.base_fret > 1 {
        parts.push(say(locale, "chord-starts-at", &[("fret", chord.base_fret.into())]));
    }
    crate::music::list(locale, &parts)
}

/// The size of a diagram in logical pixels for a chord and options, with spacing from the
/// theme unit.
pub fn diagram_size(chord: &ChordShape, options: &ChordOptions, unit: f32) -> (f32, f32) {
    let m = Metrics::new(chord, options, unit);
    (m.width, m.height)
}

struct Metrics {
    n: usize,
    rows: u32,
    cell_w: f32,
    cell_h: f32,
    pad: f32,
    head: f32,
    marks: f32,
    width: f32,
    height: f32,
}

impl Metrics {
    fn new(chord: &ChordShape, options: &ChordOptions, unit: f32) -> Metrics {
        let s = options.scale.max(0.25);
        let n = chord.strings.len().max(2);
        let rows = chord.row_count();
        let cell_w = unit * 4.5 * s;
        let cell_h = unit * 5.5 * s;
        let pad = unit * 3.0 * s;
        let head = if options.show_name && chord.name.is_some() { unit * 6.0 * s } else { unit * s };
        let marks = unit * 4.0 * s;
        // Room for the "5fr" label beside the first row.
        let side = if chord.base_fret > 1 { unit * 8.0 * s } else { pad };
        let width = pad + (n - 1) as f32 * cell_w + side;
        let names = if chord.string_names.is_empty() { 0.0 } else { unit * 4.0 * s };
        let height = head + marks + rows as f32 * cell_h + names + pad;
        Metrics { n, rows, cell_w, cell_h, pad, head, marks, width, height }
    }
}

/// A chord diagram for the shape in `chord`: the strings as vertical lines (lowest at the
/// left), frets across, a dot for each fretted string with its finger, barres, an O over open
/// strings and an X over muted ones, the name above and a fret label when it starts above the
/// nut. It redraws when the signal changes and is described to screen readers string by string
/// ([`describe_chord`]).
#[track_caller]
pub fn chord_diagram(
    ui: &Ui,
    label: impl Into<String>,
    chord: Signal<ChordShape>,
    options: ChordOptions,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let first = chord.get_untracked().unwrap_or_else(|_| ChordShape::new(None, Vec::new()));
    let (w, h) = diagram_size(&first, &options, unit);
    let el = ui.el("chord_diagram").width(w).height(h).layout(|l| l.shrink = 0.0).paint(move |cx| {
        let ch = chord.get();
        let m = Metrics::new(&ch, &options, unit);
        let c = &cx.theme.colors;
        let d = cx.theme.derive();
        let full = cx.size.width;
        let mirror = move |x: f32| if options.left_handed { full - x } else { x };
        let left = m.pad;
        let top = m.head + m.marks;
        let right = left + (m.n - 1) as f32 * m.cell_w;
        let bottom = top + m.rows as f32 * m.cell_h;
        let x_of = |s: usize| mirror(left + s as f32 * m.cell_w);
        let r = m.cell_w * 0.38;
        let ink = d.text;
        // Name.
        if options.show_name
            && let Some(name) = &ch.name
        {
            cx.text(
                name,
                Point::new(cx.size.width / 2.0, m.head / 2.0),
                Vec2::new(0.5, 0.5),
                TextPaint {
                    size: cx.theme.type_scale.title * options.scale.clamp(0.6, 2.0),
                    weight: 600.0,
                    color: ink,
                    mono: false,
                },
            );
        }
        // Frets, then strings.
        for k in 0..=m.rows {
            let y = top + k as f32 * m.cell_h;
            let nut = k == 0 && ch.base_fret == 1;
            let t = if nut { (m.cell_h * 0.16).max(3.0) } else { 1.0 };
            let (a, b) = (mirror(left).min(mirror(right)), mirror(left).max(mirror(right)));
            cx.list.fill_rect(
                Rect::new(a, y - t / 2.0, b, y + t / 2.0),
                Brush::Solid(if nut { ink } else { d.muted }),
            );
        }
        for s in 0..m.n {
            let x = x_of(s);
            cx.list.fill_rect(Rect::new(x - 0.5, top, x + 0.5, bottom), Brush::Solid(d.muted));
        }
        let row_y = |fret: u32| top + (fret.saturating_sub(ch.base_fret)) as f32 * m.cell_h + m.cell_h / 2.0;
        let in_view = |fret: u32| fret >= ch.base_fret && fret < ch.base_fret + m.rows;
        let finger =
            |center: Point, f: Option<u8>| f.filter(|_| options.fingers).map(|f| (center, f.to_string()));
        // A string's own label goes before its finger number.
        let said = |s: usize, center: Point, f: Option<u8>| match ch.labels.get(s).cloned().flatten() {
            Some(text) => Some((center, text)),
            None => finger(center, f),
        };
        let root_ink = remedy_ui_core::style::ensure_contrast(c.bg, c.warn, 4.5);
        // The text on each dot, and whether the dot is a root (its ink differs).
        let mut labels: Vec<(Point, String, bool)> = Vec::new();
        // Barres under the dots.
        for b in &ch.barres {
            if !in_view(b.fret) || b.from >= m.n || b.to >= m.n {
                continue;
            }
            let (lo, hi) = (b.from.min(b.to), b.from.max(b.to));
            let (x0, x1) = (x_of(lo).min(x_of(hi)), x_of(lo).max(x_of(hi)));
            let y = row_y(b.fret);
            cx.list.fill_rounded(
                Rect::new(x0 - r, y - r, x1 + r, y + r),
                CornerRadii::all(r),
                Brush::Solid(c.accent),
            );
            labels.extend(finger(Point::new((x0 + x1) / 2.0, y), b.finger).map(|(at, t)| (at, t, false)));
        }
        // Per-string marks.
        for (s, st) in ch.strings.iter().enumerate() {
            let x = x_of(s);
            match st {
                StringState::Muted => {
                    let y = top - m.marks / 2.0;
                    let q = r * 0.6;
                    let mut p = Path::new();
                    p.move_to(Point::new(x - q, y - q));
                    p.line_to(Point::new(x + q, y + q));
                    p.move_to(Point::new(x + q, y - q));
                    p.line_to(Point::new(x - q, y + q));
                    let mut stroke = Stroke::new(1.5);
                    stroke.cap = StrokeCap::Round;
                    cx.list.push(Item::StrokePath { path: p, stroke, brush: Brush::Solid(d.muted) });
                }
                StringState::Open => {
                    let y = top - m.marks / 2.0;
                    let q = r * 0.6;
                    let ring = if ch.roots.contains(&s) { c.warn } else { d.muted };
                    cx.list.border(Rect::new(x - q, y - q, x + q, y + q), CornerRadii::all(q), 1.5, ring);
                }
                StringState::Fretted { fret, finger: f } => {
                    if !in_view(*fret) {
                        continue;
                    }
                    let center = Point::new(x, row_y(*fret));
                    let covered = ch
                        .barres
                        .iter()
                        .any(|b| b.fret == *fret && (b.from.min(b.to)..=b.from.max(b.to)).contains(&s));
                    let root = ch.roots.contains(&s);
                    let own = ch.labels.get(s).is_some_and(Option::is_some);
                    // A root, or a string with its own label, shows its dot over a barre too.
                    if !covered || root || own {
                        cx.list.fill_rounded(
                            Rect::new(center.x - r, center.y - r, center.x + r, center.y + r),
                            CornerRadii::all(r),
                            Brush::Solid(if root { c.warn } else { c.accent }),
                        );
                        labels.extend(said(s, center, *f).map(|(at, t)| (at, t, root)));
                    }
                }
            }
        }
        for (at, text, root) in labels {
            // Two characters ("b3", "R5") still fit the dot.
            let size = (r * if text.chars().count() > 1 { 1.15 } else { 1.5 }).max(8.0);
            let color = if root { root_ink } else { d.on_accent };
            cx.text(&text, at, Vec2::new(0.5, 0.5), TextPaint { size, weight: 600.0, color, mono: false });
        }
        // The strings' names under the box.
        for (s, name) in ch.string_names.iter().enumerate().take(m.n) {
            cx.text(
                name,
                Point::new(x_of(s), bottom + unit * 2.4 * options.scale.max(0.25)),
                Vec2::new(0.5, 0.5),
                TextPaint {
                    size: cx.theme.type_scale.small * options.scale.clamp(0.6, 2.0),
                    weight: 400.0,
                    color: d.muted,
                    mono: false,
                },
            );
        }
        // Where the diagram starts.
        if ch.base_fret > 1 {
            let (x, anchor) = if options.left_handed {
                (mirror(right) - r - m.cell_w * 0.4, Vec2::new(1.0, 0.5))
            } else {
                (right + r + m.cell_w * 0.4, Vec2::new(0.0, 0.5))
            };
            cx.text(
                &format!("{}fr", ch.base_fret),
                Point::new(x, top + m.cell_h / 2.0),
                anchor,
                TextPaint {
                    size: cx.theme.type_scale.small * options.scale.clamp(0.6, 2.0),
                    weight: 400.0,
                    color: d.muted,
                    mono: true,
                },
            );
        }
    });
    let (sem_el, u) = (el.clone(), ui.clone());
    el.bind(move || {
        let ch = chord.get();
        let (w, h) = diagram_size(&ch, &options, unit);
        let _ = sem_el.clone().layout(|l| {
            l.width = Length::Px(w);
            l.height = Length::Px(h);
        });
        let locale = u.locale();
        let name = ch.name.clone().map_or_else(
            || label.clone(),
            |n| {
                crate::music::say(
                    &locale,
                    "chord-named",
                    &[("label", label.as_str().into()), ("name", n.into())],
                )
            },
        );
        let _ = sem_el
            .clone()
            .semantics(Semantics::new(Role::Image).label(name).description(describe_chord_in(&ch, &locale)));
        sem_el.repaint();
    })
}
