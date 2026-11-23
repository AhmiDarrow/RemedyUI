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

mod limits;
mod path;
mod video;

pub use limits::{PaintLimits, paint_limits, set_paint_limits};
pub use path::{Path, PathEl, Stroke, StrokeCap, StrokeJoin};
pub use remedy_ui_core::ColorMatrix;
pub use video::{
    Coefficients, CpuPixels, DRM_FORMAT_MOD_LINEAR, DRM_FORMAT_NV12, DmaBufPlane, GpuAdapter, GpuFrame,
    GpuPicture, GpuTexture, PagePlacement, PixelFormat, Plane, VideoFrame, YuvMatrix, gpu_video_adapter,
    gpu_video_dma_buf_modifiers, set_gpu_video_adapter, set_gpu_video_dma_buf_modifiers,
};

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
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
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
    /// An elliptical radial gradient around `center` (CSS `radial-gradient(ellipse ...)`):
    /// offset 1 is the axis-aligned ellipse with half-widths `radii.x` and `radii.y`.
    Elliptical {
        /// The center.
        center: Point,
        /// The ellipse's horizontal and vertical radii at offset 1.
        radii: Vec2,
        /// The stops, in order.
        stops: Vec<Stop>,
    },
    /// A conic (sweep) gradient around `center` (CSS `conic-gradient`): offset 0 at
    /// `start_angle`, going clockwise to offset 1 a full turn later.
    Conic {
        /// The center.
        center: Point,
        /// Where offset 0 is, in radians from the positive x axis, clockwise (y points down).
        start_angle: f32,
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
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum Sampling {
    /// Smooth (bilinear).
    #[default]
    Linear,
    /// Hard pixels (pixel art, Ahmi Pixel).
    Nearest,
}

/// How a layer combines with what is below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum Blend {
    /// Normal source-over.
    #[default]
    Normal,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
    /// Additive ("plus", linear dodge): adds its light to what is below. Glows, light pools
    /// and beams stack the way light does, with no shader.
    Plus,
}

/// A filter on everything drawn between [`Item::PushFilter`] and [`Item::PopFilter`], applied
/// to the group as one picture after it is drawn, in the group's own space (a scaled window
/// blurs by as many device pixels as it scales). Both renderers draw it with the same
/// Gaussian; see [`Filter::outset`] for how far it reaches past what the group draws.
#[derive(Clone, Copy, Debug, PartialEq)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum Filter {
    /// A Gaussian blur of the group (CSS `filter: blur(radius)`): `radius` is the standard
    /// deviation, and the blur reaches about three times as far.
    Blur {
        /// The standard deviation, in the group's space. 0 draws the group unchanged.
        radius: f32,
    },
    /// A shadow of whatever shape the group draws (text, icons, paths, images with
    /// transparency) under it (CSS `filter: drop-shadow(...)`): the group's coverage, blurred,
    /// moved by `offset` and filled with `color`, then the group itself on top.
    DropShadow {
        /// How far the shadow falls, in the group's space.
        offset: Vec2,
        /// The blur radius as CSS gives it: twice the standard deviation.
        blur: f32,
        /// The shadow's color (its alpha scales the shadow).
        color: Color,
    },
    /// A colour matrix on every pixel the group draws (CSS `saturate()`, `hue-rotate()`,
    /// `grayscale()`, `sepia()`, `brightness()`, `contrast()`, `invert()`; SVG
    /// `feColorMatrix`): the group is drawn to pixels of its own, unpremultiplied, run
    /// through the matrix, clamped and premultiplied again. It reaches no further than the
    /// group draws, and pixels the group leaves transparent stay transparent.
    ColorMatrix {
        /// The matrix ([`ColorMatrix`]'s presets build the CSS ones).
        matrix: ColorMatrix,
    },
}

