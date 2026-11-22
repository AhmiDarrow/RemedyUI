//! RemedyUI images: decoded off the UI thread at the size they are shown, color-managed,
//! upright, shared while in use and freed when nothing shows them.
//!
//! - [`image()`]: an element showing a file, bytes or an address, fitted to its box
//! - [`image_ext`]: the same, with the app's own placeholder and failure content, and the
//!   load's progress (the image's real size) reported to it
//! - [`image_with`]: the same element, with pixels from a supplier of your own (animations)
//! - [`load`] and [`probe`]: the loader underneath, for custom elements (avatars, previews)
//! - [`decode`]: the decoder on its own
//! - [`set_router`]: decoding somewhere else (a sandboxed helper process)
//! - [`set_fetcher`] and [`set_cache_dir`]: how images from addresses are fetched and kept
//! - [`set_memory_limit`], [`set_disk_limit`] and [`set_known_sizes_limit`]: how much is kept
//!
//! In a browser page images are drawn on the page's canvas, so an image from another site
//! shows only when its server allows it with CORS headers; otherwise the app fetches it
//! through its own server (a same-origin image proxy). The engine does not lay a DOM `<img>`
//! over the canvas for such images: it could not be clipped, rounded, layered under popups
//! and dialogs, faded, transformed or hit-tested with the rest of the UI, nor decoded at the
//! size shown and cached like every other image.

/// The image view's own words (what screen readers hear when an image cannot be shown) in
/// every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a language,
/// with [`remedy_ui_view::strings::add_messages`] under the name `media-image`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("media-image");

mod decode;
mod load;
mod remote;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_paint::{Brush, Image, Item, Path, Sampling, Stroke};
use remedy_ui_reactive::{Effect, Signal};
use remedy_ui_view::{El, Ui, show};

pub use decode::{
    DecodeError, Decoded, Format, Router, Srgb, add_format, decode, dimensions, finish_rgba, fitted,
    fitted_scalable, is_scalable, router, set_router,
};
pub use remote::{
    DEFAULT_DISK_LIMIT, DEFAULT_MEMORY_LIMIT, Fetch, FetchDone, FetchReply, FetchRequest, fetching,
    forget_remembered, remembered_bytes, set_cache_dir, set_disk_limit, set_fetcher, set_memory_limit,
    set_web_proxy,
};

pub use load::{
    DEFAULT_KNOWN_SIZES_LIMIT, ImageState, Request, Source, SourceId, Stamp, cached, decode_for_box,
    known_sizes, load, probe, set_known_sizes_limit, spawn_job,
};

/// How an image fills its element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
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

