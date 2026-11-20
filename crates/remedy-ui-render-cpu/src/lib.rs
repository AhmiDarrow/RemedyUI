//! RemedyUI's CPU renderer.
//!
//! Draws a [`DisplayList`] to premultiplied RGBA pixels with `vello_cpu` (multithreaded-ready,
//! SIMD) for shapes, paths, gradients, clips and layers, and RemedyUI's own hinted glyph
//! coverage for text: each glyph is rasterized once at its device size and subpixel bin,
//! cached as an alpha image, and drawn tinted with the run's color, in order with everything
//! else so clips and layers apply. It is the floor every machine can run, and the reference
//! renderer for golden-image tests. See `docs/design/04-rendering.md`.
//!
//! Corner radii in display lists are physical (`top_start` is top-left): widgets resolve
//! right-to-left before they paint.

mod composite;
mod surface;

pub use composite::CpuCompositor;
pub use surface::{ImageKey, Surface};

use std::collections::HashMap;
use std::sync::Arc;

use remedy_ui_core::{Color, CornerRadii, Point, Rect};
use remedy_ui_paint::{
    Affine, Blend, Brush, DisplayList, ExternalTexture, GlyphRun, Image, Item, Path, PathEl, Sampling,
    Stroke, StrokeCap, StrokeJoin, VideoFrame,
};
use remedy_ui_text::{Antialias, GlyphRasterizer, Hinting, size_64, subpixel_bin};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, BezPath, Shape};
use vello_cpu::peniko::{self, ColorStop, Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext, Resources};

/// A rendered frame: premultiplied RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// Premultiplied RGBA, row by row.
    pub pixels: Vec<u8>,
}

impl Frame {
    /// The pixel at (x, y), or `None` outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let p = self.pixels.get(i..i + 4)?;
        Some([p.first().copied()?, p.get(1).copied()?, p.get(2).copied()?, p.get(3).copied()?])
    }
}

/// Why a frame could not be rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    /// The size is zero or larger than the CPU renderer's 65,535-pixel axis.
    BadSize,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the frame size is zero or beyond 65,535 pixels on an axis")
    }
}

impl std::error::Error for RenderError {}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u64,
    face: u32,
    glyph: u32,
    size_64: u32,
    bin: u8,
    hinting: Hinting,
    /// `Some(bgr)` for LCD coverage.
    lcd: Option<bool>,
}

struct CachedGlyph {
    id: u64,
    /// For LCD glyphs, the additive color pass (see `glyph_run`).
    color_pass: Option<(u64, Arc<Pixmap>)>,
    image: Arc<Pixmap>,
    left: i32,
    top: i32,
}

/// Draws display lists onto any [`Surface`], caching glyph coverage and image pixmaps between
/// frames. Both renderers use it, so they draw the same pixels.
pub struct Painter {
    raster: GlyphRasterizer,
    glyphs: HashMap<GlyphKey, Option<CachedGlyph>>,
    /// App images by id: the converted pixmap, and the app's pixels it came from. When
    /// nothing holds those pixels any more the image is gone and its cache entry goes too.
    images: HashMap<u64, (std::sync::Weak<[u8]>, Arc<Pixmap>)>,
    /// Images dropped since the last [`Painter::take_evicted`], for caches downstream.
    evicted: Vec<u64>,
    /// Video streams drawn: the latest frame's id and bytes (gone when nothing holds them),
    /// and its CPU conversion once made.
    videos: HashMap<u64, VideoEntry>,
    /// Streams whose frames are all gone since the last [`Painter::take_evicted_videos`].
    evicted_videos: Vec<u64>,
    next_glyph: u64,
}

impl std::fmt::Debug for Painter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Painter({} glyphs, {} images cached)", self.glyphs.len(), self.images.len())
    }
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

/// The CPU renderer. Keep one per window: its painter caches glyphs and images between frames.
#[derive(Debug, Default)]
pub struct CpuRenderer {
    painter: Painter,
    /// A render context kept between rasters: making one allocates its strip and tile
    /// buffers, which costs more than drawing a small tile.
    ctx: Reused,
}

/// A [`RenderContext`] kept for reuse.
#[derive(Default)]
pub(crate) struct Reused(Option<RenderContext>);