/// The most a filter blurs, as a standard deviation in the filter's own units (device pixels
/// at scale 1 or less). A filter's buffers grow by about three times this on every side, so a
/// stranger's theme asking for a blur of a million pixels gets this, not a buffer of a million
/// pixels. It is not divided by a high-DPI scale: a blur looks the same at 100% and 200%.
/// The default of [`PaintLimits::filter_std_dev`].
pub const MAX_FILTER_STD_DEV: f32 = 64.0;
/// The farthest a drop shadow falls, in device pixels, on each axis. The default of
/// [`PaintLimits::filter_offset`].
pub const MAX_FILTER_OFFSET: f32 = 2048.0;
/// The most groups (clips, opacity and blend layers, filters, and the layers glass draws
/// through) a renderer keeps open at once. Renderers nest one surface layer per group, and the
/// GPU's layer handling recurses per level, so a list from a stranger nesting thousands of
/// groups would overflow the stack. A push past this opens nothing and its pop closes nothing:
/// what it holds is drawn as if the group were not there (unclipped, unfaded, unfiltered),
/// and glass that deep is not drawn. Real interfaces nest a few dozen. This is a safety bound,
/// not a capacity limit: the recursion is in vello, outside RemedyUI's control, and the bound
/// keeps it within any thread's stack. RemedyUI's own walks (the compositor's layer tree)
/// use explicit stacks and take any depth. The default of [`PaintLimits::group_depth`].
pub const MAX_GROUP_DEPTH: usize = 256;
/// How deep glass seen through glass is drawn: glass ([`Item::Backdrop`]) draws again what
/// is under it, and glass under glass draws again what is under that, so a stack of panes
/// would cost twice as much for every pane. Past this depth a pane seen through others is
/// drawn without its frost. The default of [`PaintLimits::backdrop_depth`].
pub const MAX_BACKDROP_DEPTH: usize = 2;
/// The most panes of glass drawn with their frost in one raster (a tile, a frame); the rest
/// are drawn without it, so a theme stacking glass cannot stall a frame. The default of
/// [`PaintLimits::backdrop_replays`].
pub const MAX_BACKDROP_REPLAYS: usize = 128;
/// The most colour-matrix groups ([`Filter::ColorMatrix`]) renderers draw nested inside one
/// another. Each open one holds a picture of its own, so a theme nesting a thousand of them
/// does not hold a thousand pictures: groups deeper than this are drawn without their matrix.
/// The default of [`PaintLimits::filter_matrix_depth`].
pub const MAX_FILTER_MATRIX_DEPTH: usize = 4;
/// The most colour-matrix groups a renderer draws through their matrix in one raster (a tile,
/// a snapshot). Each costs a picture and a pass of its own, so a theme putting one on every
/// row of a long list is bounded: the groups past this many are drawn without their matrix.
/// The default of [`PaintLimits::filter_matrix_groups`].
pub const MAX_FILTER_MATRIX_GROUPS: usize = 64;

/// How much blur and offset a chain of nested filters has left to draw with.
///
/// [`MAX_FILTER_STD_DEV`] and [`MAX_FILTER_OFFSET`] bound one filter, but a filter's buffers
/// hold everything nested in it: every filter in a chain grows them again by its own reach,
/// and blurs them all again. A stranger's theme nesting shadows at the bound would make each
/// level's buffer thousands of pixels wider than the last, past what a GPU can hold and with
/// work that grows with the square of the depth. So the bounds hold for the whole chain: a
/// filter nested in others gets what they left, and the chain reaches no farther than one
/// filter at the bound. Filters side by side each start from what their common parents left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterBudget {
    /// How much farther shadows may fall, in device pixels, on each axis.
    offset: Vec2,
    /// How much more blur, as a standard deviation in [`MAX_FILTER_STD_DEV`]'s units.
    std_dev: f32,
}

impl FilterBudget {
    /// All of it at the default bounds ([`PaintLimits::DEFAULT`]).
    pub const FULL: FilterBudget = FilterBudget {
        offset: Vec2 { x: MAX_FILTER_OFFSET, y: MAX_FILTER_OFFSET },
        std_dev: MAX_FILTER_STD_DEV,
    };

    /// All of it: what a filter with no filters around it may use, at the bounds set now
    /// ([`set_paint_limits`]).
    pub fn full() -> FilterBudget {
        FilterBudget::of(&paint_limits())
    }

    /// All of `limits`' blur and offset (a bound that is not finite or is negative counts as 0).
    pub fn of(limits: &PaintLimits) -> FilterBudget {
        let clean = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let offset = clean(limits.filter_offset);
        FilterBudget { offset: Vec2 { x: offset, y: offset }, std_dev: clean(limits.filter_std_dev) }
    }

