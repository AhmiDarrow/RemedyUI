//! Annotations: marks people add to pages (highlights, underlines, ink, notes, text boxes and
//! shapes). They live in page points, the same space as the page's text runs, and are drawn
//! inside each page's own transform, so they stay exact at every zoom and rotation. Reading
//! and writing them in a file format (PDF annotations, a sidecar) stays with the app, as
//! parsing does: the view holds them, draws them, lets people make and change them, and tells
//! the app what changed.

use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_engine::paint::{Brush, Image, Item, Path, Sampling, Stroke, StrokeCap, StrokeJoin};

/// What an annotation is.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AnnotationKind {
    /// Text marked with a translucent fill: one rectangle per line.
    Highlight {
        /// The lines' boxes, page points.
        quads: Vec<Rect>,
    },
    /// Text with a line under each line.
    Underline {
        /// The lines' boxes.
        quads: Vec<Rect>,
    },
    /// Text struck through.
    Strikeout {
        /// The lines' boxes.
        quads: Vec<Rect>,
    },
    /// Text with a wavy line under it.
    Squiggly {
        /// The lines' boxes.
        quads: Vec<Rect>,
    },
    /// Freehand drawing.
    Ink {
        /// Each stroke's points, page points.
        strokes: Vec<Vec<Point>>,
        /// Line width, points.
        width: f32,
    },
    /// A note pinned at a point, opened to read its comment.
    Note {
        /// Where it is pinned (its icon's top-left), page points.
        at: Point,
    },
    /// Text written on the page.
    FreeText {
        /// The box it wraps in, page points.
        rect: Rect,
        /// What it says.
        text: String,
        /// Its size, points.
        size: f32,
    },
    /// A rectangle, ellipse, line or arrow.
    Shape {
        /// Which.
        shape: Shape,
        /// Where the drag started, page points.
        from: Point,
        /// Where it ended.
        to: Point,
        /// Line width, points.
        width: f32,
    },
    /// A picture placed on the page (a signature, a stamp, a scan). The app places it with
    /// [`PageView::add_annotation`](crate::doc_pages::PageView::add_annotation); selected, it is dragged
    /// to move it and by a corner to resize it, keeping its shape.
    Image {
        /// Where it lies, page points.
        rect: Rect,
        /// Its pixels.
        image: Image,
    },
}

/// The shapes people draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Shape {
    /// A rectangle.
    Rect,
    /// An ellipse in the box.
    Ellipse,
    /// A straight line.
    Line,
    /// A line with an arrowhead at its end.
    Arrow,
}

/// A mark on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    /// Its id within the view (the app's own ids go in `key`).
    pub id: u64,
    /// The page it is on, from 0.
    pub page: usize,
    /// What it is.
    pub kind: AnnotationKind,
    /// Its color.
    pub color: Color,
    /// Its comment.
    pub comment: String,
    /// Who made it, if known.
    pub author: Option<String>,
    /// The text under a text mark, captured when it was made (for lists, search and screen
    /// readers).
    pub marked_text: String,
    /// The app's own identifier (a PDF object number), carried through edits.
    pub key: Option<String>,
}

impl Annotation {
    /// A new annotation on `page` (the view gives it its id when it is added).
    pub fn new(page: usize, kind: AnnotationKind, color: Color) -> Self {
        Annotation {
            id: 0,
            page,
            kind,
            color,
            comment: String::new(),
            author: None,
            marked_text: String::new(),
            key: None,
        }
    }

