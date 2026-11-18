//! Windows in the page: the first fills the page (or its mount element); later ones float
//! over it as panels.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU32, Ordering};

use remedy_ui_core::{Point, Size};
use remedy_ui_host::{
    DropEffect, HostEvent, Pixels, ResizeEdge, SafeArea, Unsupported, WindowId, WindowOptions,
};
use remedy_ui_input::KeyEvent;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{HtmlCanvasElement, HtmlElement};

use super::dom::{self, Listener};
use super::{Driver, aria, post, text};
use crate::convert;

/// Height of the title bar the bridge draws for decorated floating windows, in CSS pixels.
const TITLE_BAR: f64 = 32.0;

/// Ids for the `data-raw-handle` attribute wgpu finds GPU canvases by.
static NEXT_CANVAS: AtomicU32 = AtomicU32::new(1);

/// What a window is in the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Fills the page or its mount element; the page decides its size.
    Root,
    /// A panel over the page that the user (or the app) moves and sizes.
    Floating,
}

/// A move or resize the user is making by dragging.
#[derive(Clone, Copy, Debug)]
enum Drag {
    Move,
    Resize(ResizeEdge),
}

/// A drag under way: what it does, where the pointer pressed, the window's geometry then
/// (left, top, width, height), and the listeners following the pointer.
type DragState = (Drag, f64, f64, [f64; 4], Vec<Listener>);

/// One window.
pub(crate) struct Win {
    pub(crate) id: WindowId,
    pub(crate) kind: Kind,
    /// The outermost element the bridge made for the window.
    pub(crate) frame: HtmlElement,
    /// The content: canvases, the accessibility mirror and the hidden text field. It takes
    /// keyboard focus for the window.
    pub(crate) surface: HtmlElement,
    title_el: Option<HtmlElement>,
    cpu: HtmlCanvasElement,
    gpu: HtmlCanvasElement,
    ctx2d: RefCell<Option<web_sys::CanvasRenderingContext2d>>,
    pub(crate) canvas_id: u32,
    pub(crate) text: text::TextInput,
    pub(crate) aria: aria::Mirror,
    /// Device pixels and device pixels per CSS pixel.
    pub(crate) size: Cell<(u32, u32, f32)>,
    frame_requested: Cell<bool>,
    raf_cb: Closure<dyn FnMut(f64)>,
    observer: Option<web_sys::ResizeObserver>,
    _observer_cb: Closure<dyn FnMut(js_sys::Array)>,
    pub(crate) listeners: RefCell<Vec<Listener>>,
    drag: RefCell<Option<DragState>>,
    resolution: RefCell<Option<Listener>>,
    transparent: bool,
    pub(crate) visible: Cell<bool>,
    pub(crate) minimized: Cell<bool>,
    pub(crate) maximized: Cell<bool>,
    restore: Cell<Option<[f64; 4]>>,
    pub(crate) on_top: Cell<bool>,
    pub(crate) drop_effect: Cell<DropEffect>,
    /// Where the last pointer press was, in client CSS pixels (moves and resizes start there).
    pub(crate) last_down: Cell<Option<(f64, f64)>>,
    pub(crate) focused: Cell<bool>,
    /// A paste shortcut waiting for the browser's `paste` event, which brings the text.
    pub(crate) deferred_paste: RefCell<Option<KeyEvent>>,
    /// A copy or cut shortcut reached the app; its `copy`/`cut` event follows.
    pub(crate) clipboard_key: Cell<bool>,
    /// The app consumed the last pointer event it was given.
    pub(crate) consumed: Cell<bool>,
    /// The app consumed the last press: the browser's context menu that follows it (on the
    /// press, or on the release on Windows) stays away.
    pub(crate) press_taken: Cell<bool>,
    pub(crate) safe_area: Cell<SafeArea>,
    /// A touchpad pinch in progress, and the browser timer that ends it.
    pub(crate) pinch: Cell<Option<i32>>,
    /// Where the pinch is.
    pub(crate) pinch_at: Cell<Point>,
    /// Safari: the total scale its gesture events reported last.
    pub(crate) gesture_scale: Cell<f64>,
    pub(crate) pinch_end_cb: Closure<dyn FnMut()>,
}