impl std::fmt::Debug for Reused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Reused(context)" } else { "Reused(none)" })
    }
}

impl Reused {
    /// A context of `w` × `h`, reset, reusing the last one's memory.
    pub(crate) fn take(&mut self, w: u16, h: u16) -> RenderContext {
        match self.0.take() {
            Some(mut c) => {
                c.reset_and_resize(w, h);
                c
            }
            None => RenderContext::new(w, h),
        }
    }

    /// Keeps `ctx` for the next [`take`](Self::take).
    pub(crate) fn keep(&mut self, ctx: RenderContext) {
        self.0 = Some(ctx);
    }
}

fn color(c: Color) -> AlphaColor<Srgb> {
    AlphaColor::new([c.r, c.g, c.b, c.a])
}

fn kaffine(a: Affine) -> kurbo::Affine {
    let [x0, x1, x2, x3, x4, x5] = a.0;
    kurbo::Affine::new([
        f64::from(x0),
        f64::from(x1),
        f64::from(x2),
        f64::from(x3),
        f64::from(x4),
        f64::from(x5),
    ])
}

fn krect(r: Rect) -> kurbo::Rect {
    kurbo::Rect::new(f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y))
}

fn kpoint(p: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(p.x), f64::from(p.y))
}

/// A rounded rectangle as a path, corners fitted to the size (`top_start` is top-left).
pub fn rounded_path(rect: Rect, radii: CornerRadii) -> BezPath {
    rounded(rect, radii)
}

fn rounded(rect: Rect, radii: CornerRadii) -> BezPath {
    let r = radii.fit(rect.size());
    if r == CornerRadii::ZERO {
        return krect(rect).to_path(0.1);
    }
    let rr = kurbo::RoundedRectRadii::new(
        f64::from(r.top_start),
        f64::from(r.top_end),
        f64::from(r.bottom_end),
        f64::from(r.bottom_start),
    );
    kurbo::RoundedRect::from_rect(krect(rect), rr).to_path(0.1)
}

fn bezpath(path: &Path) -> BezPath {
    let mut b = BezPath::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(kpoint(p)),
            PathEl::LineTo(p) => b.line_to(kpoint(p)),
            PathEl::QuadTo(c, p) => b.quad_to(kpoint(c), kpoint(p)),
            PathEl::CubicTo(c1, c2, p) => b.curve_to(kpoint(c1), kpoint(c2), kpoint(p)),
            PathEl::Close => b.close_path(),
        }
    }
    b
}

fn stops(s: &[remedy_ui_paint::Stop]) -> Vec<ColorStop> {
    s.iter().map(|st| ColorStop::from((st.offset, color(st.color)))).collect()
}

fn set_brush<S: Surface>(ctx: &mut S, brush: &Brush) {
    match brush {
        Brush::Solid(c) => ctx.set_paint(color(*c)),
        Brush::Linear { start, end, stops: s } => ctx.set_paint(peniko::Gradient {
            kind: peniko::LinearGradientPosition { start: kpoint(*start), end: kpoint(*end) }.into(),
            stops: stops(s).as_slice().into(),
            extend: Extend::Pad,
            ..Default::default()
        }),
        Brush::Radial { center, radius, stops: s } => ctx.set_paint(peniko::Gradient {
            kind: peniko::RadialGradientPosition {
                start_center: kpoint(*center),
                start_radius: 0.0,
                end_center: kpoint(*center),
                end_radius: *radius,
            }
            .into(),
            stops: stops(s).as_slice().into(),
            extend: Extend::Pad,
            ..Default::default()
        }),
    }
}

fn kstroke(s: &Stroke) -> kurbo::Stroke {
    let mut k = kurbo::Stroke::new(f64::from(s.width))
        .with_caps(match s.cap {
            StrokeCap::Butt => kurbo::Cap::Butt,
            StrokeCap::Round => kurbo::Cap::Round,
            StrokeCap::Square => kurbo::Cap::Square,
        })
        .with_join(match s.join {
            StrokeJoin::Miter => kurbo::Join::Miter,
            StrokeJoin::Round => kurbo::Join::Round,
            StrokeJoin::Bevel => kurbo::Join::Bevel,
        });
    if !s.dashes.is_empty() {
        k = k.with_dashes(0.0, s.dashes.iter().map(|d| f64::from(*d)));
    }
    k
}