/// Where an image of `source` pixels goes in `pixel_box` (logical, its edges on the device
/// grid of `scale`), on whole device pixels as browsers place an `<img>` with `object-fit`:
/// sized by `fit` and centered, then snapped. Its size in device pixels is what the loader
/// decodes at ([`fitted`]), so a decoded image is drawn one pixel to one pixel.
fn place_px(fit: Fit, source: (f32, f32), at: (f32, f32), pixel_box: Rect, scale: f32) -> Rect {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let dev = |v: f32| (v * scale).round();
    let (bx, by) = (dev(pixel_box.min.x), dev(pixel_box.min.y));
    let (bw, bh) = (dev(pixel_box.max.x) - bx, dev(pixel_box.max.y) - by);
    let (iw, ih) = (source.0.max(1.0), source.1.max(1.0));
    let s = match fit {
        Fit::Fill => return pixel_box,
        Fit::Contain => (bw / iw).min(bh / ih),
        Fit::Cover => (bw / iw).max(bh / ih),
        Fit::ScaleDown => (bw / iw).min(bh / ih).min(1.0),
    };
    let (dw, dh) = ((iw * s).round().max(1.0), (ih * s).round().max(1.0));
    // At `object-position` (centred by default), then on the grid (half pixels round up, as
    // Firefox's NSToIntRound does).
    let (x, y) = ((bx + (bw - dw) * at.0 + 0.5).floor(), (by + (bh - dh) * at.1 + 0.5).floor());
    Rect::new(x / scale, y / scale, (x + dw) / scale, (y + dh) / scale)
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

/// What an image element shows besides its pixels, and who hears how its load goes
/// ([`image_ext`]).
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ImageOptions {
    /// Shown, filling the element, while the image loads, instead of the theme's placeholder.
    pub placeholder: Option<El>,
    /// Built when the image cannot be shown, filling the element (and sizing it when nothing
    /// else does), instead of the broken-image mark: a letter tile, a generic poster.
    pub fallback: Option<Rc<dyn Fn() -> El>>,
    /// Hears every step of the load: [`ImageState::Sized`] and [`ImageState::Ready`] carry the
    /// image's real size (to frame wide or square art), [`ImageState::Failed`] why it failed.
    pub on_state: Option<OnState>,
    /// Where the picture sits in its box when it does not fill it exactly (CSS
    /// `object-position`), as fractions of the room left on each axis: `(0.5, 0.5)` centres it
    /// (`None`, the default), `(0.5, 0.25)` is `center 25%`, `(0.0, 0.0)` the top left.
    pub position: Option<(f32, f32)>,
    /// Loads the picture only once its element comes within this many logical pixels of the
    /// window and of the scroll containers around it, as `loading="lazy"` does on the web
    /// (`None`, the default: at once).
    pub lazy: Option<f32>,
}

/// What hears an image element's load steps ([`ImageOptions::on_state`]).
pub type OnState = Rc<dyn Fn(&ImageState)>;

impl ImageOptions {
    /// No placeholder, fallback or listener: what [`image()`] does.
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows `el` while the image loads.
    pub fn placeholder(mut self, el: El) -> Self {
        self.placeholder = Some(el);
        self
    }

    /// Shows what `f` builds when the image cannot be shown.
    pub fn fallback(mut self, f: impl Fn() -> El + 'static) -> Self {
        self.fallback = Some(Rc::new(f));
        self
    }

    /// Calls `f` with each step of the load.
    pub fn on_state(mut self, f: impl Fn(&ImageState) + 'static) -> Self {
        self.on_state = Some(Rc::new(f));
        self
    }

    /// Places the picture at `x`, `y` (fractions of the room left, CSS `object-position`):
    /// `position(0.5, 0.25)` is `object-position: center 25%`.
    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.position = Some((x, y));
        self
    }

    /// Loads the picture only once it comes within `margin` logical pixels of being seen
    /// (`loading="lazy"`; browsers use a margin of a few hundred pixels).
    pub fn lazy(mut self, margin: f32) -> Self {
        self.lazy = Some(margin);
        self
    }
}

impl std::fmt::Debug for ImageOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageOptions")
            .field("placeholder", &self.placeholder.is_some())
            .field("fallback", &self.fallback.is_some())
            .field("on_state", &self.on_state.is_some())
            .field("position", &self.position)
            .field("lazy", &self.lazy)
            .finish()
    }
}

/// Like [`image()`], with the app's own placeholder and failure content and a listener for
/// the load ([`ImageOptions`]).
///
/// ```no_run
/// # use remedy_ui_media_image::{image_ext, Fit, ImageOptions, ImageState, Source};
/// # fn demo(ui: &remedy_ui_view::Ui) {
/// let ui2 = ui.clone();
/// let poster = image_ext(
///     ui,
///     Source::Url("https://example.com/poster.jpg".into()),
///     "Poster",
///     Fit::Cover,
///     ImageOptions::new()
///         .fallback(move || ui2.el("letter-tile").text("A"))
///         .on_state(|s| {
///             if let ImageState::Sized((w, h)) = s {
///                 let _wide = w > h;
///             }
///         }),
/// );
/// # let _ = poster;
/// # }
/// ```
#[track_caller]
pub fn image_ext(ui: &Ui, source: Source, alt: impl Into<String>, fit: Fit, options: ImageOptions) -> El {
    build(
        ui,
        source,
        alt.into(),
        fit,
        Rc::new(|source, box_px, cover, state| Box::new(load(source, box_px, cover, state)) as Box<dyn Any>),
        options,
    )
}