impl Win {
    /// Makes a window: in `mount` as the page's root window, or floating over the page.
    pub(crate) fn create(
        d: &Rc<Driver>,
        id: WindowId,
        options: &WindowOptions,
        mount: Option<web_sys::Element>,
        z: i32,
    ) -> Result<Rc<Win>, Unsupported> {
        let fail = |what: &str| Unsupported::new("windows", format!("the page refused to make {what}"));
        let document =
            dom::document().ok_or_else(|| Unsupported::new("windows", "the page has no document"))?;
        let kind = if mount.is_some() { Kind::Root } else { Kind::Floating };
        let frame: HtmlElement = dom::element("div").ok_or_else(|| fail("an element"))?;
        let surface: HtmlElement = dom::element("div").ok_or_else(|| fail("an element"))?;
        let cpu: HtmlCanvasElement = dom::element("canvas").ok_or_else(|| fail("a canvas"))?;
        let gpu: HtmlCanvasElement = dom::element("canvas").ok_or_else(|| fail("a canvas"))?;
        let aria_root: HtmlElement = dom::element("div").ok_or_else(|| fail("an element"))?;
        let canvas_id = NEXT_CANVAS.fetch_add(1, Ordering::Relaxed);
        let _ = gpu.set_attribute("data-raw-handle", &canvas_id.to_string());
        let _ = frame.set_attribute("data-remedy-window", &id.0.to_string());
        let full =
            [("position", "absolute"), ("left", "0"), ("top", "0"), ("width", "100%"), ("height", "100%")];
        dom::style(&cpu, &full);
        dom::style(&gpu, &full);
        dom::style(&gpu, &[("display", "none")]);
        // The canvases are pictures of the app: screen readers read the mirror instead.
        let _ = cpu.set_attribute("aria-hidden", "true");
        let _ = gpu.set_attribute("aria-hidden", "true");
        dom::style(
            &surface,
            &[
                ("position", "relative"),
                ("overflow", "hidden"),
                ("touch-action", "none"),
                ("user-select", "none"),
                ("-webkit-user-select", "none"),
                ("-webkit-tap-highlight-color", "transparent"),
                ("outline", "none"),
                ("cursor", "default"),
            ],
        );
        surface.set_tab_index(0);
        let _ = surface.append_child(&cpu);
        let _ = surface.append_child(&gpu);
        let _ = surface.append_child(&aria_root);
        let text = text::TextInput::new(&surface).ok_or_else(|| fail("a text field"))?;
        let mut title_el = None;
        match &mount {
            Some(container) => {
                let _ = frame.append_child(&surface);
                dom::style(&surface, &[("width", "100%"), ("height", "100%")]);
                dom::style(
                    &frame,
                    &[
                        ("position", "relative"),
                        ("width", "100%"),
                        ("height", "100%"),
                        ("overflow", "hidden"),
                    ],
                );
                if container.tag_name().eq_ignore_ascii_case("body") {
                    // The page is the window: no margins, no page scrolling.
                    if let Some(html) =
                        document.document_element().and_then(|e| e.dyn_into::<HtmlElement>().ok())
                    {
                        dom::style(&html, &[("height", "100%"), ("overflow", "hidden")]);
                    }
                    if let Ok(body) = container.clone().dyn_into::<HtmlElement>() {
                        dom::style(&body, &[("margin", "0"), ("height", "100%"), ("overflow", "hidden")]);
                    }
                }
                let _ = container.append_child(&frame);
            }
            None => {
                let body =
                    document.body().ok_or_else(|| Unsupported::new("windows", "the page has no body"))?;
                let (w, h) = (f64::from(options.size.width), f64::from(options.size.height));
                let bar = if options.decorated { TITLE_BAR } else { 0.0 };
                let win = web_sys::window();
                let vw =
                    win.as_ref().and_then(|w| w.inner_width().ok()).and_then(|v| v.as_f64()).unwrap_or(w);
                let vh =
                    win.as_ref().and_then(|w| w.inner_height().ok()).and_then(|v| v.as_f64()).unwrap_or(h);
                let (x, y) =
                    options.position.map_or(((vw - w).max(0.0) / 2.0, (vh - h - bar).max(0.0) / 2.0), |p| {
                        (f64::from(p.x), f64::from(p.y))
                    });
                dom::style(
                    &frame,
                    &[
                        ("position", "fixed"),
                        ("left", &format!("{x}px")),
                        ("top", &format!("{y}px")),
                        ("width", &format!("{w}px")),
                        ("height", &format!("{}px", h + bar)),
                        ("display", "flex"),
                        ("flex-direction", "column"),
                        ("overflow", "hidden"),
                        ("box-sizing", "border-box"),
                        ("z-index", &z.to_string()),
                    ],
                );
                if !options.transparent {
                    dom::style(
                        &frame,
                        &[("box-shadow", "0 8px 32px rgba(0, 0, 0, 0.35)"), ("background", "Canvas")],
                    );
                }
                if options.resizable {
                    dom::style(&frame, &[("resize", "both")]);
                }
                if let Some(min) = options.min_size {
                    dom::style(
                        &frame,
                        &[
                            ("min-width", &format!("{}px", min.width)),
                            ("min-height", &format!("{}px", f64::from(min.height) + bar)),
                        ],
                    );
                }
                let _ = frame.set_attribute("role", "dialog");
                let _ = frame.set_attribute("aria-label", &options.title);
                if options.decorated {
                    title_el = Some(title_bar(&frame, &options.title).ok_or_else(|| fail("a title bar"))?);
                }
                dom::style(&surface, &[("flex", "1 1 auto"), ("min-height", "0")]);
                let _ = frame.append_child(&surface);
                let _ = body.append_child(&frame);
            }
        }
        let aria = aria::Mirror::new(aria_root, id);
        let win = Rc::new_cyclic(|me: &Weak<Win>| {
            let m = me.clone();
            let raf_cb = Closure::<dyn FnMut(f64)>::new(move |ts: f64| {
                if let Some(w) = m.upgrade() {
                    w.frame_requested.set(false);
                    let at = std::time::Duration::from_secs_f64(ts.max(0.0) / 1000.0);
                    post(HostEvent::Frame { window: w.id, at });
                }
            });
            let m = me.clone();
            let observer_cb = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
                if let Some(w) = m.upgrade() {
                    w.observed(&entries);
                }
            });
            let observer = web_sys::ResizeObserver::new(observer_cb.as_ref().unchecked_ref()).ok();
            let m = me.clone();
            let pinch_end_cb = Closure::<dyn FnMut()>::new(move || {
                if let Some(w) = m.upgrade() {
                    super::input::end_pinch(&w);
                }
            });
            Win {
                id,
                kind,
                frame,
                surface,
                title_el,
                cpu,
                gpu,
                ctx2d: RefCell::new(None),
                canvas_id,
                text,
                aria,
                size: Cell::new((0, 0, 1.0)),
                frame_requested: Cell::new(false),
                raf_cb,
                observer,
                _observer_cb: observer_cb,
                listeners: RefCell::new(Vec::new()),
                drag: RefCell::new(None),
                resolution: RefCell::new(None),
                transparent: options.transparent,
                visible: Cell::new(options.visible),
                minimized: Cell::new(false),
                maximized: Cell::new(false),
                restore: Cell::new(None),
                on_top: Cell::new(options.always_on_top),
                drop_effect: Cell::new(DropEffect::None),
                last_down: Cell::new(None),
                focused: Cell::new(false),
                deferred_paste: RefCell::new(None),
                clipboard_key: Cell::new(false),
                consumed: Cell::new(false),
                press_taken: Cell::new(false),
                safe_area: Cell::new(SafeArea::default()),
                pinch: Cell::new(None),
                pinch_at: Cell::new(Point::ZERO),
                gesture_scale: Cell::new(1.0),
                pinch_end_cb,
            }
        });
        if let Some(t) = &win.title_el {
            win.attach_title_bar(t);
        }
        super::input::attach(d, &win);
        win.set_visible(options.visible);
        win.observe();
        win.watch_resolution();
        Ok(win)
    }

    fn attach_title_bar(self: &Rc<Self>, bar: &HtmlElement) {
        let me = Rc::downgrade(self);
        let down = Listener::new(bar, "pointerdown", move |e| {
            let Some(w) = me.upgrade() else { return };
            let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() else { return };
            if p.button() != 0 {
                return;
            }
            // The close button handles its own press.
            let on_button = p
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .is_some_and(|t| t.tag_name() == "BUTTON");
            if on_button {
                return;
            }
            p.prevent_default();
            w.raise();
            w.last_down.set(Some((f64::from(p.client_x()), f64::from(p.client_y()))));
            w.begin(Drag::Move);
        });
        let me = Rc::downgrade(self);
        let close = bar.query_selector("button").ok().flatten().map(|b| {
            Listener::new(&b, "click", move |_| {
                if let Some(w) = me.upgrade() {
                    post(HostEvent::CloseRequested(w.id));
                }
            })
        });
        let mut l = self.listeners.borrow_mut();
        l.push(down);
        l.extend(close);
    }

    fn observe(&self) {
        let Some(o) = &self.observer else { return };
        // Device pixels where the browser reports them exactly; CSS pixels times the ratio
        // elsewhere.
        // A browser that does not know the device-pixel box throws for it; the call is made
        // through `Function.call` so that is an answer, not a trap.
        let opts = web_sys::ResizeObserverOptions::new();
        opts.set_box(web_sys::ResizeObserverBoxOptions::DevicePixelContentBox);
        let exact = dom::get(o, "observe")
            .dyn_into::<js_sys::Function>()
            .ok()
            .is_some_and(|f| f.call2(o, &self.surface, &opts).is_ok());
        if !exact {
            o.observe(&self.surface);
        }
    }

    /// Follows the device pixel ratio (page zoom, another screen), which can change without
    /// the element's size in device pixels changing.
    fn watch_resolution(self: &Rc<Self>) {
        let Some(win) = web_sys::window() else { return };
        let dpr = win.device_pixel_ratio();
        let Ok(Some(mql)) = win.match_media(&format!("(resolution: {dpr}dppx)")) else { return };
        let me = Rc::downgrade(self);
        let l = Listener::new(&mql, "change", move |_| {
            if let Some(w) = me.upgrade() {
                w.measure();
                w.watch_resolution();
            }
        });
        *self.resolution.borrow_mut() = Some(l);
    }

    fn observed(&self, entries: &js_sys::Array) {
        let Some(entry) = entries.get(0).dyn_into::<web_sys::ResizeObserverEntry>().ok() else { return };
        let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        let exact = entry
            .device_pixel_content_box_size()
            .get(0)
            .dyn_into::<web_sys::ResizeObserverSize>()
            .ok()
            .map(|s| (s.inline_size(), s.block_size()));
        let (w, h) = exact.unwrap_or_else(|| {
            let r = entry.content_rect();
            ((r.width() * dpr).round(), (r.height() * dpr).round())
        });
        self.apply_size(w, h, dpr);
    }

    /// Measures the window again from its layout (after the device pixel ratio changed).
    pub(crate) fn measure(&self) {
        let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        let r = self.surface.get_bounding_client_rect();
        self.apply_size((r.width() * dpr).round(), (r.height() * dpr).round(), dpr);
    }

    fn apply_size(&self, w: f64, h: f64, dpr: f64) {
        let (w, h) = (w.max(0.0) as u32, h.max(0.0) as u32);
        let scale = if dpr.is_finite() && dpr > 0.0 { dpr as f32 } else { 1.0 };
        let old = self.size.get();
        if old == (w, h, scale) {
            return;
        }
        self.size.set((w, h, scale));
        for c in [&self.cpu, &self.gpu] {
            if c.width() != w {
                c.set_width(w);
            }
            if c.height() != h {
                c.set_height(h);
            }
        }
        post(HostEvent::Resized { window: self.id, width: w, height: h, scale });
        self.update_safe_area();
    }

    /// The parts of the window that the screen's cutouts, rounded corners and the on-screen
    /// keyboard cover, and tells the app when they change.
    pub(crate) fn update_safe_area(&self) {
        let Some(win) = web_sys::window() else { return };
        let insets = super::settings::safe_insets(&win);
        let r = self.surface.get_bounding_client_rect();
        let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(r.right());
        let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(r.bottom());
        // Only the part of an inset over the window counts.
        let area = SafeArea {
            top: (f64::from(insets.top) - r.top()).max(0.0) as f32,
            left: (f64::from(insets.left) - r.left()).max(0.0) as f32,
            right: (f64::from(insets.right) - (vw - r.right())).max(0.0) as f32,
            bottom: (f64::from(insets.bottom) - (vh - r.bottom())).max(0.0) as f32,
        };
        if area != self.safe_area.get() {
            self.safe_area.set(area);
            post(HostEvent::SafeArea(self.id, area));
        }
    }

    pub(crate) fn request_frame(&self) {
        if self.frame_requested.replace(true) {
            return;
        }
        if let Some(w) = web_sys::window()
            && w.request_animation_frame(self.raf_cb.as_ref().unchecked_ref()).is_err()
        {
            self.frame_requested.set(false);
        }
    }

    /// Shows the canvas of the renderer drawing now.
    pub(crate) fn show_renderer(&self, gpu: bool) {
        dom::style(&self.gpu, &[("display", if gpu { "block" } else { "none" })]);
        dom::style(&self.cpu, &[("display", if gpu { "none" } else { "block" })]);
    }

    pub(crate) fn present(
        &self,
        pixels: Pixels<'_>,
        damage: &[remedy_ui_core::Rect],
    ) -> Result<(), Unsupported> {
        if pixels.data.len() != pixels.width as usize * pixels.height as usize * 4 {
            return Err(Unsupported::new("present", "the pixel data does not match its size"));
        }
        if self.cpu.width() != pixels.width || self.cpu.height() != pixels.height {
            // A frame drawn for another size (the window changed meanwhile): draw it whole.
            self.cpu.set_width(pixels.width);
            self.cpu.set_height(pixels.height);
            return self.put(pixels, &[]);
        }
        self.put(pixels, damage)
    }

    fn put(&self, pixels: Pixels<'_>, damage: &[remedy_ui_core::Rect]) -> Result<(), Unsupported> {
        let ctx = self.context()?;
        for rect in convert::damage_rects(damage, pixels.width, pixels.height) {
            let straight = convert::straight_rgba(pixels.data, pixels.width, rect);
            let (x, y, w, h) = rect;
            let image = web_sys::ImageData::new_with_u8_clamped_array_and_sh(Clamped(&straight), w, h)
                .map_err(|e| Unsupported::new("CPU presents", format!("{e:?}")))?;
            ctx.put_image_data(&image, f64::from(x), f64::from(y))
                .map_err(|e| Unsupported::new("CPU presents", format!("{e:?}")))?;
        }
        Ok(())
    }

    fn context(&self) -> Result<web_sys::CanvasRenderingContext2d, Unsupported> {
        if let Some(c) = self.ctx2d.borrow().as_ref() {
            return Ok(c.clone());
        }
        let opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&opts, &"alpha".into(), &JsValue::from_bool(self.transparent));
        let ctx = self
            .cpu
            .get_context_with_context_options("2d", &opts)
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
            .ok_or_else(|| Unsupported::new("CPU presents", "the canvas gave no 2D context"))?;
        *self.ctx2d.borrow_mut() = Some(ctx.clone());
        Ok(ctx)
    }

    pub(crate) fn set_visible(&self, on: bool) {
        self.visible.set(on);
        let display = match (on && !self.minimized.get(), self.kind) {
            (false, _) => "none",
            (true, Kind::Root) => "block",
            (true, Kind::Floating) => "flex",
        };
        dom::style(&self.frame, &[("display", display)]);
    }

    pub(crate) fn set_title(&self, title: &str, page_title: bool) {
        match self.kind {
            Kind::Root => {
                if page_title && let Some(doc) = dom::document() {
                    doc.set_title(title);
                }
                let _ = self.surface.set_attribute("aria-label", title);
            }
            Kind::Floating => {
                let _ = self.frame.set_attribute("aria-label", title);
                if let Some(t) = self.title_el.as_ref().and_then(|b| b.first_element_child()) {
                    t.set_text_content(Some(title));
                }
            }
        }
    }

    /// Puts a floating window above the others.
    pub(crate) fn raise(&self) {
        if self.kind != Kind::Floating {
            return;
        }
        let Some(d) = super::driver() else { return };
        let z = {
            let mut s = d.state.borrow_mut();
            s.next_z += 1;
            s.next_z
        };
        // Windows kept on top stay above every other window.
        let z = if self.on_top.get() { z.saturating_add(1_000_000) } else { z };
        dom::style(&self.frame, &[("z-index", &z.to_string())]);
    }

    /// The top-left of a floating window in the page, or of the root window's element.
    pub(crate) fn position(&self) -> Point {
        let r = self.frame.get_bounding_client_rect();
        Point::new(r.left() as f32, r.top() as f32)
    }

    pub(crate) fn set_position(&self, p: Point) -> Result<(), Unsupported> {
        if self.kind == Kind::Root {
            return Err(Unsupported::new(
                "moving windows",
                "the page lays out its own window; move it with CSS",
            ));
        }
        dom::style(&self.frame, &[("left", &format!("{}px", p.x)), ("top", &format!("{}px", p.y))]);
        post(HostEvent::Moved(self.id, p));
        Ok(())
    }

    pub(crate) fn set_size(&self, s: Size) -> Result<(), Unsupported> {
        if self.kind == Kind::Root {
            return Err(Unsupported::new(
                "sizing windows",
                "the page lays out its own window; size it with CSS",
            ));
        }
        let bar = if self.title_el.is_some() { TITLE_BAR } else { 0.0 };
        dom::style(
            &self.frame,
            &[("width", &format!("{}px", s.width)), ("height", &format!("{}px", f64::from(s.height) + bar))],
        );
        Ok(())
    }

    fn geometry(&self) -> [f64; 4] {
        let r = self.frame.get_bounding_client_rect();
        [r.left(), r.top(), r.width(), r.height()]
    }

    fn set_geometry(&self, g: [f64; 4]) {
        let [x, y, w, h] = g;
        dom::style(
            &self.frame,
            &[
                ("left", &format!("{x}px")),
                ("top", &format!("{y}px")),
                ("width", &format!("{w}px")),
                ("height", &format!("{h}px")),
            ],
        );
    }

    pub(crate) fn set_maximized(&self, on: bool) {
        if self.kind == Kind::Root || on == self.maximized.get() {
            return;
        }
        if on {
            self.restore.set(Some(self.geometry()));
            dom::style(
                &self.frame,
                &[("left", "0"), ("top", "0"), ("width", "100vw"), ("height", "100vh"), ("resize", "none")],
            );
        } else if let Some(g) = self.restore.take() {
            self.set_geometry(g);
            dom::style(&self.frame, &[("resize", "both")]);
        }
        self.maximized.set(on);
    }

    pub(crate) fn set_minimized(&self, on: bool) {
        if self.kind == Kind::Root || on == self.minimized.get() {
            return;
        }
        self.minimized.set(on);
        self.set_visible(self.visible.get());
        post(HostEvent::Occluded(self.id, on));
    }

    pub(crate) fn begin_move(&self) -> Result<(), Unsupported> {
        if self.kind == Kind::Root {
            return Err(Unsupported::new("moving windows", "the page's own window does not move"));
        }
        self.begin(Drag::Move);
        Ok(())
    }

    pub(crate) fn begin_resize(&self, edge: ResizeEdge) -> Result<(), Unsupported> {
        if self.kind == Kind::Root {
            return Err(Unsupported::new("resizing windows", "the page lays out its own window"));
        }
        self.begin(Drag::Resize(edge));
        Ok(())
    }

    /// Follows the pointer until it is released, moving or resizing the window.
    fn begin(&self, drag: Drag) {
        let Some((x0, y0)) = self.last_down.get() else { return };
        let Some(doc) = dom::document() else { return };
        let g = self.geometry();
        let id = self.id;
        let moved = move |e: web_sys::Event| {
            let Some(d) = super::driver() else { return };
            let Some(w) = d.window(id) else { return };
            let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() else { return };
            w.dragged(f64::from(p.client_x()), f64::from(p.client_y()));
        };
        let up = move |_: web_sys::Event| {
            if let Some(w) = super::driver().and_then(|d| d.window(id)) {
                w.end_drag();
            }
        };
        let listeners = vec![
            Listener::new(&doc, "pointermove", moved),
            Listener::new(&doc, "pointerup", up),
            Listener::new(&doc, "pointercancel", up),
        ];
        *self.drag.borrow_mut() = Some((drag, x0, y0, g, listeners));
    }

    fn dragged(&self, x: f64, y: f64) {
        let Some((drag, x0, y0, [gx, gy, gw, gh])) =
            self.drag.borrow().as_ref().map(|(d, x0, y0, g, _)| (*d, *x0, *y0, *g))
        else {
            return;
        };
        let (dx, dy) = (x - x0, y - y0);
        match drag {
            Drag::Move => {
                dom::style(
                    &self.frame,
                    &[("left", &format!("{}px", gx + dx)), ("top", &format!("{}px", gy + dy))],
                );
            }
            Drag::Resize(edge) => {
                use ResizeEdge as E;
                let (mut nx, mut ny, mut nw, mut nh) = (gx, gy, gw, gh);
                if matches!(edge, E::Left | E::TopLeft | E::BottomLeft) {
                    nx = gx + dx;
                    nw = gw - dx;
                }
                if matches!(edge, E::Right | E::TopRight | E::BottomRight) {
                    nw = gw + dx;
                }
                if matches!(edge, E::Top | E::TopLeft | E::TopRight) {
                    ny = gy + dy;
                    nh = gh - dy;
                }
                if matches!(edge, E::Bottom | E::BottomLeft | E::BottomRight) {
                    nh = gh + dy;
                }
                if nw >= 1.0 && nh >= 1.0 {
                    self.set_geometry([nx, ny, nw, nh]);
                }
            }
        }
    }

    fn end_drag(&self) {
        let ended = self.drag.borrow_mut().take();
        if let Some((Drag::Move, ..)) = ended {
            post(HostEvent::Moved(self.id, self.position()));
        }
    }

    pub(crate) fn is_fullscreen(&self) -> bool {
        dom::document().and_then(|d| d.fullscreen_element()).is_some_and(|e| e == *self.frame.as_ref())
    }

    pub(crate) fn set_fullscreen(&self, on: bool) -> Result<(), Unsupported> {
        let doc = dom::document()
            .ok_or_else(|| Unsupported::new("full-screen windows", "the page has no document"))?;
        if on == self.is_fullscreen() {
            return Ok(());
        }
        if !on {
            doc.exit_fullscreen();
            return Ok(());
        }
        if !doc.fullscreen_enabled() {
            return Err(Unsupported::new(
                "full-screen windows",
                "this page may not use full screen (a frame without allowfullscreen)",
            ));
        }
        if !super::input::user_active() {
            return Err(Unsupported::new(
                "full-screen windows",
                "a browser fills the screen only in answer to a click or key press",
            ));
        }
        self.frame.request_fullscreen().map_err(|e| Unsupported::new("full-screen windows", format!("{e:?}")))
    }

    /// Takes the window out of the page.
    pub(crate) fn remove(&self) {
        if let Some(o) = &self.observer {
            o.disconnect();
        }
        self.listeners.borrow_mut().clear();
        self.drag.borrow_mut().take();
        self.resolution.borrow_mut().take();
        if let Some(h) = self.pinch.take()
            && let Some(w) = web_sys::window()
        {
            w.clear_timeout_with_handle(h);
        }
        self.text.end();
        self.frame.remove();
    }

    pub(crate) fn handles(&self) -> crate::os::CanvasHandle {
        crate::os::CanvasHandle { id: self.canvas_id }
    }
}