/// A video stream the painter has drawn.
struct VideoEntry {
    /// Its latest frame.
    id: u64,
    /// That frame's bytes; when nothing holds them the stream is gone.
    data: std::sync::Weak<[u8]>,
    /// The frame converted on the CPU, when the surface did not convert it.
    pixmap: Option<Arc<Pixmap>>,
}

/// Draws a GPU texture where the surface holds one (the GPU renderer); a surface without
/// textures (the CPU renderer) draws nothing for it.
fn texture_item<S: Surface>(ctx: &mut S, texture: &ExternalTexture, rect: Rect, radii: CornerRadii) {
    let Some(source) = ctx.texture_source(texture) else { return };
    ctx.set_paint(vello_cpu::Image {
        image: source,
        sampler: ImageSampler { quality: ImageQuality::Medium, ..Default::default() },
    });
    let sx = f64::from(rect.width()) / f64::from(texture.width.max(1));
    let sy = f64::from(rect.height()) / f64::from(texture.height.max(1));
    ctx.set_paint_transform(
        kurbo::Affine::translate((f64::from(rect.min.x), f64::from(rect.min.y)))
            * kurbo::Affine::scale_non_uniform(sx, sy),
    );
    ctx.fill_path(&rounded(rect, radii));
}

/// The vello filter for a display-list filter, or `None` when it changes nothing.
pub(crate) fn vello_filter(
    f: &remedy_ui_paint::Filter,
    scale: f32,
) -> Option<vello_cpu::filter_effects::Filter> {
    use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
    // Bounded: a filter's buffers grow with its blur and offset.
    let f = &f.limited(scale);
    let sigma = f.std_dev();
    let primitive = match f {
        remedy_ui_paint::Filter::Blur { .. } if sigma == 0.0 => return None,
        remedy_ui_paint::Filter::Blur { .. } => FilterPrimitive::GaussianBlur {
            std_deviation_x: sigma,
            std_deviation_y: sigma,
            edge_mode: EdgeMode::None,
        },
        remedy_ui_paint::Filter::DropShadow { offset, color: c, .. } => FilterPrimitive::DropShadow {
            dx: if offset.x.is_finite() { offset.x } else { 0.0 },
            dy: if offset.y.is_finite() { offset.y } else { 0.0 },
            std_deviation: sigma,
            color: color(*c),
            edge_mode: EdgeMode::None,
        },
    };
    Some(Filter::from_primitive(primitive))
}

/// The filter a backdrop of standard deviation `blur` draws its glass with, or `None` for no
/// blur. Both compositors use it, so they blur alike.
/// `scale` is how much the glass's transform scales lengths (the blur is bounded by
/// [`remedy_ui_paint::MAX_FILTER_STD_DEV`]).
pub fn backdrop_filter(blur: f32, scale: f32) -> Option<vello_cpu::filter_effects::Filter> {
    vello_filter(&remedy_ui_paint::Filter::Blur { radius: blur }, scale)
}

/// The vello filter a display-list filter is drawn with, or `None` when it changes nothing.
/// Both compositors use it for layers' filters, so they filter alike.
/// `scale` is how much the layer's transform scales lengths.
pub fn layer_filter(f: &remedy_ui_paint::Filter, scale: f32) -> Option<vello_cpu::filter_effects::Filter> {
    vello_filter(f, scale)
}

/// How far, in device pixels, filter `f` pushed under transform `t` carries drawing.
fn filter_reach(f: &remedy_ui_paint::Filter, t: Affine) -> f32 {
    let k = t.scale_factor();
    let f = &f.limited(k);
    let off = match f {
        remedy_ui_paint::Filter::DropShadow { offset, .. } => offset.x.abs().max(offset.y.abs()),
        remedy_ui_paint::Filter::Blur { .. } => 0.0,
    };
    let r = ((f.std_dev() * 3.0 + if off.is_finite() { off } else { 0.0 }) * k).ceil() + 1.0;
    if r.is_finite() { r } else { 0.0 }
}

fn grow(r: Rect, by: f32) -> Rect {
    Rect::new(r.min.x - by, r.min.y - by, r.max.x + by, r.max.y + by)
}