    /// `filter` as renderers draw it under a transform scaling lengths by `scale`, within
    /// what is left, and what is left after it for the filters nested inside it.
    pub fn limit(self, filter: &Filter, scale: f32) -> (Filter, FilterBudget) {
        let k = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        // The blur bound is in the filter's own units, or device pixels when zoomed out:
        // a blur looks the same at 100% and 200%.
        let unit = k.min(1.0);
        let sigma = filter.std_dev().min(self.std_dev / unit);
        let std_dev = (self.std_dev - sigma * unit).max(0.0);
        match *filter {
            Filter::Blur { .. } => (Filter::Blur { radius: sigma }, FilterBudget { std_dev, ..self }),
            // A matrix neither blurs nor moves: it takes none of the budget.
            Filter::ColorMatrix { matrix } => (Filter::ColorMatrix { matrix: matrix.limited() }, self),
            Filter::DropShadow { offset, color, .. } => {
                let o = |v: f32, left: f32| if v.is_finite() { v.clamp(-left / k, left / k) } else { 0.0 };
                let offset = Vec2::new(o(offset.x, self.offset.x), o(offset.y, self.offset.y));
                let left = Vec2::new(
                    (self.offset.x - offset.x.abs() * k).max(0.0),
                    (self.offset.y - offset.y.abs() * k).max(0.0),
                );
                (
                    Filter::DropShadow { offset, blur: sigma * 2.0, color },
                    FilterBudget { offset: left, std_dev },
                )
            }
        }
    }
}

impl Default for FilterBudget {
    fn default() -> Self {
        FilterBudget::full()
    }
}

impl Filter {
    /// The filter as renderers draw it under a transform scaling lengths by `scale`: its
    /// blur at most [`PaintLimits::filter_std_dev`] (in device pixels when `scale` is below 1)
    /// and its offset at most [`PaintLimits::filter_offset`] device pixels, whatever was
    /// asked. Nested in other filters it gets less: see [`FilterBudget`].
    pub fn limited(&self, scale: f32) -> Filter {
        FilterBudget::full().limit(self, scale).0
    }

    /// The Gaussian's standard deviation, in the group's space.
    pub fn std_dev(&self) -> f32 {
        let s = match self {
            Filter::Blur { radius } => *radius,
            Filter::DropShadow { blur, .. } => *blur / 2.0,
            Filter::ColorMatrix { .. } => 0.0,
        };
        if s.is_finite() { s.max(0.0) } else { 0.0 }
    }

    /// How far past the group's own drawing the filter can draw: three standard deviations
    /// (where a Gaussian has all but 0.3% of its weight) on each side, and the shadow's offset.
    pub fn outset(&self, r: Rect) -> Rect {
        if let Filter::ColorMatrix { .. } = self {
            return r;
        }
        let reach = (self.std_dev() * 3.0).ceil();
        let grown = Rect::new(r.min.x - reach, r.min.y - reach, r.max.x + reach, r.max.y + reach);
        match self {
            Filter::Blur { .. } | Filter::ColorMatrix { .. } => grown,
            Filter::DropShadow { offset, .. } if offset.x.is_finite() && offset.y.is_finite() => {
                r.union(grown.translate(*offset))
            }
            Filter::DropShadow { .. } => grown,
        }
    }

    /// The reverse of [`outset`](Self::outset): the area whose drawing the filter can carry
    /// into `r` (a tile draws what is within it).
    pub fn inset_source(&self, r: Rect) -> Rect {
        if let Filter::ColorMatrix { .. } = self {
            return r;
        }
        let reach = (self.std_dev() * 3.0).ceil();
        let grown = Rect::new(r.min.x - reach, r.min.y - reach, r.max.x + reach, r.max.y + reach);
        match self {
            Filter::Blur { .. } | Filter::ColorMatrix { .. } => grown,
            Filter::DropShadow { offset, .. } if offset.x.is_finite() && offset.y.is_finite() => {
                r.union(grown.translate(Vec2::new(-offset.x, -offset.y)))
            }
            Filter::DropShadow { .. } => grown,
        }
    }

    /// True when the filter changes nothing (a blur of zero, the identity matrix).
    pub fn is_identity(&self) -> bool {
        match self {
            Filter::Blur { .. } => self.std_dev() == 0.0,
            Filter::ColorMatrix { matrix } => matrix.is_identity(),
            Filter::DropShadow { .. } => false,
        }
    }
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

/// A texture registered with the GPU renderer's device under `id` (premultiplied RGBA8),
/// showing picture number `frame`. Plain data: the texture itself stays with the device, so
/// a list holding this can still go to the compositor thread, and a new picture in the same
/// texture is a new `frame`, which makes the item differ and the renderer draw it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExternalTexture {
    /// The registration id.
    pub id: u64,
    /// The picture in it, counting up.
    pub frame: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// One drawing command.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
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
    /// A texture the app rendered on the GPU renderer's own device, placed into `rect`.
    /// Only the GPU renderer can draw it; content shown on the CPU renderer is read back
    /// and drawn as an [`Item::Image`] instead.
    Texture {
        /// Which texture, and which picture in it.
        texture: ExternalTexture,
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
    /// Starts a group drawn through `Filter` (a blur, a drop shadow of its shape), ended by
    /// [`Item::PopFilter`].
    PushFilter(Filter),
    /// Ends the last filter group.
    PopFilter,
    /// Blurs what is already drawn under a rounded rectangle (CSS `backdrop-filter: blur()`):
    /// frosted glass. It sees what was drawn before it since the start of the list or of the
    /// innermost open [`Item::PushLayer`] or [`Item::PushFilter`] group around it (its
    /// backdrop root, as on the web), and draws nothing elsewhere. Elements in a window get
    /// theirs from the compositor instead (`remedy-ui-compositor`'s layer backdrops), which
    /// sees every layer below; this item is for display lists drawn on their own.
    Backdrop {
        /// The glass.
        rect: Rect,
        /// Its corners.
        radii: CornerRadii,
        /// The blur's standard deviation (CSS `blur(radius)`).
        blur: f32,
    },
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
            | Item::Video { rect, .. }
            | Item::Texture { rect, .. }
            | Item::Backdrop { rect, .. } => Some(*rect),
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
            | Item::PushFilter(_)
            | Item::PopFilter
            | Item::PushTransform(_)
            | Item::PopTransform => None,
        }
    }
}

