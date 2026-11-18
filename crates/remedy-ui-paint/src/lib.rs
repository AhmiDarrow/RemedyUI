//! RemedyUI display lists.
//!
//! Elements record what they draw as [`Item`]s in a [`DisplayList`]: plain data with no
//! pointers into the tree, so a list can be cached per element, shared between frames, sent
//! to the compositor thread and drawn by the GPU or CPU renderer with the same result. See
//! `docs/design/04-rendering.md`.
//!
//! ```
//! use remedy_ui_core::{Color, CornerRadii, Rect};
//! use remedy_ui_paint::*;
//!
//! let mut list = DisplayList::new();
//! list.fill_rounded(Rect::new(0.0, 0.0, 120.0, 28.0), CornerRadii::all(3.0), Brush::Solid(Color::rgb8(77, 122, 90)));
//! list.push_clip(Rect::new(4.0, 4.0, 116.0, 24.0), CornerRadii::ZERO);
//! list.pop_clip();
//! assert_eq!(list.len(), 3);
//! assert_eq!(list.bounds(), Some(Rect::new(0.0, 0.0, 120.0, 28.0)));
//! ```

mod path;
mod video;

pub use path::{Path, PathEl, Stroke, StrokeCap, StrokeJoin};
pub use video::{Coefficients, PixelFormat, Plane, VideoFrame, YuvMatrix};

use std::sync::Arc;

use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_text::{Antialias, Hinting, RunFont};

/// A 2D affine transform: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub [f32; 6]);

impl Affine {
    /// No change.
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// A move.
    pub fn translate(v: Vec2) -> Self {
        Affine([1.0, 0.0, 0.0, 1.0, v.x, v.y])
    }

    /// A scale about the origin.
    pub fn scale(sx: f32, sy: f32) -> Self {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// A rotation about the origin, in radians.
    pub fn rotate(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    /// `self` then `other`.
    pub fn then(self, other: Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Affine([
            a2 * a + c2 * b,
            b2 * a + d2 * b,
            a2 * c + c2 * d,
            b2 * c + d2 * d,
            a2 * e + c2 * f + e2,
            b2 * e + d2 * f + f2,
        ])
    }

    /// Applies the transform to a point.
    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// The bounding box of a transformed rectangle.
    pub fn apply_rect(&self, r: Rect) -> Rect {
        let pts =
            [r.min, Point::new(r.max.x, r.min.y), r.max, Point::new(r.min.x, r.max.y)].map(|p| self.apply(p));
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Rect::new(x0, y0, x1, y1)
    }

    /// The transform that undoes this one, or `None` when it collapses space (zero scale).
    pub fn inverse(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv = 1.0 / det;
        let (ia, ib, ic, id) = (d * inv, -b * inv, -c * inv, a * inv);
        Some(Affine([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }

    /// How much the transform scales lengths: the mean of its axis scales.
    pub fn scale_factor(&self) -> f32 {
        let [a, b, c, d, _, _] = self.0;
        ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0
    }

    /// True when the transform only moves.
    pub fn is_translation(&self) -> bool {
        let [a, b, c, d, _, _] = self.0;
        a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0
    }
}

/// A color stop of a gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// Position along the gradient, 0 to 1.
    pub offset: f32,
    /// Color there.
    pub color: Color,
}

/// What fills a shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    /// One color.
    Solid(Color),
    /// A linear gradient from `start` to `end`.
    Linear {
        /// Where offset 0 is.
        start: Point,
        /// Where offset 1 is.
        end: Point,
        /// The stops, in order.
        stops: Vec<Stop>,
    },
    /// A radial gradient around `center`.
    Radial {
        /// The center.
        center: Point,
        /// The radius at offset 1.
        radius: f32,
        /// The stops, in order.
        stops: Vec<Stop>,
    },
}

/// Pixels to draw, premultiplied RGBA8, shared without copying.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    /// A stable id for caches (the same pixels, the same id).
    pub id: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Premultiplied RGBA8 pixels, row by row.
    pub pixels: Arc<[u8]>,
}

impl Image {
    /// An image of premultiplied RGBA8 `pixels` (`width * height * 4` bytes) with a fresh
    /// id of its own. Renderers skip an image whose pixels do not match its size.
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Image {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Image { id, width, height, pixels }
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({} {}x{})", self.id, self.width, self.height)
    }
}

/// How images are sampled when scaled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampling {
    /// Smooth (bilinear).
    #[default]
    Linear,
    /// Hard pixels (pixel art, Ahmi Pixel).
    Nearest,
}

/// How a layer combines with what is below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    /// Normal source-over.
    #[default]
    Normal,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
}

