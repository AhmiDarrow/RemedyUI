//! RemedyUI images: decoded off the UI thread at the size they are shown, color-managed,
//! upright, shared while in use and freed when nothing shows them.
//!
//! - [`image()`]: an element showing a file or bytes, fitted to its box
//! - [`image_with`]: the same element, with pixels from a supplier of your own (animations)
//! - [`load`] and [`probe`]: the loader underneath, for custom elements (avatars, previews)
//! - [`decode`]: the decoder on its own
//! - [`set_router`]: decoding somewhere else (a sandboxed helper process)

mod decode;
mod load;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_paint::{Brush, Image, Item, Path, Sampling, Stroke};
use remedy_ui_reactive::{Effect, Signal};
use remedy_ui_view::{El, Ui};

pub use decode::{
    DecodeError, Decoded, Format, Router, Srgb, add_format, decode, dimensions, finish_rgba, fitted,
    fitted_scalable, is_scalable, router, set_router,
};
pub use load::{
    ImageState, Request, Source, SourceId, Stamp, cached, decode_for_box, load, probe, spawn_job,
};

/// How an image fills its element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// All of it, as large as fits, centered (letterboxed).
    #[default]
    Contain,
    /// The whole box, cropping what does not fit, centered (thumbnails, avatars, banners).
    Cover,
    /// Stretched to the box.
    Fill,
    /// Its own size, or smaller to fit.
    ScaleDown,
}

/// Where the image goes in a `w`×`h` box: the drawn rectangle.
fn place(fit: Fit, img: (f32, f32), natural: (f32, f32), w: f32, h: f32) -> Rect {
    let (iw, ih) = (img.0.max(1.0), img.1.max(1.0));
    let s = match fit {
        Fit::Fill => return Rect::new(0.0, 0.0, w, h),
        Fit::Contain => (w / iw).min(h / ih),
        Fit::Cover => (w / iw).max(h / ih),
        Fit::ScaleDown => (w / iw).min(h / ih).min(natural.0 / iw),
    };
    let (dw, dh) = (iw * s, ih * s);
    let (x, y) = ((w - dw) / 2.0, (h - dh) / 2.0);
    Rect::new(x, y, x + dw, y + dh)
}

/// An element showing `source`, decoded off the UI thread at the size it is shown (again,
/// larger, when the element grows). `alt` says what the image shows for screen readers;
/// an empty `alt` marks it decorative.
///
/// Until its size is known the element has none of its own; it then keeps the image's
/// aspect ratio and, when nothing else sizes it, takes the image's own size (never wider
/// than its container). While loading it shows the theme's placeholder; if the image cannot
/// be shown, a broken-image mark, with the reason given to screen readers.
#[track_caller]
pub fn image(ui: &Ui, source: Source, alt: impl Into<String>, fit: Fit) -> El {
    image_with(
        ui,
        source,
        alt,
        fit,
        Rc::new(|source, box_px, cover, state| Box::new(load(source, box_px, cover, state)) as Box<dyn Any>),
    )
}

/// What supplies an image element's pixels: given the source, the box in device pixels, whether
/// the image covers it, and the signal to report to (as [`load`] does), it starts the work and
/// returns what keeps it going. The element drops that when the box outgrows it, and when it
/// is removed.
pub type Loader = Rc<dyn Fn(&Source, Option<(u32, u32)>, bool, Signal<ImageState>) -> Box<dyn Any>>;