    /// What it is, in a word, in English ([`Annotation::kind_name_in`] in another language).
    pub fn kind_name(&self) -> &'static str {
        match &self.kind {
            AnnotationKind::Highlight { .. } => "Highlight",
            AnnotationKind::Underline { .. } => "Underline",
            AnnotationKind::Strikeout { .. } => "Strikeout",
            AnnotationKind::Squiggly { .. } => "Squiggly underline",
            AnnotationKind::Ink { .. } => "Drawing",
            AnnotationKind::Note { .. } => "Note",
            AnnotationKind::FreeText { .. } => "Text box",
            AnnotationKind::Shape { shape: Shape::Rect, .. } => "Rectangle",
            AnnotationKind::Shape { shape: Shape::Ellipse, .. } => "Ellipse",
            AnnotationKind::Shape { shape: Shape::Line, .. } => "Line",
            AnnotationKind::Shape { shape: Shape::Arrow, .. } => "Arrow",
            AnnotationKind::Image { .. } => "Image",
        }
    }

    /// What it is, in a word, in `locale` ([`Annotation::kind_name`] translated).
    pub fn kind_name_in(&self, locale: &str) -> String {
        crate::doc_pages::say(locale, "annotation-kind", &[("kind", self.kind_id().into())])
    }

    /// Its kind as the catalog's messages select on it.
    pub(crate) fn kind_id(&self) -> &'static str {
        match &self.kind {
            AnnotationKind::Highlight { .. } => "highlight",
            AnnotationKind::Underline { .. } => "underline",
            AnnotationKind::Strikeout { .. } => "strikeout",
            AnnotationKind::Squiggly { .. } => "squiggly",
            AnnotationKind::Ink { .. } => "drawing",
            AnnotationKind::Note { .. } => "note",
            AnnotationKind::FreeText { .. } => "text-box",
            AnnotationKind::Shape { shape: Shape::Rect, .. } => "rectangle",
            AnnotationKind::Shape { shape: Shape::Ellipse, .. } => "ellipse",
            AnnotationKind::Shape { shape: Shape::Line, .. } => "line",
            AnnotationKind::Shape { shape: Shape::Arrow, .. } => "arrow",
            AnnotationKind::Image { .. } => "image",
        }
    }

    /// A sentence describing it, in English: "Highlight: “the text”, comment: …"
    /// ([`Annotation::describe_in`] for another language).
    pub fn describe(&self) -> String {
        self.describe_in("en")
    }

    /// A sentence describing it in `locale`, for lists and screen readers.
    pub fn describe_in(&self, locale: &str) -> String {
        let mut s = self.kind_name_in(locale);
        let quote = |s: String, text: &str| {
            crate::doc_pages::say(locale, "annotation-quoting", &[("what", s.into()), ("text", text.into())])
        };
        if !self.marked_text.is_empty() {
            s = quote(s, &self.marked_text);
        }
        if let AnnotationKind::FreeText { text, .. } = &self.kind {
            s = quote(s, text);
        }
        if !self.comment.is_empty() {
            s = crate::doc_pages::say(
                locale,
                "annotation-with-comment",
                &[("what", s.into()), ("comment", self.comment.as_str().into())],
            );
        }
        if let Some(a) = &self.author {
            s = crate::doc_pages::say(locale, "annotation-by", &[("what", s.into()), ("author", a.as_str().into())]);
        }
        s
    }

    /// The area it covers on its page, points (including its line width).
    pub fn bounds(&self) -> Rect {
        let grow = |r: Rect, d: f32| Rect::new(r.min.x - d, r.min.y - d, r.max.x + d, r.max.y + d);
        match &self.kind {
            AnnotationKind::Highlight { quads }
            | AnnotationKind::Underline { quads }
            | AnnotationKind::Strikeout { quads }
            | AnnotationKind::Squiggly { quads } => union(quads.iter().copied()).unwrap_or(Rect::ZERO),
            AnnotationKind::Ink { strokes, width } => {
                let pts = strokes.iter().flatten().copied();
                grow(bounds_of(pts).unwrap_or(Rect::ZERO), width / 2.0)
            }
            AnnotationKind::Note { at } => Rect::from_origin_size(*at, remedy_ui_core::Size::new(NOTE, NOTE)),
            AnnotationKind::FreeText { rect, .. } | AnnotationKind::Image { rect, .. } => *rect,
            AnnotationKind::Shape { from, to, width, .. } => {
                grow(box_of(*from, *to), width / 2.0 + ARROW_HEAD)
            }
        }
    }

    /// Whether a press at `p` (page points) lands on it, within `slop` points.
    pub fn hit(&self, p: Point, slop: f32) -> bool {
        let inside = |r: &Rect| {
            p.x >= r.min.x - slop && p.x <= r.max.x + slop && p.y >= r.min.y - slop && p.y <= r.max.y + slop
        };
        match &self.kind {
            AnnotationKind::Highlight { quads }
            | AnnotationKind::Underline { quads }
            | AnnotationKind::Strikeout { quads }
            | AnnotationKind::Squiggly { quads } => quads.iter().any(inside),
            AnnotationKind::Ink { strokes, width } => strokes.iter().any(|s| {
                let reach = width / 2.0 + slop;
                match s.as_slice() {
                    [one] => dist(*one, p) <= reach,
                    pts => pts.windows(2).any(|w| match w {
                        [a, b] => seg_dist(p, *a, *b) <= reach,
                        _ => false,
                    }),
                }
            }),
            AnnotationKind::Note { .. } | AnnotationKind::FreeText { .. } | AnnotationKind::Image { .. } => {
                inside(&self.bounds())
            }
            AnnotationKind::Shape { shape, from, to, width } => {
                let reach = width / 2.0 + slop;
                match shape {
                    Shape::Line | Shape::Arrow => seg_dist(p, *from, *to) <= reach,
                    Shape::Rect => {
                        let r = box_of(*from, *to);
                        let corners =
                            [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)];
                        corners
                            .iter()
                            .zip(corners.iter().cycle().skip(1))
                            .any(|(a, b)| seg_dist(p, *a, *b) <= reach)
                    }
                    Shape::Ellipse => {
                        let r = box_of(*from, *to);
                        let c = r.center();
                        let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
                        if rx <= 0.0 || ry <= 0.0 {
                            return seg_dist(p, *from, *to) <= reach;
                        }
                        // Distance along the normalized radius, scaled back to points.
                        let d = (((p.x - c.x) / rx).powi(2) + ((p.y - c.y) / ry).powi(2)).sqrt();
                        (d - 1.0).abs() * rx.min(ry) <= reach
                    }
                }
            }
        }
    }

    /// Moves it by `d` points on its page.
    pub fn translate(&mut self, d: Vec2) {
        let mv = |p: &mut Point| *p = Point::new(p.x + d.x, p.y + d.y);
        match &mut self.kind {
            AnnotationKind::Highlight { quads }
            | AnnotationKind::Underline { quads }
            | AnnotationKind::Strikeout { quads }
            | AnnotationKind::Squiggly { quads } => {
                for q in quads {
                    *q = q.translate(d);
                }
            }
            AnnotationKind::Ink { strokes, .. } => strokes.iter_mut().flatten().for_each(mv),
            AnnotationKind::Note { at } => mv(at),
            AnnotationKind::FreeText { rect, .. } | AnnotationKind::Image { rect, .. } => {
                *rect = rect.translate(d);
            }
            AnnotationKind::Shape { from, to, .. } => {
                mv(from);
                mv(to);
            }
        }
    }
}

