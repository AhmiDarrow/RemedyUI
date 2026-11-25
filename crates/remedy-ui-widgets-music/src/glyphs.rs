//! Music notation drawn as paths: clefs, note heads, stems, flags, beams, rests and
//! accidentals. Everything is in units of one staff space, scaled by the caller, and takes
//! its color from the caller, so the theme decides how it looks.

use remedy_ui_core::{Color, Point, Rect};
use remedy_ui_engine::paint::{Brush, DisplayList, Item, Path, Stroke, StrokeCap, StrokeJoin};

/// How long a note lasts, as a note value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NoteValue {
    /// A whole note (semibreve).
    Whole,
    /// A half note (minim).
    Half,
    /// A quarter note (crotchet).
    Quarter,
    /// An eighth note (quaver).
    Eighth,
    /// A sixteenth note.
    Sixteenth,
    /// A thirty-second note.
    ThirtySecond,
}

impl NoteValue {
    /// The number in the note's name: 1 for whole, 2 for half, 4 for quarter, and so on.
    pub fn denominator(self) -> u32 {
        match self {
            NoteValue::Whole => 1,
            NoteValue::Half => 2,
            NoteValue::Quarter => 4,
            NoteValue::Eighth => 8,
            NoteValue::Sixteenth => 16,
            NoteValue::ThirtySecond => 32,
        }
    }

    /// The next longer note value, if there is one.
    pub fn longer(self) -> Option<NoteValue> {
        match self {
            NoteValue::Whole => None,
            NoteValue::Half => Some(NoteValue::Whole),
            NoteValue::Quarter => Some(NoteValue::Half),
            NoteValue::Eighth => Some(NoteValue::Quarter),
            NoteValue::Sixteenth => Some(NoteValue::Eighth),
            NoteValue::ThirtySecond => Some(NoteValue::Sixteenth),
        }
    }

    /// The next shorter note value, if there is one.
    pub fn shorter(self) -> Option<NoteValue> {
        match self {
            NoteValue::Whole => Some(NoteValue::Half),
            NoteValue::Half => Some(NoteValue::Quarter),
            NoteValue::Quarter => Some(NoteValue::Eighth),
            NoteValue::Eighth => Some(NoteValue::Sixteenth),
            NoteValue::Sixteenth => Some(NoteValue::ThirtySecond),
            NoteValue::ThirtySecond => None,
        }
    }

    /// How many flags or beams it has: 0 up to a quarter, 1 for an eighth, and so on.
    pub fn flags(self) -> u8 {
        match self {
            NoteValue::Whole | NoteValue::Half | NoteValue::Quarter => 0,
            NoteValue::Eighth => 1,
            NoteValue::Sixteenth => 2,
            NoteValue::ThirtySecond => 3,
        }
    }

    /// Whether the note has a stem (everything but a whole note).
    pub fn has_stem(self) -> bool {
        self != NoteValue::Whole
    }

    /// Whether the head is filled (quarter and shorter).
    pub fn filled(self) -> bool {
        self.denominator() >= 4
    }

    /// Its name in words.
    pub fn name(self) -> &'static str {
        match self {
            NoteValue::Whole => "whole",
            NoteValue::Half => "half",
            NoteValue::Quarter => "quarter",
            NoteValue::Eighth => "eighth",
            NoteValue::Sixteenth => "sixteenth",
            NoteValue::ThirtySecond => "thirty-second",
        }
    }

    /// The length in quarter notes with `dots` dots (each adds half of what came before).
    pub fn quarters(self, dots: u8) -> f32 {
        let base = 4.0 / self.denominator() as f32;
        base * (2.0 - 0.5f32.powi(i32::from(dots)))
    }
}

/// A clef.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Clef {
    /// The G clef: the second line is G4. For high voices, guitar and most melodies.
    #[default]
    Treble,
    /// The F clef: the fourth line is F3.
    Bass,
    /// The C clef on the middle line (viola).
    Alto,
    /// The C clef on the fourth line (cello and trombone in their high range).
    Tenor,
}

