//! The fretboard: strings and frets spaced as on a real neck, with marked notes, a picked
//! position and a keyboard cursor.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect, Vec2};
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_layout::Length;
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_style::ensure_contrast;
use remedy_ui_view::{El, TextPaint, Ui};

use crate::pitch::{GUITAR, Pitch};

/// A place on the neck: a string and a fret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FretPos {
    /// The string, counted from the lowest one (the thickest on a guitar): 0 is string 6.
    pub string: usize,
    /// The fret; 0 is the open string.
    pub fret: u32,
}

/// How a mark looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkKind {
    /// A note in the accent color.
    #[default]
    Normal,
    /// The root of a scale or chord, in the warning color.
    Root,
    /// A note that is not played but shown, as an outline.
    Ghost,
}

/// A marked note on the neck.
#[derive(Clone, Debug, PartialEq)]
pub struct FretMark {
    /// Where.
    pub pos: FretPos,
    /// What the mark says; `None` says nothing, or the note name when the board is set to name notes.
    pub label: Option<String>,
    /// How it looks.
    pub kind: MarkKind,
}

impl FretMark {
    /// A plain mark with no text.
    pub fn at(string: usize, fret: u32) -> FretMark {
        FretMark { pos: FretPos { string, fret }, label: None, kind: MarkKind::Normal }
    }

    /// The same mark saying `text`.
    pub fn labeled(mut self, text: impl Into<String>) -> FretMark {
        self.label = Some(text.into());
        self
    }

    /// The same mark in another look.
    pub fn kind(mut self, kind: MarkKind) -> FretMark {
        self.kind = kind;
        self
    }
}

/// The dots on the neck.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Inlays {
    /// Single dots at frets 3, 5, 7 and 9 and double dots at 12, repeating every octave.
    Standard,
    /// Dots where the instrument has them: `single` and `double` list fret numbers.
    Custom {
        /// Frets with one dot.
        single: Vec<u32>,
        /// Frets with two dots.
        double: Vec<u32>,
    },
    /// No dots.
    None,
}

impl Inlays {
    /// How many dots sit at `fret` (0, 1 or 2).
    pub fn dots_at(&self, fret: u32) -> u8 {
        match self {
            Inlays::Standard => match fret % 12 {
                _ if fret == 0 => 0,
                0 => 2,
                3 | 5 | 7 | 9 => 1,
                _ => 0,
            },
            Inlays::Custom { single, double } => {
                if double.contains(&fret) {
                    2
                } else {
                    u8::from(single.contains(&fret))
                }
            }
            Inlays::None => 0,
        }
    }
}

/// How a [`fretboard`] is set up.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FretboardOptions {
    /// The open strings as MIDI notes, from the bottom string (drawn lowest on screen) up.
    /// The number of strings is the length. Any tuning and any number of strings works.
    pub tuning: Vec<i32>,
    /// The number of the wire the shown part starts at: 0 shows the nut and open strings,
    /// 5 starts at the fifth wire and shows frets 6 onward.
    pub first_fret: u32,
    /// How many frets are shown.
    pub frets: u32,
    /// Mirror the neck for a left-handed player (the nut on the right).
    pub left_handed: bool,
    /// Show fret numbers under the neck.
    pub fret_numbers: bool,
    /// Show the open string names at the nut.
    pub string_names: bool,
    /// The dots.
    pub inlays: Inlays,
    /// Marked notes, updated by the app as it likes.
    pub marks: Option<Signal<Vec<FretMark>>>,
    /// Write the note name in marks that have no label of their own.
    pub name_notes: bool,
    /// A sounding pitch (a tuner's note, as a MIDI number): every place that makes it gets a
    /// ring. Updating it many times a second is fine; it repaints only when it changes.
    pub sounding: Option<Signal<Option<i32>>>,
    /// Show the neck without letting people pick: it is an image, not a control.
    pub read_only: bool,
    /// Spell black keys with flats in names.
    pub flats: bool,
}

impl FretboardOptions {
    /// Sets `tuning`: the open strings as MIDI notes, from the bottom string (drawn lowest on screen) up.
    pub fn tuning(mut self, value: Vec<i32>) -> Self {
        self.tuning = value;
        self
    }