/// A run of glyphs to draw.
#[derive(Clone, Debug)]
pub struct GlyphRun {
    /// The font.
    pub font: RunFont,
    /// Font size in logical pixels.
    pub size: f32,
    /// Fill color.
    pub color: Color,
    /// Glyph ids and origins (on the baseline), in logical pixels.
    pub glyphs: Vec<(u32, Point)>,
    /// Grid fitting.
    pub hinting: Hinting,
    /// Grayscale or LCD coverage. LCD is for text over opaque content, as on every platform;
    /// renderers fall back to grayscale under rotation or skew and inside opacity or blend
    /// groups.
    pub antialias: Antialias,
}

impl PartialEq for GlyphRun {
    fn eq(&self, o: &Self) -> bool {
        self.font.id() == o.font.id()
            && self.font.index() == o.font.index()
            && self.size == o.size
            && self.color == o.color
            && self.glyphs == o.glyphs
            && self.hinting == o.hinting
            && self.antialias == o.antialias
    }
}

/// One drawing command.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A filled rectangle with rounded corners.
    FillRect {
        /// The rectangle.
        rect: Rect,
        /// Corner radii (fitted to the rectangle by the renderer).
        radii: CornerRadii,
        /// The fill.
        brush: Brush,
    },
    /// A border inside a rounded rectangle's edge.
    Border {
        /// The outer edge.
        rect: Rect,
        /// Corner radii of the outer edge.
        radii: CornerRadii,
        /// Width, inward.
        width: f32,
        /// Color.
        color: Color,
    },
    /// A soft shadow of a rounded rectangle (drawn outside it).
    Shadow {
        /// The shape casting it.
        rect: Rect,
        /// Its corner radii.
        radii: CornerRadii,
        /// Shadow offset.
        offset: Vec2,
        /// Blur radius.
        blur: f32,
        /// Growth before blurring.
        spread: f32,
        /// Color.
        color: Color,
    },
    /// A filled path.
    FillPath {
        /// The path.
        path: Path,
        /// The fill.
        brush: Brush,
        /// True for the even-odd rule, false for non-zero.
        even_odd: bool,
    },
    /// A stroked path.
    StrokePath {
        /// The path.
        path: Path,
        /// The stroke.
        stroke: Stroke,
        /// The color.
        brush: Brush,
    },
    /// Glyphs.
    Glyphs(GlyphRun),
    /// An image placed into `rect`.
    Image {
        /// The image.
        image: Image,
        /// Where it goes.
        rect: Rect,
        /// Rounded corners.
        radii: CornerRadii,
        /// Sampling.
        sampling: Sampling,
    },
    /// A video frame placed into `rect`, converted to the screen's colors by the renderer.
    Video {
        /// The frame.
        frame: VideoFrame,
        /// Where it goes.
        rect: Rect,
        /// Rounded corners.
        radii: CornerRadii,
    },
    /// Starts clipping to a rounded rectangle.
    PushClip {
        /// The clip.
        rect: Rect,
        /// Its corners.
        radii: CornerRadii,
    },
    /// Ends the last clip.
    PopClip,
    /// Starts a group drawn with opacity and blending.
    PushLayer {
        /// Opacity, 0 to 1.
        opacity: f32,
        /// Blend mode.
        blend: Blend,
    },
    /// Ends the last layer.
    PopLayer,
    /// Starts a transform.
    PushTransform(Affine),
    /// Ends the last transform.
    PopTransform,
}