impl Annotation {
    /// The box of an annotation that has one of its own (an image, a text box).
    pub fn rect(&self) -> Option<Rect> {
        match &self.kind {
            AnnotationKind::FreeText { rect, .. } | AnnotationKind::Image { rect, .. } => Some(*rect),
            _ => None,
        }
    }

    /// Gives such an annotation another box. `false` for the kinds without one.
    pub fn set_rect(&mut self, to: Rect) -> bool {
        match &mut self.kind {
            AnnotationKind::FreeText { rect, .. } | AnnotationKind::Image { rect, .. } => {
                *rect = to;
                true
            }
            _ => false,
        }
    }
}

/// The edge of a resize handle at a selected image's corner, points.
pub(crate) const HANDLE: f32 = 7.0;
/// The smallest an image is resized to, points.
const SMALLEST: f32 = 4.0;

/// The corners of a box, clockwise from the top left.
fn corners(r: Rect) -> [Point; 4] {
    [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)]
}

/// The corner of a resizable annotation (an image) within `reach` points of `p`: 0 to 3,
/// clockwise from the top left.
pub(crate) fn corner_at(a: &Annotation, p: Point, reach: f32) -> Option<usize> {
    let AnnotationKind::Image { rect, .. } = &a.kind else { return None };
    corners(*rect).iter().position(|c| (c.x - p.x).abs() <= reach && (c.y - p.y).abs() <= reach)
}