    /// Sets `first_fret`: the number of the wire the shown part starts at: 0 shows the nut and open strings, 5 starts at the fifth wire and shows frets 6 onward.
    pub fn first_fret(mut self, value: u32) -> Self {
        self.first_fret = value;
        self
    }

    /// Sets `frets`: how many frets are shown.
    pub fn frets(mut self, value: u32) -> Self {
        self.frets = value;
        self
    }

    /// Sets `left_handed`: mirror the neck for a left-handed player (the nut on the right).
    pub fn left_handed(mut self, value: bool) -> Self {
        self.left_handed = value;
        self
    }

    /// Sets `fret_numbers`: show fret numbers under the neck.
    pub fn fret_numbers(mut self, value: bool) -> Self {
        self.fret_numbers = value;
        self
    }

    /// Sets `string_names`: show the open string names at the nut.
    pub fn string_names(mut self, value: bool) -> Self {
        self.string_names = value;
        self
    }

    /// Sets `inlays`: the dots.
    pub fn inlays(mut self, value: Inlays) -> Self {
        self.inlays = value;
        self
    }

    /// Sets `marks`: marked notes, updated by the app as it likes.
    pub fn marks(mut self, value: Signal<Vec<FretMark>>) -> Self {
        self.marks = Some(value);
        self
    }

    /// Sets `name_notes`: write the note name in marks that have no label of their own.
    pub fn name_notes(mut self, value: bool) -> Self {
        self.name_notes = value;
        self
    }

    /// Sets `sounding`: a sounding pitch (a tuner's note, as a MIDI number): every place that makes it gets a ring.
    pub fn sounding(mut self, value: Signal<Option<i32>>) -> Self {
        self.sounding = Some(value);
        self
    }

    /// Sets `read_only`: show the neck without letting people pick: it is an image, not a control.
    pub fn read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }

    /// Sets `flats`: spell black keys with flats in names.
    pub fn flats(mut self, value: bool) -> Self {
        self.flats = value;
        self
    }
}

impl Default for FretboardOptions {
    fn default() -> Self {
        FretboardOptions {
            tuning: GUITAR.to_vec(),
            first_fret: 0,
            frets: 12,
            left_handed: false,
            fret_numbers: true,
            string_names: true,
            inlays: Inlays::Standard,
            marks: None,
            name_notes: false,
            sounding: None,
            read_only: false,
            flats: false,
        }
    }
}

/// Where fret `n` is along the scale length, from the nut (0) toward the bridge (1), by the
/// equal-tempered rule that each fret is the twelfth root of two shorter than the one before.
pub fn fret_position(n: u32) -> f64 {
    1.0 - 2f64.powf(-f64::from(n) / 12.0)
}

/// Where everything on a fretboard is, for a given width. The widget draws and hit-tests
/// with this; apps can use it to put their own overlays on the neck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FretboardLayout {
    /// The strings.
    pub strings: usize,
    /// The number of the wire the neck starts at.
    pub first_fret: u32,
    /// How many frets are shown.
    pub frets: u32,
    /// Mirrored for a left-handed player.
    pub left_handed: bool,
    /// The whole width.
    pub width: f32,
    /// The whole height.
    pub height: f32,
    /// The distance between strings.
    pub string_gap: f32,
    /// The space left of the nut for open string names and marks.
    pub open_zone: f32,
    /// The part of that space, at the outer edge, taken by the string names.
    pub names: f32,
    /// The room at the right edge.
    pub tail: f32,
    /// The room above the top string.
    pub top: f32,
    /// The height of the row of fret numbers.
    pub numbers: f32,
}