/// Premultiplied RGBA8 pixels as a pixmap, or `None` if the sizes do not fit.
fn pixmap_from(width: u32, height: u32, rgba: Vec<u8>) -> Option<Pixmap> {
    let (w, h) = (u16::try_from(width).ok()?, u16::try_from(height).ok()?);
    if rgba.len() != width as usize * height as usize * 4 || w == 0 || h == 0 {
        return None;
    }
    let mut p = Pixmap::new(w, h);
    p.data_as_u8_slice_mut().copy_from_slice(&rgba);
    Some(p)
}

fn size16(width: u32, height: u32) -> Result<(u16, u16), RenderError> {
    let w = u16::try_from(width).map_err(|_| RenderError::BadSize)?;
    let h = u16::try_from(height).map_err(|_| RenderError::BadSize)?;
    if w == 0 || h == 0 {
        return Err(RenderError::BadSize);
    }
    Ok((w, h))
}

impl CpuRenderer {
    /// A renderer with empty caches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops cached glyphs and images (for memory pressure).
    pub fn clear_caches(&mut self) {
        self.painter.clear_caches();
        let _ = self.painter.take_evicted();
    }

    /// How many app images are cached (images the app dropped are freed on the next draw).
    pub fn cached_images(&self) -> usize {
        self.painter.cached_images()
    }

    /// Renders `list` at `scale` device pixels per logical pixel onto `background`.
    pub fn render(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        scale: f32,
        background: Color,
    ) -> Result<Frame, RenderError> {
        self.render_with(list, width, height, Affine::scale(scale, scale), background)
    }

    /// Renders `list` through `base` (list space to device pixels) onto `background`, which
    /// may be transparent (for tiles and snapshots).
    pub fn render_with(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        base: Affine,
        background: Color,
    ) -> Result<Frame, RenderError> {
        let target = self.render_pixmap(list, width, height, base, background)?;
        Ok(Frame { width, height, pixels: target.data_as_u8_slice().to_vec() })
    }

    fn render_pixmap(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        base: Affine,
        background: Color,
    ) -> Result<Pixmap, RenderError> {
        let (w, h) = size16(width, height)?;
        let mut ctx = self.ctx.take(w, h);
        ctx.set_paint(color(background));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        let area = Rect::new(0.0, 0.0, f32::from(w), f32::from(h));
        self.painter.draw_within(&mut ctx, list, base, Some(area));
        ctx.flush();
        let mut target = Pixmap::new(w, h);
        let mut resources = Resources::new();
        ctx.render(&mut target, &mut resources);
        self.ctx.keep(ctx);
        Ok(target)
    }
}

impl Painter {
    /// A painter with empty caches.
    pub fn new() -> Self {
        Painter {
            raster: GlyphRasterizer::new(),
            glyphs: HashMap::new(),
            images: HashMap::new(),
            evicted: Vec::new(),
            videos: HashMap::new(),
            evicted_videos: Vec::new(),
            next_glyph: 0,
        }
    }

    /// Drops cached images the app no longer holds. Runs at the start of every draw.
    fn sweep_images(&mut self) {
        let before = self.images.len();
        let evicted = &mut self.evicted;
        self.images.retain(|id, (source, _)| {
            let live = source.strong_count() > 0;
            if !live {
                evicted.push(*id);
            }
            live
        });
        debug_assert!(self.images.len() + self.evicted.len() >= before);
        let gone = &mut self.evicted_videos;
        self.videos.retain(|stream, v| {
            let live = v.data.strong_count() > 0;
            if !live {
                gone.push(*stream);
            }
            live
        });
    }