impl Clef {
    /// The diatonic number ([`crate::Pitch::diatonic`]) of the staff's bottom line.
    pub fn bottom_line(self) -> i32 {
        match self {
            Clef::Treble => 4 * 7 + 2, // E4
            Clef::Bass => 2 * 7 + 4,   // G2
            Clef::Alto => 3 * 7 + 3,   // F3
            Clef::Tenor => 3 * 7 + 1,  // D3
        }
    }

    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Clef::Treble => "treble",
            Clef::Bass => "bass",
            Clef::Alto => "alto",
            Clef::Tenor => "tenor",
        }
    }
}

/// Where the staff is: its bottom line's y and the size of a staff space.
#[derive(Clone, Copy, Debug)]
pub struct Staff {
    /// The y of the bottom line.
    pub bottom: f32,
    /// The distance between two lines.
    pub space: f32,
}

impl Staff {
    /// The y of a staff position: 0 is the bottom line, 1 the first space, 8 the top line.
    pub fn y(&self, position: i32) -> f32 {
        self.bottom - position as f32 * self.space / 2.0
    }
}

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn stroke(list: &mut DisplayList, path: Path, width: f32, color: Color) {
    let mut s = Stroke::new(width);
    s.cap = StrokeCap::Round;
    s.join = StrokeJoin::Round;
    list.push(Item::StrokePath { path, stroke: s, brush: Brush::Solid(color) });
}

fn fill(list: &mut DisplayList, path: Path, color: Color) {
    list.push(Item::FillPath { path, brush: Brush::Solid(color), even_odd: false });
}

/// An ellipse as a path, tilted by `tilt` radians.
fn ellipse(c: Point, rx: f32, ry: f32, tilt: f32) -> Path {
    const K: f32 = 0.552_284_8;
    let (s, co) = tilt.sin_cos();
    let at = |x: f32, y: f32| pt(c.x + x * co - y * s, c.y + x * s + y * co);
    let mut p = Path::new();
    p.move_to(at(rx, 0.0));
    p.cubic_to(at(rx, ry * K), at(rx * K, ry), at(0.0, ry));
    p.cubic_to(at(-rx * K, ry), at(-rx, ry * K), at(-rx, 0.0));
    p.cubic_to(at(-rx, -ry * K), at(-rx * K, -ry), at(0.0, -ry));
    p.cubic_to(at(rx * K, -ry), at(rx, -ry * K), at(rx, 0.0));
    p.close();
    p
}

/// A line of the staff or a ledger line.
pub fn line(list: &mut DisplayList, x0: f32, x1: f32, y: f32, width: f32, color: Color) {
    list.fill_rect(Rect::new(x0, y - width / 2.0, x1, y + width / 2.0), Brush::Solid(color));
}