impl FretboardLayout {
    /// The layout for `options` in a box `width` wide, with spacing from the theme `unit`.
    pub fn new(options: &FretboardOptions, width: f32, unit: f32) -> FretboardLayout {
        let strings = options.tuning.len().max(1);
        let string_gap = unit * 5.0;
        let top = unit * 3.0;
        let numbers = if options.fret_numbers { unit * 5.0 } else { 0.0 };
        let names = if options.string_names { unit * 4.5 } else { 0.0 };
        let open_zone = names + if options.first_fret == 0 { unit * 6.0 } else { unit };
        let tail = unit * 2.0;
        let height = top * 2.0 + (strings - 1) as f32 * string_gap + numbers;
        FretboardLayout {
            strings,
            first_fret: options.first_fret,
            frets: options.frets.max(1),
            left_handed: options.left_handed,
            width,
            height,
            string_gap,
            open_zone,
            names,
            tail,
            top,
            numbers,
        }
    }

    fn mirror(&self, x: f32) -> f32 {
        if self.left_handed { self.width - x } else { x }
    }

    /// The x of the wire that ends fret `n` (and starts fret `n + 1`), `first_fret` to
    /// `first_fret + frets`. Wires are spaced by [`fret_position`].
    pub fn wire_x(&self, n: u32) -> f32 {
        let span = self.width - self.open_zone - self.tail;
        let from = fret_position(self.first_fret);
        let to = fret_position(self.first_fret + self.frets);
        let t = ((fret_position(n) - from) / (to - from)) as f32;
        self.mirror(self.open_zone + t * span)
    }

    /// The x of the middle of fret `n`'s cell (between its wires); for fret 0 the open string
    /// mark's place left of the nut.
    pub fn cell_x(&self, fret: u32) -> f32 {
        if fret == 0 {
            return self.mirror(self.names + (self.open_zone - self.names) / 2.0);
        }
        (self.wire_x(fret - 1) + self.wire_x(fret)) / 2.0
    }

    /// The y of string `s` (0 is the bottom string).
    pub fn string_y(&self, s: usize) -> f32 {
        self.top + (self.strings - 1 - s.min(self.strings - 1)) as f32 * self.string_gap
    }

    /// The first and last fret that can be marked or picked.
    pub fn fret_range(&self) -> (u32, u32) {
        (if self.first_fret == 0 { 0 } else { self.first_fret + 1 }, self.first_fret + self.frets)
    }

    /// The position under a point (local to the element), if it is on the neck.
    pub fn hit(&self, p: Point) -> Option<FretPos> {
        let (lo, hi) = self.fret_range();
        let slack = self.string_gap * 0.6;
        if p.y < self.top - slack || p.y > self.top + (self.strings - 1) as f32 * self.string_gap + slack {
            return None;
        }
        let row = ((p.y - self.top) / self.string_gap).round().clamp(0.0, (self.strings - 1) as f32) as usize;
        let string = self.strings - 1 - row;
        // The cell whose span holds x.
        let mut best: Option<(u32, f32)> = None;
        for f in lo..=hi {
            let d = (self.cell_x(f) - p.x).abs();
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((f, d));
            }
        }
        let (fret, d) = best?;
        // Not far outside the neck.
        let outer = if fret == 0 {
            self.open_zone / 2.0
        } else {
            (self.wire_x(fret) - self.wire_x(fret - 1)).abs() / 2.0
        };
        (d <= outer + 1.0).then_some(FretPos { string, fret })
    }
}

fn note_of(options: &FretboardOptions, pos: FretPos) -> Option<Pitch> {
    let open = *options.tuning.get(pos.string)?;
    Some(Pitch::from_midi(open + pos.fret as i32, options.flats))
}

/// How a position is said: "string 3 (G3), fret 5, C4".
pub fn describe_position(options: &FretboardOptions, pos: FretPos) -> String {
    let n = options.tuning.len();
    let number = n.saturating_sub(pos.string);
    let open = options.tuning.get(pos.string).map(|m| Pitch::from_midi(*m, options.flats).spoken());
    let note = note_of(options, pos).map(Pitch::spoken);
    let fret = if pos.fret == 0 { "open".to_owned() } else { format!("fret {}", pos.fret) };
    match (open, note) {
        (Some(o), Some(nt)) => format!("string {number} ({o}), {fret}, {nt}"),
        _ => format!("string {number}, {fret}"),
    }
}