/// Makes `el` fill its parent where it has no size of its own.
fn fill(el: El) -> El {
    el.layout(|l| {
        if l.width == remedy_ui_layout::Length::Auto {
            l.width = remedy_ui_layout::Length::Percent(1.0);
        }
        if l.height == remedy_ui_layout::Length::Auto {
            l.height = remedy_ui_layout::Length::Percent(1.0);
        }
    })
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
    build(ui, source, alt.into(), fit, loader, ImageOptions::default())
}

#[track_caller]
fn build(ui: &Ui, source: Source, alt: String, fit: Fit, loader: Loader, options: ImageOptions) -> El {
    let ImageOptions { placeholder, fallback, on_state, position, lazy } = options;
    let (px_at, py_at) = position.unwrap_or((0.5, 0.5));
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
    if let Some(f) = on_state {
        let _ = Effect::new(move || state.with(|s| f(s)));
    }
    // An address's size comes with its bytes: the fetch goes with the element. A lazy image
    // fetches nothing until it is near.
    let probing = Rc::new(RefCell::new(if lazy.is_none() { load::probe_held(&source, state) } else { None }));
    let near = Rc::new(Cell::new(lazy.is_none()));
    let wanted: Rc<Cell<Option<(u32, u32)>>> = Rc::default();

    let request: Rc<RefCell<Option<Box<dyn Any>>>> = Rc::default();
    let asked: Rc<Cell<Option<(u32, u32)>>> = Rc::default();
    let own_placeholder = placeholder.is_some();
    let mut el = ui.el("image").variant(if own_placeholder { "" } else { "loading" });
    if let Some(p) = &placeholder {
        el = el.child(fill(p.clone()));
    }
    let has_fallback = fallback.is_some();
    if let Some(f) = fallback {
        let fb = fill(
            show(ui, move || failed.with(Option::is_some) && shown.with(Option::is_none), move || fill(f()))
                .el(),
        );
        el = el.child(fb);
    }
    let el = if fit == Fit::Cover { el.clip() } else { el };
    // Asks the loader for the box `wanted`, once the element is near enough to load.
    let ask: Rc<dyn Fn()> = {
        let (src, req, asked2, near, wanted) =
            (source.clone(), request.clone(), asked.clone(), near.clone(), wanted.clone());
        Rc::new(move || {
            let Some(px) = wanted.get().filter(|_| near.get()) else { return };
            if asked2.get() == Some(px) {
                return;
            }
            asked2.set(Some(px));
            let next = loader(&src, Some(px), fit == Fit::Cover, state);
            *req.borrow_mut() = Some(next);
        })
    };
    let (ui2, ask2, wanted2) = (ui.clone(), ask.clone(), wanted.clone());
    let el = el.on_resize(move |sz| {
        let scale = ui2.scale().max(0.5);
        // The box in whole device pixels, as it is drawn: the image is decoded at exactly the
        // size it shows, as a browser decodes an `<img>`, so it is drawn without resampling.
        let px = ((sz.width * scale).round() as u32, (sz.height * scale).round() as u32);
        if px.0 == 0 || px.1 == 0 {
            return;
        }
        wanted2.set(Some(px));
        ask2();
    });
    let el = match lazy {
        Some(margin) => {
            let (near2, src2, probing2) = (near.clone(), source, probing.clone());
            el.on_visibility_within(margin, move |seen| {
                if seen && !near2.replace(true) {
                    // Near at last: its size and pixels are fetched now, and it stays loaded.
                    *probing2.borrow_mut() = load::probe_held(&src2, state);
                    ask();
                }
            })
        }
        None => el,
    };
    // Removed from the window, the element lets its pixels go (its signals outlive it in
    // the scope that built it).
    let el = el.on_removed(move || {
        request.borrow_mut().take();
        probing.borrow_mut().take();
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
        let waiting = !ready && !failed.with(Option::is_some);
        let _ = e2.clone().variant(if waiting && !own_placeholder { "loading" } else { "" });
        if let Some(p) = &placeholder {
            let _ = p.clone().hidden(!waiting);
        }
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

    let (alt2, ui4) = (alt.clone(), ui.clone());
    let el = el.semantics(move || {
        if alt2.is_empty() {
            return Semantics::new(Role::Image).hidden(true);
        }
        let s = Semantics::new(Role::Image).label(alt2.clone());
        match failed.get() {
            // The reason is the decoder's or loader's own (English) words.
            Some(why) => s.description(STRINGS.format(&ui4, "image-not-shown", &[("reason", why.into())])),
            None => s,
        }
    });

    el.paint(move |cx| {
        let (w, h) = (cx.size.width, cx.size.height);
        if failed.with(Option::is_some) && shown.with(Option::is_none) {
            if has_fallback {
                return;
            }
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
        let rect = place_px(
            fit,
            (source.0 as f32, source.1 as f32),
            (px_at, py_at),
            cx.pixel_box,
            cx.scale.max(0.5),
        );
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

    fn place_px_c(fit: Fit, source: (f32, f32), b: Rect, scale: f32) -> Rect {
        place_px(fit, source, (0.5, 0.5), b, scale)
    }

    #[test]
    fn object_position_moves_the_picture_in_the_room_left() {
        // A 16:9 still covering a 2:3 box, at `center 25%` and at the top left.
        let b = Rect::new(0.0, 0.0, 200.0, 300.0);
        let r = place_px(Fit::Cover, (1920.0, 1080.0), (0.5, 0.25), b, 1.0);
        assert_eq!((r.width(), r.height()), (533.0, 300.0));
        assert_eq!(r.min.x, -166.0, "centred across: (200 - 533) / 2, the half pixel rounded up");
        let r = place_px(Fit::Contain, (1920.0, 1080.0), (0.5, 0.25), b, 1.0);
        assert_eq!((r.height(), r.min.y), (113.0, 47.0), "a quarter of the 187 px left above it");
        let r = place_px(Fit::Contain, (1920.0, 1080.0), (0.0, 0.0), b, 1.0);
        assert_eq!((r.min.x, r.min.y), (0.0, 0.0));
    }

    #[test]
    fn images_land_on_whole_device_pixels_at_the_size_they_are_decoded() {
        // A 342×513 poster covering a box snapped to 25..219 (a 194.25 px box at x = 24.5):
        // the picture is the size `fitted` decodes it at, on the grid.
        let b = Rect::new(25.0, 0.0, 219.0, 291.0);
        let r = place_px_c(Fit::Cover, (342.0, 513.0), b, 1.0);
        assert_eq!(fitted((342, 513), (194, 291), true), (194, 291));
        assert_eq!((r.min.x, r.min.y, r.width(), r.height()), (25.0, 0.0, 194.0, 291.0));
        // A wide still covering it is cropped at its sides, centered on a whole pixel.
        let r = place_px_c(Fit::Cover, (500.0, 281.0), b, 1.0);
        assert_eq!((r.width(), r.height()), (518.0, 291.0));
        assert_eq!(r.min.x, -137.0, "(194 - 518) / 2 from the box's left edge");
        // At 2x the grid is half a logical pixel.
        let r = place_px_c(Fit::Contain, (100.0, 50.0), Rect::new(0.0, 0.0, 30.0, 30.0), 2.0);
        assert_eq!((r.width(), r.height(), r.min.y), (30.0, 15.0, 7.5));
        let r = place_px_c(Fit::ScaleDown, (20.0, 10.0), Rect::new(0.0, 0.0, 100.0, 100.0), 1.0);
        assert_eq!((r.width(), r.height()), (20.0, 10.0));
    }

    #[test]
    fn decode_sizes_fit_or_cover_and_never_grow() {
        assert_eq!(fitted((4000, 3000), (400, 400), false), (400, 300));
        assert_eq!(fitted((4000, 3000), (400, 400), true), (533, 400));
        assert_eq!(fitted((100, 50), (400, 400), false), (100, 50));
    }
}