/// Draws a clef with its anchor (the G line for treble, the F line for bass, the middle line
/// for the C clefs) at `(x, y)`. The clef extends to the right of `x`.
pub fn draw_clef(list: &mut DisplayList, clef: Clef, x: f32, staff: Staff, color: Color) {
    let sp = staff.space;
    match clef {
        Clef::Treble => {
            let y = staff.y(2);
            let m = |px: f32, py: f32| pt(x + (px + 0.75) * sp, y - py * sp);
            let mut stem = Path::new();
            stem.move_to(m(0.32, -2.0));
            stem.cubic_to(m(0.28, -0.5), m(0.38, 2.2), m(0.42, 3.75));
            stroke(list, stem, sp * 0.2, color);
            let mut curl = Path::new();
            curl.move_to(m(0.32, -2.0));
            curl.cubic_to(m(0.32, -2.75), m(-0.6, -2.75), m(-0.55, -1.95));
            stroke(list, curl, sp * 0.2, color);
            let mut bow = Path::new();
            bow.move_to(m(0.42, 3.75));
            bow.cubic_to(m(0.95, 3.2), m(0.9, 2.2), m(0.25, 1.55));
            bow.cubic_to(m(-0.55, 0.8), m(-0.75, 0.0), m(-0.3, -0.55));
            bow.cubic_to(m(0.2, -1.05), m(0.9, -0.65), m(0.85, 0.05));
            bow.cubic_to(m(0.8, 0.6), m(0.2, 0.85), m(0.05, 0.3));
            bow.cubic_to(m(-0.05, -0.05), m(0.3, -0.25), m(0.4, 0.1));
            stroke(list, bow, sp * 0.2, color);
            fill(list, ellipse(m(-0.55, -1.8), sp * 0.14, sp * 0.14, 0.0), color);
        }
        Clef::Bass => {
            let y = staff.y(6);
            let m = |px: f32, py: f32| pt(x + (px + 0.5) * sp, y - py * sp);
            fill(list, ellipse(m(0.0, 0.0), sp * 0.42, sp * 0.4, 0.0), color);
            let mut tail = Path::new();
            tail.move_to(m(-0.4, 0.15));
            tail.cubic_to(m(-0.2, 0.95), m(1.3, 0.9), m(1.2, -0.45));
            tail.cubic_to(m(1.1, -1.25), m(0.5, -1.8), m(-0.5, -2.2));
            stroke(list, tail, sp * 0.24, color);
            for dy in [0.5, -0.5] {
                fill(list, ellipse(m(1.75, dy), sp * 0.13, sp * 0.13, 0.0), color);
            }
        }
        Clef::Alto | Clef::Tenor => {
            let line = if clef == Clef::Alto { 4 } else { 6 };
            let y = staff.y(line);
            let m = |px: f32, py: f32| pt(x + px * sp, y - py * sp);
            list.fill_rect(
                Rect::new(m(0.0, 0.0).x, m(0.0, 2.0).y, m(0.45, 0.0).x, m(0.0, -2.0).y),
                Brush::Solid(color),
            );
            list.fill_rect(
                Rect::new(m(0.62, 0.0).x, m(0.0, 2.0).y, m(0.72, 0.0).x, m(0.0, -2.0).y),
                Brush::Solid(color),
            );
            for sign in [1.0f32, -1.0] {
                let mut p = Path::new();
                p.move_to(m(0.75, 0.05 * sign));
                p.cubic_to(m(1.5, 0.15 * sign), m(1.95, 0.9 * sign), m(1.45, 1.55 * sign));
                p.cubic_to(m(1.25, 1.85 * sign), m(0.85, 1.9 * sign), m(0.8, 1.55 * sign));
                stroke(list, p, sp * 0.2, color);
                fill(list, ellipse(m(1.1, 1.0 * sign), sp * 0.15, sp * 0.15, 0.0), color);
            }
        }
    }
}

/// The width a clef takes, in staff spaces, including some room after it.
pub fn clef_width(clef: Clef) -> f32 {
    match clef {
        Clef::Treble => 2.6,
        Clef::Bass => 3.4,
        Clef::Alto | Clef::Tenor => 3.2,
    }
}

/// A note head centered at `c`: filled for quarter notes and shorter, hollow otherwise.
pub fn draw_head(list: &mut DisplayList, value: NoteValue, c: Point, space: f32, color: Color) {
    let (rx, ry) = (space * 0.66, space * 0.45);
    if value == NoteValue::Whole {
        stroke(list, ellipse(c, rx * 1.12, ry, 0.0), space * 0.28, color);
        stroke(list, ellipse(c, rx * 0.5, ry * 0.9, 0.9), space * 0.1, color);
    } else if value.filled() {
        fill(list, ellipse(c, rx, ry, -0.35), color);
    } else {
        stroke(list, ellipse(c, rx * 0.92, ry * 0.86, -0.35), space * 0.17, color);
    }
}

/// Half the width of a note head.
pub fn head_half_width(space: f32) -> f32 {
    space * 0.66
}

/// A stem from a note head's edge to `end_y`. `up` puts it on the right of the head.
pub fn draw_stem(list: &mut DisplayList, head: Point, up: bool, end_y: f32, space: f32, color: Color) {
    let x = if up { head.x + head_half_width(space) - 0.5 } else { head.x - head_half_width(space) + 0.5 };
    let (a, b) = if up { (end_y, head.y) } else { (head.y, end_y) };
    list.fill_rect(Rect::new(x - 0.6, a.min(b), x + 0.6, a.max(b)), Brush::Solid(color));
}

/// The x of a stem for a head at `x`.
pub fn stem_x(x: f32, up: bool, space: f32) -> f32 {
    if up { x + head_half_width(space) - 0.5 } else { x - head_half_width(space) + 0.5 }
}