    /// Video streams whose frames are all gone since the last call, so a renderer can free
    /// what it keeps for them.
    pub fn take_evicted_videos(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.evicted_videos)
    }

    /// The ids of images dropped from the cache since the last call, so a renderer can free
    /// what it uploaded for them.
    pub fn take_evicted(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.evicted)
    }

    /// How many app images are cached.
    pub fn cached_images(&self) -> usize {
        self.images.len()
    }

    /// Drops cached glyphs and images (for memory pressure).
    pub fn clear_caches(&mut self) {
        self.glyphs.clear();
        self.evicted.extend(self.images.keys().copied());
        self.images.clear();
        self.raster.clear();
    }

    /// Draws `list` onto `s` through `base` (list space to device pixels).
    pub fn draw<S: Surface>(&mut self, s: &mut S, list: &DisplayList, base: Affine) {
        self.draw_within(s, list, base, None);
    }

    /// Draws `list` onto `s` through `base`, skipping items that fall wholly outside `area`
    /// (device pixels; the target's size). A compositor tile draws only what crosses it, and
    /// what a filter can carry into it from around it.
    pub fn draw_within<S: Surface>(
        &mut self,
        s: &mut S,
        list: &DisplayList,
        base: Affine,
        area: Option<Rect>,
    ) {
        self.sweep_images();
        // Groups the list leaves open are closed: a list is drawn as a whole, balanced.
        self.draw_items(s, list.items(), base, area, 0, true);
        s.set_transform(kurbo::Affine::IDENTITY);
        s.reset_paint_transform();
    }

    /// Draws `items` through `base`. `outer_groups` is how many opacity, blend or filter
    /// groups are open around them (LCD text inside one falls back to grayscale). With
    /// `close`, groups the items leave open are closed at the end (a backdrop's replay).
    fn draw_items<S: Surface>(
        &mut self,
        s: &mut S,
        items: &[Item],
        base: Affine,
        area: Option<Rect>,
        outer_groups: usize,
        close: bool,
    ) {
        let scale = base.scale_factor();
        let mut transforms = vec![base];
        // Open opacity/blend/filter groups: LCD text inside one falls back to grayscale.
        let mut groups = outer_groups;
        // The culling area: inside a filter group, grown by what the filter carries in.
        let mut areas = vec![area];
        // Surface layers open (clips and groups), to close at the end of a replay.
        let mut open = 0usize;
        // Backdrop roots: where the innermost open group's content starts, and the transform
        // it starts under.
        let mut roots: Vec<(usize, Affine)> = vec![(0, base)];
        for (i, item) in items.iter().enumerate() {
            let t = transforms.last().copied().unwrap_or(base);
            let cull = areas.last().copied().flatten();
            // Culling is conservative: transformed bounds are boxes around the item.
            if let (Some(area), Some(b)) = (cull, item.bounds())
                && t.apply_rect(b).intersect(area).is_none_or(|r| r.is_empty())
            {
                continue;
            }
            s.set_transform(kaffine(t));
            s.reset_paint_transform();
            match item {
                Item::FillRect { rect, radii, brush } => {
                    set_brush(s, brush);
                    s.fill_path(&rounded(*rect, *radii));
                }
                Item::Border { rect, radii, width, color: c } => {
                    let inner = rect.deflate(remedy_ui_core::Insets::all(*width));
                    let r = radii.fit(rect.size());
                    let inner_r = CornerRadii {
                        top_start: (r.top_start - width).max(0.0),
                        top_end: (r.top_end - width).max(0.0),
                        bottom_end: (r.bottom_end - width).max(0.0),
                        bottom_start: (r.bottom_start - width).max(0.0),
                    };
                    let mut p = rounded(*rect, *radii);
                    p.extend(rounded(inner, inner_r));
                    s.set_fill_rule(peniko::Fill::EvenOdd);
                    s.set_paint(color(*c));
                    s.fill_path(&p);
                    s.set_fill_rule(peniko::Fill::NonZero);
                }
                Item::Shadow { rect, radii, offset, blur, spread, color: c } => {
                    let r = rect.translate(*offset);
                    let grown = remedy_ui_core::Rect::new(
                        r.min.x - spread,
                        r.min.y - spread,
                        r.max.x + spread,
                        r.max.y + spread,
                    );
                    let radius = radii.fit(rect.size());
                    let radius =
                        radius.top_start.max(radius.top_end).max(radius.bottom_end).max(radius.bottom_start);
                    s.set_paint(color(*c));
                    s.fill_blurred_rounded_rect(
                        &krect(grown),
                        radius + spread.max(0.0),
                        (blur / 2.0).max(0.0),
                    );
                }
                Item::FillPath { path, brush, even_odd } => {
                    set_brush(s, brush);
                    s.set_fill_rule(if *even_odd { peniko::Fill::EvenOdd } else { peniko::Fill::NonZero });
                    s.fill_path(&bezpath(path));
                    s.set_fill_rule(peniko::Fill::NonZero);
                }
                Item::StrokePath { path, stroke, brush } => {
                    set_brush(s, brush);
                    s.set_stroke(kstroke(stroke));
                    s.stroke_path(&bezpath(path));
                }
                Item::Glyphs(run) => self.glyph_run(s, run, t, scale, groups == 0),
                Item::Image { image, rect, radii, sampling } => {
                    self.image(s, image, *rect, *radii, *sampling)
                }
                Item::Video { frame, rect, radii } => self.video(s, frame, *rect, *radii),
                Item::Texture { texture, rect, radii } => texture_item(s, texture, *rect, *radii),
                Item::PushClip { rect, radii } => {
                    s.push_clip_layer(&rounded(*rect, *radii));
                    open += 1;
                }
                // A pop with nothing open (a list with one pop too many) is skipped: the
                // surface's own layers are not the list's to close.
                Item::PopClip if open == 0 => {}
                Item::PopLayer | Item::PopFilter if open == 0 => {}
                Item::PopClip => {
                    s.pop_layer();
                    open -= 1;
                }
                Item::PopLayer | Item::PopFilter => {
                    groups = groups.saturating_sub(1);
                    s.pop_layer();
                    open -= 1;
                    if roots.len() > 1 {
                        roots.pop();
                    }
                    if matches!(item, Item::PopFilter) && areas.len() > 1 {
                        areas.pop();
                    }
                }
                Item::PushLayer { opacity, blend } => {
                    groups += 1;
                    let mode = match blend {
                        Blend::Normal => peniko::BlendMode::default(),
                        Blend::Multiply => peniko::BlendMode::from(peniko::Mix::Multiply),
                        Blend::Screen => peniko::BlendMode::from(peniko::Mix::Screen),
                        Blend::Plus => peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Plus),
                    };
                    s.push_group(mode, *opacity);
                    open += 1;
                    roots.push((i + 1, t));
                }
                Item::PushFilter(f) => {
                    groups += 1;
                    match vello_filter(f, t.scale_factor()) {
                        Some(vf) => s.push_filter_layer(vf),
                        None => s.push_group(peniko::BlendMode::default(), 1.0),
                    }
                    open += 1;
                    roots.push((i + 1, t));
                    // What the group draws around the area can reach into it.
                    let reach = filter_reach(f, t);
                    areas.push(cull.map(|a| grow(a, reach)));
                }
                Item::Backdrop { rect, radii, blur } => {
                    let sigma = if blur.is_finite() { blur.max(0.0) } else { 0.0 };
                    let Some(&(start, root_t)) = roots.last() else { continue };
                    if sigma == 0.0 && start == i {
                        continue;
                    }
                    s.push_clip_layer(&rounded(*rect, *radii));
                    let f = remedy_ui_paint::Filter::Blur { radius: sigma };
                    match vello_filter(&f, t.scale_factor()) {
                        Some(vf) => s.push_filter_layer(vf),
                        None => s.push_group(peniko::BlendMode::default(), 1.0),
                    }
                    // What lies under the glass, drawn again from the backdrop root, blurred.
                    let reach = filter_reach(&f, t);
                    let under = cull.map(|a| grow(a, reach));
                    if let Some(prefix) = items.get(start..i) {
                        self.draw_items(s, prefix, root_t, under, groups + 1, true);
                    }
                    s.set_transform(kaffine(t));
                    s.pop_layer();
                    s.pop_layer();
                }
                Item::PushTransform(a) => transforms.push(a.then(t)),
                Item::PopTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                }
            }
        }
        if close {
            for _ in 0..open {
                s.pop_layer();
            }
        }
    }

    fn image<S: Surface>(
        &mut self,
        ctx: &mut S,
        image: &Image,
        rect: Rect,
        radii: CornerRadii,
        sampling: Sampling,
    ) {
        let pix = match self.images.get(&image.id) {
            Some((_, p)) => p.clone(),
            None => {
                let Some(p) = pixmap_from(image.width, image.height, image.pixels.to_vec()) else { return };
                let p = Arc::new(p);
                self.images.insert(image.id, (Arc::downgrade(&image.pixels), p.clone()));
                p
            }
        };
        let sx = f64::from(rect.width()) / f64::from(image.width.max(1));
        let sy = f64::from(rect.height()) / f64::from(image.height.max(1));
        let linear = sampling == Sampling::Linear;
        let Some(source) = ctx.image_source(ImageKey::Image(image.id), &pix, linear) else { return };
        ctx.set_paint(vello_cpu::Image {
            image: source,
            sampler: ImageSampler {
                quality: match sampling {
                    Sampling::Linear => ImageQuality::Medium,
                    Sampling::Nearest => ImageQuality::Low,
                },
                ..Default::default()
            },
        });
        ctx.set_paint_transform(
            kurbo::Affine::translate((f64::from(rect.min.x), f64::from(rect.min.y)))
                * kurbo::Affine::scale_non_uniform(sx, sy),
        );
        ctx.fill_path(&rounded(rect, radii));
    }

    /// Draws a video frame: converted by the surface when it can, else here (once per frame).
    fn video<S: Surface>(&mut self, ctx: &mut S, frame: &VideoFrame, rect: Rect, radii: CornerRadii) {
        if !frame.is_valid() {
            return;
        }
        let entry = self.videos.entry(frame.stream).or_insert_with(|| VideoEntry {
            id: frame.id,
            data: Arc::downgrade(&frame.data),
            pixmap: None,
        });
        if entry.id != frame.id {
            *entry = VideoEntry { id: frame.id, data: Arc::downgrade(&frame.data), pixmap: None };
        }
        let source = match ctx.video_source(frame) {
            Some(s) => s,
            None => {
                let pix = match &entry.pixmap {
                    Some(p) => p.clone(),
                    None => {
                        let Some(p) =
                            frame.to_rgba().and_then(|rgba| pixmap_from(frame.width, frame.height, rgba))
                        else {
                            return;
                        };
                        let p = Arc::new(p);
                        entry.pixmap = Some(p.clone());
                        p
                    }
                };
                let Some(s) = ctx.image_source(ImageKey::Video(frame.stream, frame.id), &pix, true) else {
                    return;
                };
                s
            }
        };
        ctx.set_paint(vello_cpu::Image {
            image: source,
            sampler: ImageSampler { quality: ImageQuality::Medium, ..Default::default() },
        });
        let sx = f64::from(rect.width()) / f64::from(frame.width.max(1));
        let sy = f64::from(rect.height()) / f64::from(frame.height.max(1));
        ctx.set_paint_transform(
            kurbo::Affine::translate((f64::from(rect.min.x), f64::from(rect.min.y)))
                * kurbo::Affine::scale_non_uniform(sx, sy),
        );
        ctx.fill_path(&rounded(rect, radii));
    }

    /// Draws a glyph run. Grayscale runs draw each glyph's coverage as an alpha mask tinted
    /// with the run's color. LCD runs (axis-aligned, outside opacity and blend groups) blend
    /// each color channel with its own subpixel coverage `c` (times the color's alpha `a`) in
    /// two passes, exact over opaque pixels: a multiply pass with premultiplied
    /// `(m - c, alpha m)` (`m` the largest channel coverage) leaves `dst * (1 - c·a)`, then an
    /// additive pass of `(c, alpha m)` tinted by the color adds `color * c·a`. Images are valid
    /// premultiplied data (GPU pipelines rely on it) and color only enters through tints, so
    /// cached images do not depend on color. Like every platform's subpixel text, LCD is for
    /// text over opaque content: over transparent pixels the result stays valid but its edges
    /// come out slightly too opaque.
    fn glyph_run<S: Surface>(&mut self, ctx: &mut S, run: &GlyphRun, t: Affine, scale: f32, flat: bool) {
        let [a, b, c, d, _, _] = t.0;
        let device_scale = ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0;
        let px_size = run.size * if device_scale > 0.0 { device_scale } else { scale };
        let axis_aligned = b == 0.0 && c == 0.0;
        let lcd = match run.antialias {
            Antialias::Lcd { bgr } if axis_aligned && flat => Some(bgr),
            _ => None,
        };
        ctx.set_transform(kurbo::Affine::IDENTITY);
        let [r, g, bl, al] = [run.color.r, run.color.g, run.color.b, run.color.a];
        // Tint colors are unpremultiplied: (1, 1, 1, a) scales a premultiplied image by a.
        let alpha_tint = vello_cpu::Tint {
            color: AlphaColor::new([1.0, 1.0, 1.0, al]),
            mode: vello_cpu::TintMode::Multiply,
        };
        let color_tint =
            vello_cpu::Tint { color: AlphaColor::new([r, g, bl, al]), mode: vello_cpu::TintMode::Multiply };
        let mask_tint = vello_cpu::Tint { color: color(run.color), mode: vello_cpu::TintMode::AlphaMask };
        for (id, origin) in &run.glyphs {
            let dev = t.apply(*origin);
            let hinting = if axis_aligned { run.hinting } else { Hinting::None };
            let bin = subpixel_bin(dev.x, hinting);
            let key = GlyphKey {
                font: run.font.id(),
                face: run.font.index(),
                glyph: *id,
                size_64: size_64(px_size),
                bin,
                hinting,
                lcd,
            };
            if !self.glyphs.contains_key(&key) {
                let aa = lcd.map_or(Antialias::Grayscale, |bgr| Antialias::Lcd { bgr });
                let img = self.raster.rasterize(&run.font, *id, px_size, bin, hinting, aa);
                let id = self.next_glyph;
                self.next_glyph += 2;
                let cached = img.and_then(|g| {
                    if lcd.is_some() {
                        let px = g.data.as_chunks::<3>().0;
                        let multiply: Vec<u8> = px
                            .iter()
                            .flat_map(|&[cr, cg, cb]| {
                                let m = cr.max(cg).max(cb);
                                [m - cr, m - cg, m - cb, m]
                            })
                            .collect();
                        let add: Vec<u8> =
                            px.iter().flat_map(|&[cr, cg, cb]| [cr, cg, cb, cr.max(cg).max(cb)]).collect();
                        let first = pixmap_from(g.width, g.height, multiply)?;
                        let second = pixmap_from(g.width, g.height, add)?;
                        Some(CachedGlyph {
                            id,
                            color_pass: Some((id + 1, Arc::new(second))),
                            image: Arc::new(first),
                            left: g.left,
                            top: g.top,
                        })
                    } else {
                        let rgba: Vec<u8> = g.data.iter().flat_map(|&v| [v, v, v, v]).collect();
                        pixmap_from(g.width, g.height, rgba).map(|p| CachedGlyph {
                            id,
                            color_pass: None,
                            image: Arc::new(p),
                            left: g.left,
                            top: g.top,
                        })
                    }
                });
                self.glyphs.insert(key, cached);
            }
            let Some(Some(gl)) = self.glyphs.get(&key) else { continue };
            let x = (dev.x.floor() as i32 + gl.left) as f64;
            let y = (dev.y.round() as i32 - gl.top) as f64;
            let rect =
                kurbo::Rect::new(x, y, x + f64::from(gl.image.width()), y + f64::from(gl.image.height()));
            let sampler = ImageSampler { quality: ImageQuality::Low, ..Default::default() };
            let draw = |ctx: &mut S, key: ImageKey, image: &Arc<Pixmap>, tint, blend| {
                let Some(source) = ctx.image_source(key, image, false) else { return };
                ctx.set_tint(Some(tint));
                ctx.set_blend_mode(blend);
                ctx.set_paint(vello_cpu::Image { image: source, sampler });
                ctx.set_paint_transform(kurbo::Affine::translate((x, y)));
                ctx.fill_rect(&rect);
            };
            match &gl.color_pass {
                Some((add_id, add)) => {
                    let multiply = peniko::BlendMode::new(peniko::Mix::Multiply, peniko::Compose::SrcOver);
                    let plus = peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Plus);
                    draw(ctx, ImageKey::Glyph(gl.id), &gl.image, alpha_tint, multiply);
                    draw(ctx, ImageKey::Glyph(*add_id), add, color_tint, plus);
                }
                None => draw(ctx, ImageKey::Glyph(gl.id), &gl.image, mask_tint, peniko::BlendMode::default()),
            }
        }
        ctx.set_blend_mode(peniko::BlendMode::default());
        ctx.set_tint(None);
        ctx.reset_paint_transform();
    }
}