/// Like [`image()`], with pixels from `loader` instead of the still-image loader. The element
/// takes its size, placeholder, failure mark, fit and accessibility from the image the loader
/// reports, and draws each `Ready` it reports as it arrives: a loader that reports a new
/// frame each time is an animation.
#[track_caller]
pub fn image_with(ui: &Ui, source: Source, alt: impl Into<String>, fit: Fit, loader: Loader) -> El {
    let alt = alt.into();
    let state = Signal::new(ImageState::Loading);
    // What is drawn: the last decoded pixels stay up while a sharper version loads.
    let shown: Signal<Option<(Image, (u32, u32))>> = Signal::new(None);
    let failed: Signal<Option<String>> = Signal::new(None);
    let _ = Effect::new(move || match state.get() {
        ImageState::Ready(img, size) => {
            shown.set(Some((img, size)));
            failed.set(None);
        }
        ImageState::Failed(e) => failed.set(Some(e.to_string())),
        ImageState::Loading | ImageState::Sized(_) => {}
    });
    probe(&source, state);

    let request: Rc<RefCell<Option<Box<dyn Any>>>> = Rc::default();
    let asked: Rc<Cell<Option<(u32, u32)>>> = Rc::default();
    let el = ui.el("image").variant("loading");
    let el = if fit == Fit::Cover { el.clip() } else { el };
    let (ui2, src, req, asked2) = (ui.clone(), source, request.clone(), asked.clone());
    let el = el.on_resize(move |sz| {
        let scale = ui2.scale().max(0.5);
        let px = ((sz.width * scale).ceil() as u32, (sz.height * scale).ceil() as u32);
        if px.0 == 0 || px.1 == 0 {
            return;
        }
        // Ask again only when the box outgrows what was decoded.
        let bucket = (px.0.next_power_of_two(), px.1.next_power_of_two());
        if asked2.get().is_some_and(|(w, h)| bucket.0 <= w && bucket.1 <= h) {
            return;
        }
        asked2.set(Some(bucket));
        let next = loader(&src, Some(px), fit == Fit::Cover, state);
        *req.borrow_mut() = Some(next);
    });
    // Removed from the window, the element lets its pixels go (its signals outlive it in
    // the scope that built it).
    let el = el.on_removed(move || {
        request.borrow_mut().take();
        let _ = shown.try_set(None);
        let _ = state.try_set(ImageState::Loading);
    });

    // Size from the image once it is known.
    let (e2, ui3) = (el.clone(), ui.clone());
    let el = el.bind(move || {
        let size = match state.get() {
            ImageState::Sized(s) | ImageState::Ready(_, s) => Some(s),
            _ => None,
        };
        let ready = shown.with(Option::is_some);
        let _ = e2.clone().variant(if ready || failed.with(Option::is_some) { "" } else { "loading" });
        if let Some((w, h)) = size {
            let scale = ui3.scale().max(0.5);
            let _ = e2.clone().layout(move |l| {
                if l.aspect_ratio.is_none() {
                    l.aspect_ratio = Some(w.max(1) as f32 / h.max(1) as f32);
                }
                if l.width == remedy_ui_layout::Length::Auto && l.height == remedy_ui_layout::Length::Auto {
                    l.width = remedy_ui_layout::Length::Px(w as f32 / scale);
                    l.max_width = remedy_ui_layout::Length::Percent(1.0);
                }
            });
        }
    });

    let alt2 = alt.clone();
    let el = el.semantics_dyn(move || {
        if alt2.is_empty() {
            return Semantics::new(Role::Image).hidden(true);
        }
        let s = Semantics::new(Role::Image).label(alt2.clone());
        match failed.get() {
            Some(why) => s.description(format!("Not shown: {why}")),
            None => s,
        }
    });

    el.paint(move |cx| {
        let (w, h) = (cx.size.width, cx.size.height);
        if failed.with(Option::is_some) && shown.with(Option::is_none) {
            // A broken-image mark: a frame with a crossed corner.
            let s = (w.min(h) * 0.4).clamp(8.0, 40.0);
            let (x, y) = ((w - s) / 2.0, (h - s) / 2.0);
            let mut p = Path::new();
            p.move_to(Point::new(x, y))
                .line_to(Point::new(x + s, y))
                .line_to(Point::new(x + s, y + s))
                .line_to(Point::new(x, y + s))
                .close()
                .move_to(Point::new(x, y + s))
                .line_to(Point::new(x + s * 0.45, y + s * 0.5))
                .line_to(Point::new(x + s * 0.65, y + s * 0.7))
                .line_to(Point::new(x + s, y + s * 0.35));
            cx.list.push(Item::StrokePath {
                path: p,
                stroke: Stroke::new(1.5),
                brush: Brush::Solid(cx.style.foreground.with_alpha(0.5)),
            });
            return;
        }
        let Some((img, source)) = shown.get() else { return };
        let scale = cx.scale.max(0.5);
        let natural = (source.0 as f32 / scale, source.1 as f32 / scale);
        let rect = place(fit, (source.0 as f32, source.1 as f32), natural, w, h);
        cx.list.push(Item::Image {
            image: img,
            rect,
            radii: CornerRadii::all(cx.style.radius),
            sampling: Sampling::Linear,
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_place_the_image() {
        let r = place(Fit::Contain, (200.0, 100.0), (200.0, 100.0), 100.0, 100.0);
        assert_eq!((r.min.y, r.height()), (25.0, 50.0));
        let r = place(Fit::Cover, (200.0, 100.0), (200.0, 100.0), 100.0, 100.0);
        assert_eq!((r.min.x, r.width(), r.height()), (-50.0, 200.0, 100.0));
        let r = place(Fit::ScaleDown, (20.0, 10.0), (20.0, 10.0), 100.0, 100.0);
        assert_eq!((r.width(), r.height()), (20.0, 10.0));
        let r = place(Fit::Fill, (20.0, 10.0), (20.0, 10.0), 100.0, 60.0);
        assert_eq!((r.width(), r.height()), (100.0, 60.0));
    }

    #[test]
    fn decode_sizes_fit_or_cover_and_never_grow() {
        assert_eq!(fitted((4000, 3000), (400, 400), false), (400, 300));
        assert_eq!(fitted((4000, 3000), (400, 400), true), (533, 400));
        assert_eq!(fitted((100, 50), (400, 400), false), (100, 50));
    }
}