/// A flag at the end of a stem (for a note that is not beamed). `flags` is 1 to 3.
pub fn draw_flags(
    list: &mut DisplayList,
    stem_x: f32,
    end_y: f32,
    up: bool,
    flags: u8,
    space: f32,
    color: Color,
) {
    let dir = if up { 1.0 } else { -1.0 };
    for i in 0..flags {
        // Each flag sits a little lower on the stem.
        let y0 = end_y + dir * i as f32 * space * 0.85;
        let mut p = Path::new();
        p.move_to(pt(stem_x, y0));
        p.cubic_to(
            pt(stem_x + space * 0.15, y0 + dir * space * 0.9),
            pt(stem_x + space * 1.15, y0 + dir * space * 1.0),
            pt(stem_x + space * 0.85, y0 + dir * space * 2.1),
        );
        stroke(list, p, space * 0.22, color);
    }
}

/// A beam between two stem ends, `thick` tall, as a parallelogram.
pub fn draw_beam(list: &mut DisplayList, x0: f32, y0: f32, x1: f32, y1: f32, thick: f32, color: Color) {
    let mut p = Path::new();
    p.move_to(pt(x0, y0));
    p.line_to(pt(x1, y1));
    p.line_to(pt(x1, y1 + thick));
    p.line_to(pt(x0, y0 + thick));
    p.close();
    fill(list, p, color);
}

/// A dot after a note head.
pub fn draw_dot(list: &mut DisplayList, c: Point, space: f32, color: Color) {
    fill(list, ellipse(c, space * 0.17, space * 0.17, 0.0), color);
}

/// An accidental centered on `(x, y)` where `alter` is -2 to 2 for double flat to double sharp,
/// with 0 drawing a natural.
pub fn draw_accidental(list: &mut DisplayList, alter: i8, x: f32, y: f32, space: f32, color: Color) {
    let s = space;
    match alter {
        1 => {
            for dx in [-0.22f32, 0.22] {
                list.fill_rect(
                    Rect::new(x + dx * s - 0.5, y - 1.15 * s, x + dx * s + 0.5, y + 1.15 * s),
                    Brush::Solid(color),
                );
            }
            for dy in [-0.32f32, 0.32] {
                let mut p = Path::new();
                p.move_to(pt(x - 0.5 * s, y + (dy + 0.2) * s));
                p.line_to(pt(x + 0.5 * s, y + (dy - 0.2) * s));
                stroke(list, p, s * 0.28, color);
            }
        }
        0 => {
            let (l, r) = (x - 0.28 * s, x + 0.28 * s);
            list.fill_rect(Rect::new(l - 0.5, y - 1.15 * s, l + 0.5, y + 0.55 * s), Brush::Solid(color));
            list.fill_rect(Rect::new(r - 0.5, y - 0.55 * s, r + 0.5, y + 1.15 * s), Brush::Solid(color));
            for (y0, y1) in [(0.55f32, 0.3f32), (-0.3, -0.55)] {
                let mut p = Path::new();
                p.move_to(pt(l, y + y0 * s));
                p.line_to(pt(r, y + y1 * s));
                stroke(list, p, s * 0.26, color);
            }
        }
        -1 | -2 => {
            let shift = if alter == -2 { -0.28 } else { 0.0 };
            let flat = |list: &mut DisplayList, cx: f32| {
                list.fill_rect(
                    Rect::new(cx - 0.3 * s - 0.5, y - 1.55 * s, cx - 0.3 * s + 0.5, y + 0.55 * s),
                    Brush::Solid(color),
                );
                let mut p = Path::new();
                p.move_to(pt(cx - 0.3 * s, y + 0.55 * s));
                p.cubic_to(
                    pt(cx + 0.7 * s, y + 0.2 * s),
                    pt(cx + 0.65 * s, y - 0.65 * s),
                    pt(cx - 0.3 * s, y - 0.25 * s),
                );
                stroke(list, p, s * 0.24, color);
            };
            if alter == -2 {
                flat(list, x - 0.3 * s);
                flat(list, x + 0.28 * s);
            } else {
                flat(list, x + shift * s + 0.15 * s);
            }
        }
        _ => {
            // Double sharp (or more): a cross.
            let mut p = Path::new();
            p.move_to(pt(x - 0.4 * s, y - 0.4 * s));
            p.line_to(pt(x + 0.4 * s, y + 0.4 * s));
            p.move_to(pt(x + 0.4 * s, y - 0.4 * s));
            p.line_to(pt(x - 0.4 * s, y + 0.4 * s));
            stroke(list, p, s * 0.3, color);
        }
    }
}