/// `rect` with `corner` dragged to `to`: the opposite corner stays, and the box keeps its
/// shape (the larger of the two pulls decides its size).
pub(crate) fn drag_corner(rect: Rect, corner: usize, to: Point) -> Rect {
    let (w, h) = (rect.width().max(f32::MIN_POSITIVE), rect.height().max(f32::MIN_POSITIVE));
    let (left, top) = (corner == 0 || corner == 3, corner == 0 || corner == 1);
    let fixed =
        Point::new(if left { rect.max.x } else { rect.min.x }, if top { rect.max.y } else { rect.min.y });
    let pull_x = if left { fixed.x - to.x } else { to.x - fixed.x };
    let pull_y = if top { fixed.y - to.y } else { to.y - fixed.y };
    let scale = (pull_x / w).max(pull_y / h).max(SMALLEST / w.min(h));
    let (nw, nh) = (w * scale, h * scale);
    Rect::new(
        if left { fixed.x - nw } else { fixed.x },
        if top { fixed.y - nh } else { fixed.y },
        if left { fixed.x } else { fixed.x + nw },
        if top { fixed.y } else { fixed.y + nh },
    )
}

/// A note's icon edge, points.
pub(crate) const NOTE: f32 = 20.0;
/// An arrowhead's length, points (per point of line width, at least this).
const ARROW_HEAD: f32 = 10.0;

fn dist(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn seg_dist(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    dist(p, Point::new(a.x + t * dx, a.y + t * dy))
}

pub(crate) fn box_of(a: Point, b: Point) -> Rect {
    Rect::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
}

fn bounds_of(mut pts: impl Iterator<Item = Point>) -> Option<Rect> {
    let first = pts.next()?;
    Some(pts.fold(Rect::new(first.x, first.y, first.x, first.y), |r, p| {
        Rect::new(r.min.x.min(p.x), r.min.y.min(p.y), r.max.x.max(p.x), r.max.y.max(p.y))
    }))
}

fn union(mut rs: impl Iterator<Item = Rect>) -> Option<Rect> {
    let first = rs.next()?;
    Some(rs.fold(first, |a, b| {
        Rect::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y), a.max.x.max(b.max.x), a.max.y.max(b.max.y))
    }))
}