impl Item {
    /// The area a drawing item can touch, in its own space (before transforms and clips).
    /// `None` for items that only push or pop state.
    pub fn bounds(&self) -> Option<Rect> {
        match self {
            Item::FillRect { rect, .. }
            | Item::Border { rect, .. }
            | Item::Image { rect, .. }
            | Item::Video { rect, .. } => Some(*rect),
            Item::Shadow { rect, offset, blur, spread, .. } => {
                let grow = blur * 1.5 + spread;
                Some(
                    Rect::new(rect.min.x - grow, rect.min.y - grow, rect.max.x + grow, rect.max.y + grow)
                        .translate(*offset),
                )
            }
            Item::FillPath { path, .. } => path.bounds(),
            Item::StrokePath { path, stroke, .. } => path.bounds().map(|b| {
                let h = stroke.width / 2.0 + stroke.width * 2.0;
                Rect::new(b.min.x - h, b.min.y - h, b.max.x + h, b.max.y + h)
            }),
            Item::Glyphs(run) => glyph_bounds(run),
            Item::PushClip { .. }
            | Item::PopClip
            | Item::PushLayer { .. }
            | Item::PopLayer
            | Item::PushTransform(_)
            | Item::PopTransform => None,
        }
    }
}

/// A list of drawing commands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    items: Vec<Item>,
}

impl DisplayList {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// The items.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when nothing is drawn.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Empties the list, keeping its memory for reuse.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Adds an item.
    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Appends another list.
    pub fn extend(&mut self, other: &DisplayList) {
        self.items.extend_from_slice(&other.items);
    }

    /// Fills a rectangle.
    pub fn fill_rect(&mut self, rect: Rect, brush: Brush) {
        self.push(Item::FillRect { rect, radii: CornerRadii::ZERO, brush });
    }

    /// Fills a rounded rectangle.
    pub fn fill_rounded(&mut self, rect: Rect, radii: CornerRadii, brush: Brush) {
        self.push(Item::FillRect { rect, radii, brush });
    }

    /// Draws a border.
    pub fn border(&mut self, rect: Rect, radii: CornerRadii, width: f32, color: Color) {
        self.push(Item::Border { rect, radii, width, color });
    }

    /// Starts a clip.
    pub fn push_clip(&mut self, rect: Rect, radii: CornerRadii) {
        self.push(Item::PushClip { rect, radii });
    }

    /// Ends a clip.
    pub fn pop_clip(&mut self) {
        self.push(Item::PopClip);
    }

    /// The area the list can draw to, without transforms' rotation (transforms are applied
    /// to bounds as boxes). `None` for a list that draws nothing.
    pub fn bounds(&self) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut transforms = vec![Affine::IDENTITY];
        let mut clips: Vec<Rect> = Vec::new();
        for item in &self.items {
            let t = transforms.last().copied().unwrap_or(Affine::IDENTITY);
            let r = match item {
                Item::PushClip { rect, .. } => {
                    clips.push(t.apply_rect(*rect));
                    None
                }
                Item::PopClip => {
                    clips.pop();
                    None
                }
                Item::PushTransform(a) => {
                    transforms.push(a.then(t));
                    None
                }
                Item::PopTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                    None
                }
                Item::PushLayer { .. } | Item::PopLayer => None,
                drawn => drawn.bounds(),
            };
            if let Some(r) = r {
                let mut r = t.apply_rect(r);
                for c in &clips {
                    r = r.intersect(*c).unwrap_or(Rect::ZERO);
                }
                if !r.is_empty() {
                    out = Some(out.map_or(r, |o| o.union(r)));
                }
            }
        }
        out
    }
}

/// A conservative box for glyphs: each glyph's em box around its origin.
fn glyph_bounds(run: &GlyphRun) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for (_, p) in &run.glyphs {
        let r =
            Rect::new(p.x - run.size * 0.5, p.y - run.size * 1.2, p.x + run.size * 1.5, p.y + run.size * 0.5);
        out = Some(out.map_or(r, |o| o.union(r)));
    }
    out
}