/// A title bar for a decorated floating window: the title and a close button; dragging it
/// moves the window.
fn title_bar(frame: &HtmlElement, title: &str) -> Option<HtmlElement> {
    let bar: HtmlElement = dom::element("div")?;
    let label: HtmlElement = dom::element("span")?;
    let close: HtmlElement = dom::element("button")?;
    dom::style(
        &bar,
        &[
            ("flex", "0 0 auto"),
            ("height", &format!("{TITLE_BAR}px")),
            ("display", "flex"),
            ("align-items", "center"),
            ("padding", "0 0 0 12px"),
            ("font", "13px system-ui, sans-serif"),
            ("background", "ButtonFace"),
            ("color", "ButtonText"),
            ("user-select", "none"),
            ("cursor", "default"),
            ("touch-action", "none"),
        ],
    );
    dom::style(
        &label,
        &[
            ("flex", "1 1 auto"),
            ("overflow", "hidden"),
            ("text-overflow", "ellipsis"),
            ("white-space", "nowrap"),
        ],
    );
    label.set_text_content(Some(title));
    close.set_text_content(Some("\u{00d7}"));
    let _ = close.set_attribute("aria-label", "Close");
    let _ = close.set_attribute("type", "button");
    dom::style(
        &close,
        &[
            ("width", "44px"),
            ("height", "100%"),
            ("border", "none"),
            ("background", "transparent"),
            ("color", "inherit"),
            ("font", "18px system-ui, sans-serif"),
        ],
    );
    let _ = bar.append_child(&label);
    let _ = bar.append_child(&close);
    let _ = frame.append_child(&bar);
    Some(bar)
}
