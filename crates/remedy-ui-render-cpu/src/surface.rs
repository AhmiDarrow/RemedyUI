//! The vector surface a display list draws onto.

use std::sync::Arc;

use vello_cpu::filter_effects::Filter;
use vello_cpu::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_cpu::peniko::{BlendMode, Fill};
use vello_cpu::{ImageSource, PaintType, Pixmap, RenderContext, Tint};

/// What a pixmap handed to [`Surface::image_source`] is: the same key always means the same
/// pixels, so surfaces may keep uploads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImageKey {
    /// An app image, by its [`Image::id`](remedy_ui_paint::Image).
    Image(u64),
    /// A rasterized glyph, by the painter's id for it.
    Glyph(u64),
    /// A video frame converted on the CPU: its stream and the frame's id.
    Video(u64, u64),
}

/// A vello-style drawing surface. `vello_cpu`'s `RenderContext` is one; the GPU renderer's
/// `vello_gpu` scene is another, so both renderers share one [`Painter`](crate::Painter) and
/// draw the same pixels.
pub trait Surface {
    /// Sets the transform for following shapes.
    fn set_transform(&mut self, t: Affine);
    /// Sets the paint.
    fn set_paint(&mut self, paint: impl Into<PaintType>);
    /// Sets the paint's own transform (shape space to paint space).
    fn set_paint_transform(&mut self, t: Affine);
    /// Resets the paint transform.
    fn reset_paint_transform(&mut self);
    /// Sets the fill rule.
    fn set_fill_rule(&mut self, rule: Fill);
    /// Sets the stroke style.
    fn set_stroke(&mut self, stroke: Stroke);
    /// Sets or clears the tint applied to image paints.
    fn set_tint(&mut self, tint: Option<Tint>);
    /// Fills a path.
    fn fill_path(&mut self, path: &BezPath);
    /// Fills a rectangle.
    fn fill_rect(&mut self, rect: &Rect);
    /// Strokes a path.
    fn stroke_path(&mut self, path: &BezPath);
    /// Fills a Gaussian-blurred rounded rectangle with the current solid paint.
    fn fill_blurred_rounded_rect(&mut self, rect: &Rect, radius: f32, std_dev: f32);
    /// Sets how following shapes blend with what is below (never a destructive mode).
    fn set_blend_mode(&mut self, mode: BlendMode);
    /// Starts a layer clipped to `path`.
    fn push_clip_layer(&mut self, path: &BezPath);
    /// Starts a layer blended with `blend` at `opacity`.
    fn push_group(&mut self, blend: BlendMode, opacity: f32);
    /// Starts a layer drawn through `filter` (a Gaussian blur, a drop shadow) in the current
    /// transform's space, ended by [`pop_layer`](Self::pop_layer).
    fn push_filter_layer(&mut self, filter: Filter);
    /// Ends the latest layer.
    fn pop_layer(&mut self);
    /// A paint source for premultiplied `pixmap`, `filtered` when it will be sampled
    /// bilinearly. `None` when the surface cannot hold it (it is then skipped).
    fn image_source(&mut self, key: ImageKey, pixmap: &Arc<Pixmap>, filtered: bool) -> Option<ImageSource>;
    /// A paint source for a video frame when the surface converts frames itself (the GPU
    /// renderer, on the GPU), sized `frame.width` × `frame.height`. `None` lets the painter
    /// convert it on the CPU and hand it over as an image.
    fn video_source(&mut self, frame: &remedy_ui_paint::VideoFrame) -> Option<ImageSource> {
        let _ = frame;
        None
    }
    /// A paint source for a texture registered with the surface's GPU device, sized
    /// `texture.width` × `texture.height`. `None` when the surface has no such texture (the
    /// CPU renderer has none): the item is then skipped.
    fn texture_source(&mut self, texture: &remedy_ui_paint::ExternalTexture) -> Option<ImageSource> {
        let _ = texture;
        None
    }
}

impl Surface for RenderContext {
    fn set_transform(&mut self, t: Affine) {
        RenderContext::set_transform(self, t);
    }
    fn set_paint(&mut self, paint: impl Into<PaintType>) {
        RenderContext::set_paint(self, paint);
    }
    fn set_paint_transform(&mut self, t: Affine) {
        RenderContext::set_paint_transform(self, t);
    }
    fn reset_paint_transform(&mut self) {
        RenderContext::reset_paint_transform(self);
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        RenderContext::set_fill_rule(self, rule);
    }
    fn set_stroke(&mut self, stroke: Stroke) {
        RenderContext::set_stroke(self, stroke);
    }
    fn set_tint(&mut self, tint: Option<Tint>) {
        RenderContext::set_tint(self, tint);
    }
    fn fill_path(&mut self, path: &BezPath) {
        RenderContext::fill_path(self, path);
    }
    fn fill_rect(&mut self, rect: &Rect) {
        RenderContext::fill_rect(self, rect);
    }
    fn stroke_path(&mut self, path: &BezPath) {
        RenderContext::stroke_path(self, path);
    }
    fn fill_blurred_rounded_rect(&mut self, rect: &Rect, radius: f32, std_dev: f32) {
        RenderContext::fill_blurred_rounded_rect(self, rect, radius, std_dev);
    }
    fn set_blend_mode(&mut self, mode: BlendMode) {
        RenderContext::set_blend_mode(self, mode);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        RenderContext::push_clip_layer(self, path);
    }
    fn push_group(&mut self, blend: BlendMode, opacity: f32) {
        RenderContext::push_layer(self, None, Some(blend), Some(opacity), None, None);
    }
    fn push_filter_layer(&mut self, filter: Filter) {
        RenderContext::push_filter_layer(self, filter);
    }
    fn pop_layer(&mut self) {
        RenderContext::pop_layer(self);
    }
    fn image_source(&mut self, _key: ImageKey, pixmap: &Arc<Pixmap>, _filtered: bool) -> Option<ImageSource> {
        Some(ImageSource::Pixmap(pixmap.clone()))
    }
}