/// How an item changes the group depth: a push opens one, a pop closes one.
fn depth_step(item: &Item) -> i32 {
    match item {
        Item::PushClip { .. } | Item::PushLayer { .. } | Item::PushFilter(_) | Item::PushTransform(_) => 1,
        Item::PopClip | Item::PopLayer | Item::PopFilter | Item::PopTransform => -1,
        _ => 0,
    }
}

/// `items[start..end]` widened to whole groups: back to the pushes of the pops it holds,
/// on to the pops of the pushes it holds.
fn whole_groups(items: &[Item], mut start: usize, mut end: usize) -> (usize, usize) {
    let (mut depth, mut low) = (0, 0);
    for item in items.get(start..end).unwrap_or(&[]) {
        depth += depth_step(item);
        low = low.min(depth);
    }
    let mut need = -low;
    while need > 0 && start > 0 {
        start -= 1;
        need -= items.get(start).map_or(0, depth_step);
    }
    let mut open = depth - low;
    while open > 0 && end < items.len() {
        open += items.get(end).map_or(0, depth_step);
        end += 1;
    }
    (start, end)
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

    /// Starts a filter group: what follows, until [`pop_filter`](Self::pop_filter), is drawn
    /// through `filter`.
    pub fn push_filter(&mut self, filter: Filter) {
        self.push(Item::PushFilter(filter));
    }

    /// Ends a filter group.
    pub fn pop_filter(&mut self) {
        self.push(Item::PopFilter);
    }

    /// Where `new` draws differently from `old`, in the lists' space: the area the run of
    /// items between what both share at the start and at the end draws to, in either list,
    /// widened to whole groups so the shared items around it draw as before. An empty rect
    /// when they draw the same. `None` when the change cannot be bounded: it sits inside a
    /// filter group, or a backdrop at or after it reads what changed.
    ///
    /// A spinner or a video frame changing in a list that fills a window changes only its
    /// own box, so only the tiles there need drawing again.
    pub fn changed_area(old: &DisplayList, new: &DisplayList) -> Option<Rect> {
        let (a, b) = (old.items(), new.items());
        let mut p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        let most = a.len().min(b.len()) - p;
        let mut s = a.iter().rev().zip(b.iter().rev()).take(most).take_while(|(x, y)| x == y).count();
        // Prefix and suffix are shared, so widening one list widens the other alike; repeat
        // until both runs are whole groups.
        loop {
            let (pa, ea) = whole_groups(a, p, a.len() - s);
            let (pb, eb) = whole_groups(b, p, b.len() - s);
            let (np, ns) = (pa.min(pb), (a.len() - ea).min(b.len() - eb));
            if (np, ns) == (p, s) {
                break;
            }
            (p, s) = (np, ns);
        }
        let area = |items: &[Item], end: usize| -> Option<Option<Rect>> {
            if items.get(p..).unwrap_or(&[]).iter().any(|i| matches!(i, Item::Backdrop { .. })) {
                return None;
            }
            // The transform and clips the run is drawn under, from the shared prefix.
            let mut transforms = vec![Affine::IDENTITY];
            let mut clips: Vec<Option<Rect>> = Vec::new();
            let mut filters = 0usize;
            for item in items.get(..p).unwrap_or(&[]) {
                let t = transforms.last().copied().unwrap_or(Affine::IDENTITY);
                match item {
                    Item::PushClip { rect, .. } => clips.push(Some(t.apply_rect(*rect))),
                    Item::PushLayer { .. } => clips.push(None),
                    Item::PopClip | Item::PopLayer => drop(clips.pop()),
                    Item::PushTransform(m) => transforms.push(m.then(t)),
                    Item::PopTransform if transforms.len() > 1 => drop(transforms.pop()),
                    Item::PushFilter(_) => filters += 1,
                    Item::PopFilter => filters = filters.saturating_sub(1),
                    _ => {}
                }
            }
            if filters > 0 {
                return None;
            }
            let t = transforms.last().copied().unwrap_or(Affine::IDENTITY);
            let run = items.get(p..end).unwrap_or(&[]);
            Some(DisplayList::bounds_of(run).map(|r| {
                clips.iter().flatten().fold(t.apply_rect(r), |r, c| r.intersect(*c).unwrap_or(Rect::ZERO))
            }))
        };
        let (x, y) = (area(a, a.len() - s)?, area(b, b.len() - s)?);
        Some(match (x, y) {
            (Some(x), Some(y)) => x.union(y),
            (r, None) | (None, r) => r.unwrap_or(Rect::ZERO),
        })
    }

    /// The area the list can draw to, without transforms' rotation (transforms are applied
    /// to bounds as boxes). What a filter group draws is grown by how far its filter reaches
    /// ([`Filter::outset`]). `None` for a list that draws nothing.
    pub fn bounds(&self) -> Option<Rect> {
        DisplayList::bounds_of(&self.items)
    }

    /// [`bounds`](Self::bounds) of a run of items, such as the inside of one group.
    pub fn bounds_of(items: &[Item]) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut transforms = vec![Affine::IDENTITY];
        let mut clips: Vec<Rect> = Vec::new();
        // Open filter groups: the filter, the transform it was pushed under, the clips then
        // open, and the bounds of what it holds so far (in that transform's space).
        let mut filters: Vec<(Filter, Affine, usize, Option<Rect>)> = Vec::new();
        fn add(out: &mut Option<Rect>, r: Rect, clips: &[Rect]) {
            let mut r = r;
            for c in clips {
                r = r.intersect(*c).unwrap_or(Rect::ZERO);
            }
            if !r.is_empty() {
                *out = Some(out.map_or(r, |o| o.union(r)));
            }
        }
        for item in items {
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
                Item::PushFilter(f) => {
                    filters.push((*f, t, clips.len(), None));
                    None
                }
                Item::PopFilter => {
                    if let Some((f, ft, n, Some(inner))) = filters.pop() {
                        // The group's drawing, grown in its own space, then clipped by the
                        // clips around the group (not those inside it).
                        let grown = ft.apply_rect(f.outset(inner));
                        let open = clips.get(..n).unwrap_or(&clips);
                        match filters.last_mut() {
                            Some((_, pt, _, acc)) => {
                                let r = pt.inverse().map_or(grown, |i| i.apply_rect(grown));
                                *acc = Some(acc.map_or(r, |a| a.union(r)));
                            }
                            None => add(&mut out, grown, open),
                        }
                    }
                    None
                }
                Item::PushLayer { .. } | Item::PopLayer => None,
                drawn => drawn.bounds(),
            };
            let Some(r) = r else { continue };
            let r = t.apply_rect(r);
            match filters.last_mut() {
                // Inside a filter group: collected (clipped by the clips inside the group)
                // in the group's space, and grown when the group closes.
                Some((_, ft, n, acc)) => {
                    let mut r = r;
                    for c in clips.iter().skip(*n) {
                        r = r.intersect(*c).unwrap_or(Rect::ZERO);
                    }
                    if !r.is_empty() {
                        let r = ft.inverse().map_or(r, |i| i.apply_rect(r));
                        *acc = Some(acc.map_or(r, |a| a.union(r)));
                    }
                }
                None => add(&mut out, r, &clips),
            }
        }
        // A group left open still draws.
        while let Some((f, ft, n, inner)) = filters.pop() {
            if let Some(inner) = inner {
                let grown = ft.apply_rect(f.outset(inner));
                match filters.last_mut() {
                    Some((_, pt, _, acc)) => {
                        let r = pt.inverse().map_or(grown, |i| i.apply_rect(grown));
                        *acc = Some(acc.map_or(r, |a| a.union(r)));
                    }
                    None => add(&mut out, grown, clips.get(..n).unwrap_or(&clips)),
                }
            }
        }
        out
    }

    /// True when the list holds a filter group or a backdrop: renderers then draw a tile with
    /// room around it, so blurs cross tile edges seamlessly.
    pub fn has_filters(&self) -> bool {
        self.items.iter().any(|i| matches!(i, Item::PushFilter(_) | Item::Backdrop { .. }))
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