/// Drops points closer than `tolerance` to the line through their neighbours
/// (Ramer–Douglas–Peucker), keeping a stroke's shape with far fewer points.
pub(crate) fn simplify(points: &[Point], tolerance: f32) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    let last = points.len() - 1;
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.get_mut(last) {
        *k = true;
    }
    let mut stack = vec![(0usize, last)];
    while let Some((a, b)) = stack.pop() {
        let (Some(pa), Some(pb)) = (points.get(a), points.get(b)) else { continue };
        let mut far = (0.0f32, a);
        for (i, p) in points.iter().enumerate().take(b).skip(a + 1) {
            let d = seg_dist(*p, *pa, *pb);
            if d > far.0 {
                far = (d, i);
            }
        }
        if far.0 > tolerance {
            if let Some(k) = keep.get_mut(far.1) {
                *k = true;
            }
            stack.push((a, far.1));
            stack.push((far.1, b));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn round_stroke(width: f32) -> Stroke {
    let mut s = Stroke::new(width);
    s.cap = StrokeCap::Round;
    s.join = StrokeJoin::Round;
    s
}

/// Records `a` in page points (a text box's frame; the view lays out and draws its words);
/// `selected` draws its selection outline in `accent`.
pub(crate) fn paint(list: &mut remedy_ui_engine::paint::DisplayList, a: &Annotation, selected: bool, accent: Color) {
    let color = a.color;
    match &a.kind {
        AnnotationKind::Highlight { quads } => {
            let mut c = color;
            c.a *= 0.4;
            for q in quads {
                list.fill_rect(*q, Brush::Solid(c));
            }
        }
        AnnotationKind::Underline { quads } => {
            for q in quads {
                let w = (q.height() * 0.07).max(0.5);
                list.fill_rect(Rect::new(q.min.x, q.max.y - w, q.max.x, q.max.y), Brush::Solid(color));
            }
        }
        AnnotationKind::Strikeout { quads } => {
            for q in quads {
                let w = (q.height() * 0.07).max(0.5);
                let mid = (q.min.y + q.max.y) / 2.0;
                list.fill_rect(
                    Rect::new(q.min.x, mid - w / 2.0, q.max.x, mid + w / 2.0),
                    Brush::Solid(color),
                );
            }
        }
        AnnotationKind::Squiggly { quads } => {
            for q in quads {
                let amp = (q.height() * 0.06).max(0.5);
                let step = amp * 2.0;
                let y = q.max.y - amp;
                let mut path = Path::new();
                path.move_to(Point::new(q.min.x, y));
                let mut x = q.min.x;
                let mut up = true;
                while x < q.max.x {
                    let nx = (x + step).min(q.max.x);
                    path.line_to(Point::new(nx, if up { y - amp } else { y + amp }));
                    up = !up;
                    x = nx;
                }
                list.push(Item::StrokePath {
                    path,
                    stroke: round_stroke(amp * 0.8),
                    brush: Brush::Solid(color),
                });
            }
        }
        AnnotationKind::Ink { strokes, width } => {
            for s in strokes {
                let mut path = Path::new();
                match s.as_slice() {
                    [] => continue,
                    [one] => {
                        // A dot.
                        path.move_to(*one);
                        path.line_to(Point::new(one.x + 0.01, one.y));
                    }
                    [first, rest @ ..] => {
                        path.move_to(*first);
                        for p in rest {
                            path.line_to(*p);
                        }
                    }
                }
                list.push(Item::StrokePath {
                    path,
                    stroke: round_stroke(*width),
                    brush: Brush::Solid(color),
                });
            }
        }
        AnnotationKind::Note { at } => {
            let r = Rect::from_origin_size(*at, remedy_ui_core::Size::new(NOTE, NOTE));
            list.fill_rounded(r, CornerRadii::all(2.0), Brush::Solid(color));
            list.border(r, CornerRadii::all(2.0), 1.0, Color::rgba8(0, 0, 0, 90));
            // Lines of writing on the note.
            let ink = Color::rgba8(0, 0, 0, 140);
            for i in 1..4 {
                let y = r.min.y + NOTE * (i as f32) / 4.5 + 2.0;
                list.fill_rect(Rect::new(r.min.x + 4.0, y, r.max.x - 4.0, y + 1.2), Brush::Solid(ink));
            }
        }
        AnnotationKind::FreeText { rect, .. } => {
            list.border(*rect, CornerRadii::ZERO, 0.75, color);
        }
        AnnotationKind::Image { rect, image } => {
            list.push(Item::Image {
                image: image.clone(),
                rect: *rect,
                radii: CornerRadii::ZERO,
                sampling: Sampling::Linear,
            });
        }
        AnnotationKind::Shape { shape, from, to, width } => {
            let stroke = round_stroke(*width);
            let brush = Brush::Solid(color);
            let r = box_of(*from, *to);
            let mut path = Path::new();
            match shape {
                Shape::Rect => {
                    path.move_to(r.min);
                    path.line_to(Point::new(r.max.x, r.min.y));
                    path.line_to(r.max);
                    path.line_to(Point::new(r.min.x, r.max.y));
                    path.close();
                }
                Shape::Ellipse => {
                    // Four cubic arcs.
                    let c = r.center();
                    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
                    let k = 0.552_284_8;
                    path.move_to(Point::new(c.x + rx, c.y));
                    path.cubic_to(
                        Point::new(c.x + rx, c.y + ry * k),
                        Point::new(c.x + rx * k, c.y + ry),
                        Point::new(c.x, c.y + ry),
                    );
                    path.cubic_to(
                        Point::new(c.x - rx * k, c.y + ry),
                        Point::new(c.x - rx, c.y + ry * k),
                        Point::new(c.x - rx, c.y),
                    );
                    path.cubic_to(
                        Point::new(c.x - rx, c.y - ry * k),
                        Point::new(c.x - rx * k, c.y - ry),
                        Point::new(c.x, c.y - ry),
                    );
                    path.cubic_to(
                        Point::new(c.x + rx * k, c.y - ry),
                        Point::new(c.x + rx, c.y - ry * k),
                        Point::new(c.x + rx, c.y),
                    );
                    path.close();
                }
                Shape::Line | Shape::Arrow => {
                    path.move_to(*from);
                    path.line_to(*to);
                    if *shape == Shape::Arrow {
                        let len = dist(*from, *to);
                        if len > 0.0 {
                            let (ux, uy) = ((to.x - from.x) / len, (to.y - from.y) / len);
                            let head = (ARROW_HEAD).max(width * 3.0).min(len);
                            let back = Point::new(to.x - ux * head, to.y - uy * head);
                            let (nx, ny) = (-uy * head * 0.5, ux * head * 0.5);
                            path.move_to(Point::new(back.x + nx, back.y + ny));
                            path.line_to(*to);
                            path.line_to(Point::new(back.x - nx, back.y - ny));
                        }
                    }
                }
            }
            list.push(Item::StrokePath { path, stroke, brush });
        }
    }
    if selected {
        let b = a.bounds();
        let pad = 3.0;
        let r = Rect::new(b.min.x - pad, b.min.y - pad, b.max.x + pad, b.max.y + pad);
        let mut s = Stroke::new(1.0);
        s.dashes = vec![4.0, 3.0];
        let mut path = Path::new();
        path.move_to(r.min);
        path.line_to(Point::new(r.max.x, r.min.y));
        path.line_to(r.max);
        path.line_to(Point::new(r.min.x, r.max.y));
        path.close();
        list.push(Item::StrokePath { path, stroke: s, brush: Brush::Solid(accent) });
        // An image is resized by its corners.
        if let AnnotationKind::Image { rect, .. } = &a.kind {
            let half = HANDLE / 2.0;
            for c in corners(*rect) {
                let knob = Rect::new(c.x - half, c.y - half, c.x + half, c.y + half);
                list.fill_rect(knob, Brush::Solid(Color::WHITE));
                list.border(knob, CornerRadii::ZERO, 1.0, accent);
            }
        }
    }
}

/// The tools for making annotations; `Select` is the default (select text and annotations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Tool {
    /// Select text, and annotations by pressing them.
    #[default]
    Select,
    /// Drag to move around.
    Hand,
    /// Mark selected text with a highlight.
    Highlight,
    /// Underline selected text.
    Underline,
    /// Strike selected text through.
    Strikeout,
    /// Underline selected text with a wavy line.
    Squiggly,
    /// Draw freehand.
    Ink,
    /// Pin a note where pressed.
    Note,
    /// Drag a box to write in.
    FreeText,
    /// Drag a shape.
    Shape(Shape),
}

/// How new annotations look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnnotationStyle {
    /// Their color.
    pub color: Color,
    /// Ink and shape line width, points.
    pub width: f32,
    /// Text box text size, points.
    pub text_size: f32,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        AnnotationStyle { color: Color::rgba8(255, 214, 0, 255), width: 2.0, text_size: 12.0 }
    }
}