/// How much room an accidental takes, in staff spaces.
pub fn accidental_width(alter: i8) -> f32 {
    if alter == -2 { 1.6 } else { 1.2 }
}

/// A rest of `value` on `staff`, centered at `x`.
pub fn draw_rest(list: &mut DisplayList, value: NoteValue, x: f32, staff: Staff, color: Color) {
    let s = staff.space;
    let mid = staff.y(4);
    match value {
        NoteValue::Whole => {
            // Hangs from the fourth line.
            let top = staff.y(6);
            list.fill_rect(Rect::new(x - 0.6 * s, top, x + 0.6 * s, top + 0.5 * s), Brush::Solid(color));
        }
        NoteValue::Half => {
            // Sits on the middle line.
            list.fill_rect(Rect::new(x - 0.6 * s, mid - 0.5 * s, x + 0.6 * s, mid), Brush::Solid(color));
        }
        NoteValue::Quarter => {
            let mut p = Path::new();
            p.move_to(pt(x - 0.25 * s, mid - 1.5 * s));
            p.line_to(pt(x + 0.3 * s, mid - 0.6 * s));
            p.line_to(pt(x - 0.3 * s, mid + 0.1 * s));
            p.line_to(pt(x + 0.3 * s, mid + 0.9 * s));
            p.cubic_to(
                pt(x - 0.5 * s, mid + 0.7 * s),
                pt(x - 0.5 * s, mid + 1.6 * s),
                pt(x + 0.1 * s, mid + 1.6 * s),
            );
            stroke(list, p, s * 0.26, color);
        }
        v => {
            // Flagged rests: a slanted stem with a dot and a flag for each beam.
            let n = v.flags();
            let top = mid - s * (0.6 + 0.5 * f32::from(n));
            let bottom = mid + s * 1.4;
            let mut stem = Path::new();
            stem.move_to(pt(x + 0.35 * s, top));
            stem.line_to(pt(x - 0.2 * s - 0.0, bottom));
            stroke(list, stem, s * 0.2, color);
            for i in 0..n {
                let y = top + i as f32 * s * 0.9 + 0.4 * s;
                let xs = x + 0.35 * s - (y - top) / (bottom - top) * 0.55 * s;
                fill(list, ellipse(pt(xs - 0.55 * s, y), s * 0.2, s * 0.2, 0.0), color);
                let mut p = Path::new();
                p.move_to(pt(xs - 0.55 * s, y));
                p.cubic_to(
                    pt(xs - 0.2 * s, y + 0.1 * s),
                    pt(xs, y - 0.1 * s),
                    pt(xs + 0.1 * s, y - 0.55 * s),
                );
                stroke(list, p, s * 0.2, color);
            }
        }
    }
}

/// A curved tie or slur between two points on a level.
#[derive(Clone, Copy, Debug)]
pub struct Arc {
    /// Where it starts.
    pub x0: f32,
    /// Where it ends.
    pub x1: f32,
    /// The level both ends sit on.
    pub y: f32,
    /// Bulge up (true) or down.
    pub up: bool,
    /// How far it bulges.
    pub height: f32,
    /// The line's width.
    pub width: f32,
}

/// Draws a tie or slur.
pub fn draw_arc(list: &mut DisplayList, arc: Arc, color: Color) {
    let Arc { x0, x1, y, up, height, width } = arc;
    let dir = if up { -1.0 } else { 1.0 };
    let mut p = Path::new();
    p.move_to(pt(x0, y));
    p.cubic_to(
        pt(x0 + (x1 - x0) * 0.2, y + dir * height * 1.3),
        pt(x1 - (x1 - x0) * 0.2, y + dir * height * 1.3),
        pt(x1, y),
    );
    stroke(list, p, width, color);
}