/// A fretboard with the strings of `options.tuning` and a window of frets, spaced by the
/// real rule. `selected` is the position the player picked (click, or Enter on the keyboard
/// cursor; Escape clears it). Marks come from `options.marks`.
///
/// The neck takes the keyboard as one stop: Up and Down move the cursor between strings,
/// Left and Right between frets (toward the nut and away; mirrored for left-handed boards),
/// Home and End to the first and last fret, Enter or Space picks, Escape clears. Screen
/// readers hear the cursor's string, fret and note, and whether it is picked or marked. A
/// neck is not mirrored in right-to-left windows: like music notation, it is read the way
/// an instrument is built.
#[track_caller]
pub fn fretboard(
    ui: &Ui,
    label: impl Into<String>,
    selected: Signal<Option<FretPos>>,
    options: FretboardOptions,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let probe = FretboardLayout::new(&options, 0.0, unit);
    let min_cell = unit * 6.0;
    let min_width = probe.open_zone + probe.tail + probe.frets as f32 * min_cell;
    let (lo, _) = probe.fret_range();
    let start = FretPos { string: options.tuning.len().saturating_sub(1), fret: lo };
    let cursor = Signal::new(selected.get_untracked().ok().flatten().unwrap_or(start));
    let focused = Signal::new(false);
    let ro = options.read_only;
    let options = Rc::new(options);

    let paint_opts = options.clone();
    let el =
        ui.el("fretboard")
            .fill_width()
            .height(probe.height)
            .layout(move |l| l.min_width = Length::Px(min_width))
            .paint(move |cx| {
                let o = &*paint_opts;
                let g = FretboardLayout::new(o, cx.size.width, unit);
                let c = &cx.theme.colors;
                let d = cx.theme.derive();
                let (flo, fhi) = g.fret_range();
                let left = g.wire_x(g.first_fret);
                let right = g.wire_x(g.first_fret + g.frets);
                let (bx0, bx1) = (left.min(right), left.max(right));
                let neck_top = g.string_y(g.strings - 1) - g.string_gap * 0.6;
                let neck_bottom = g.string_y(0) + g.string_gap * 0.6;
                // The wood: a little lighter than the surface under it.
                let neck = Rect::new(bx0, neck_top, bx1, neck_bottom);
                cx.list.fill_rounded(neck, CornerRadii::all(cx.theme.radii.small), Brush::Solid(c.card));
                // Inlays between the wires.
                let dot = (g.string_gap * 0.2).min((bx1 - bx0) / g.frets as f32 * 0.25);
                let mid_y = (g.string_y(0) + g.string_y(g.strings - 1)) / 2.0;
                for f in flo.max(1)..=fhi {
                    let x = g.cell_x(f);
                    let n = o.inlays.dots_at(f);
                    let ys: &[f32] = if n == 2 {
                        &[mid_y - g.string_gap * 1.0, mid_y + g.string_gap * 1.0]
                    } else if n == 1 {
                        &[mid_y]
                    } else {
                        &[]
                    };
                    for y in ys {
                        cx.list.fill_rounded(
                            Rect::new(x - dot, y - dot, x + dot, y + dot),
                            CornerRadii::all(dot),
                            Brush::Solid(d.line),
                        );
                    }
                }
                // Fret wires, and the nut.
                for n in g.first_fret..=g.first_fret + g.frets {
                    let x = g.wire_x(n);
                    let nut = n == 0;
                    let w = if nut { 4.0 } else { 1.5 };
                    cx.list.fill_rect(
                        Rect::new(x - w / 2.0, neck_top, x + w / 2.0, neck_bottom),
                        Brush::Solid(if nut { d.text } else { d.muted }),
                    );
                }
                // Strings: the lower ones are thicker.
                for s in 0..g.strings {
                    let y = g.string_y(s);
                    let w = 1.0 + (g.strings - 1 - s) as f32 * 0.35;
                    cx.list.fill_rect(
                        Rect::new(bx0, y - w / 2.0, bx1, y + w / 2.0),
                        Brush::Solid(d.text.with_alpha(0.75)),
                    );
                }
                let small =
                    TextPaint { size: cx.theme.type_scale.small, weight: 400.0, color: d.muted, mono: true };
                // Open string names at the nut, and fret numbers under the neck.
                if o.string_names {
                    for s in 0..g.strings {
                        if let Some(m) = o.tuning.get(s) {
                            let p = Pitch::from_midi(*m, o.flats);
                            let (edge, anchor) = if g.left_handed {
                                (g.width - 2.0, Vec2::new(1.0, 0.5))
                            } else {
                                (2.0, Vec2::new(0.0, 0.5))
                            };
                            cx.text(&p.letter_name(), Point::new(edge, g.string_y(s)), anchor, small);
                        }
                    }
                }
                if o.fret_numbers {
                    let y = g.string_y(0) + g.string_gap * 0.6 + g.numbers / 2.0;
                    for f in flo.max(1)..=fhi {
                        cx.text(&f.to_string(), Point::new(g.cell_x(f), y), Vec2::new(0.5, 0.5), small);
                    }
                }
                let r = (g.string_gap * 0.4).min((bx1 - bx0) / g.frets as f32 * 0.42).max(3.0);
                // The sounding pitch: rings where it can be played.
                if let Some(sig) = &o.sounding
                    && let Some(midi) = sig.get()
                {
                    for (s, open) in o.tuning.iter().enumerate() {
                        let f = midi - open;
                        if f >= 0 && (flo as i32..=fhi as i32).contains(&f) {
                            let center = Point::new(g.cell_x(f as u32), g.string_y(s));
                            cx.list.border(
                                Rect::new(
                                    center.x - r - 3.0,
                                    center.y - r - 3.0,
                                    center.x + r + 3.0,
                                    center.y + r + 3.0,
                                ),
                                CornerRadii::all(r + 3.0),
                                2.0,
                                c.ok,
                            );
                        }
                    }
                }
                // Marks.
                let marks = o.marks.as_ref().map(|m| m.get()).unwrap_or_default();
                let label_style = |color| TextPaint {
                    size: cx.theme.type_scale.small - 1.0,
                    weight: 600.0,
                    color,
                    mono: false,
                };
                for m in &marks {
                    if m.pos.string >= g.strings || m.pos.fret < flo || m.pos.fret > fhi {
                        continue;
                    }
                    let center = Point::new(g.cell_x(m.pos.fret), g.string_y(m.pos.string));
                    let disc = Rect::new(center.x - r, center.y - r, center.x + r, center.y + r);
                    let text = m.label.clone().or_else(|| {
                        if o.name_notes { note_of(o, m.pos).map(|p| p.letter_name()) } else { None }
                    });
                    let (fill, ink) = match m.kind {
                        MarkKind::Normal => (Some(c.accent), d.on_accent),
                        MarkKind::Root => (Some(c.warn), ensure_contrast(c.bg, c.warn, 4.5)),
                        MarkKind::Ghost => (None, d.muted),
                    };
                    match fill {
                        Some(f) => cx.list.fill_rounded(disc, CornerRadii::all(r), Brush::Solid(f)),
                        None => {
                            cx.list.fill_rounded(disc, CornerRadii::all(r), Brush::Solid(c.card));
                            cx.list.border(disc, CornerRadii::all(r), 1.5, d.muted);
                        }
                    }
                    if let Some(t) = text {
                        cx.text(&t, center, Vec2::new(0.5, 0.5), label_style(ink));
                    }
                }
                // The picked position.
                if let Some(p) = selected.get()
                    && p.string < g.strings
                    && p.fret >= flo
                    && p.fret <= fhi
                {
                    let center = Point::new(g.cell_x(p.fret), g.string_y(p.string));
                    let rr = r + 2.0;
                    let disc = Rect::new(center.x - rr, center.y - rr, center.x + rr, center.y + rr);
                    cx.list.fill_rounded(disc, CornerRadii::all(rr), Brush::Solid(d.text));
                    let name = note_of(o, p).map(|n| n.letter_name()).unwrap_or_default();
                    cx.text(&name, center, Vec2::new(0.5, 0.5), label_style(c.bg));
                }
                // The keyboard cursor.
                if focused.get() {
                    let p = cursor.get();
                    if p.string < g.strings && p.fret >= flo && p.fret <= fhi {
                        let center = Point::new(g.cell_x(p.fret), g.string_y(p.string));
                        let rr = r + 5.0;
                        cx.list.border(
                            Rect::new(center.x - rr, center.y - rr, center.x + rr, center.y + rr),
                            CornerRadii::all(rr),
                            2.0,
                            d.focus,
                        );
                    }
                }
            });

    if ro {
        let sem_el = el.clone();
        let opts = options.clone();
        return el.bind(move || {
            let marks = opts.marks.as_ref().map(|m| m.get()).unwrap_or_default();
            let _ = sem_el.clone().semantics(
                Semantics::new(Role::Image)
                    .label(label.clone())
                    .value(summary(&opts, &marks))
                    .read_only(true),
            );
            sem_el.repaint();
        });
    }

    let hit = {
        let (pointer_el, opts) = (el.clone(), options.clone());
        move |p: Point| -> Option<FretPos> {
            let b = pointer_el.bounds()?;
            FretboardLayout::new(&opts, b.width(), unit).hit(Point::new(p.x - b.min.x, p.y - b.min.y))
        }
    };
    let down = Rc::new(Cell::new(false));
    let key_opts = options.clone();
    let sem_opts = options.clone();
    el.focusable(true)
        .on_focus_change(move |f| focused.set_if_changed(f))
        .on_pointer(move |e| {
            match e.phase {
                PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => down.set(true),
                PointerPhase::Move if down.get() => {}
                PointerPhase::Up | PointerPhase::Cancel => return down.replace(false),
                _ => return false,
            }
            if let Some(pos) = hit(e.position) {
                selected.set_if_changed(Some(pos));
                cursor.set_if_changed(pos);
            }
            true
        })
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            let g = FretboardLayout::new(&key_opts, 0.0, unit);
            let (lo, hi) = g.fret_range();
            let cur = cursor.get_untracked().unwrap_or(start);
            let toward_bridge: i64 = if key_opts.left_handed { -1 } else { 1 };
            let by_fret = |d: i64| FretPos {
                fret: (i64::from(cur.fret) + d).clamp(i64::from(lo), i64::from(hi)) as u32,
                ..cur
            };
            let next = match e.key {
                Key::ArrowUp => FretPos { string: (cur.string + 1).min(g.strings - 1), ..cur },
                Key::ArrowDown => FretPos { string: cur.string.saturating_sub(1), ..cur },
                Key::ArrowRight => by_fret(toward_bridge),
                Key::ArrowLeft => by_fret(-toward_bridge),
                Key::Home => FretPos { fret: lo, ..cur },
                Key::End => FretPos { fret: hi, ..cur },
                Key::Enter | Key::Space => {
                    selected.set_if_changed(Some(cur));
                    return true;
                }
                Key::Escape => {
                    if selected.get_untracked().ok().flatten().is_none() {
                        return false;
                    }
                    selected.set(None);
                    return true;
                }
                _ => return false,
            };
            cursor.set_if_changed(next);
            true
        })
        .semantics_dyn(move || {
            let o = &*sem_opts;
            let cur = cursor.get();
            let picked = selected.get();
            let marks = o.marks.as_ref().map(|m| m.get()).unwrap_or_default();
            let mut value = describe_position(o, cur);
            if picked == Some(cur) {
                value.push_str(", picked");
            }
            if marks.iter().any(|m| m.pos == cur) {
                value.push_str(", marked");
            }
            let mut hint = String::from("Arrow keys move over the neck, Enter picks, Escape clears.");
            if let Some(p) = picked {
                hint.push_str(&format!(" Picked: {}.", describe_position(o, p)));
            }
            Semantics::new(Role::Grid)
                .label(label.clone())
                .value(value)
                .description(hint)
                .action(Action::Focus)
        })
}

fn summary(options: &FretboardOptions, marks: &[FretMark]) -> String {
    format!("{} strings, {} frets, {} marked", options.tuning.len(), options.frets, marks.len())
}
