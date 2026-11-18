//! The web bridge in a real browser (run with `wasm-bindgen-test-runner`, see
//! `tools/web-test.sh`): an app boots in the page, draws a frame that is read back from its
//! canvas, takes pointer, key, input-method and paste input, mirrors its accessibility tree
//! into ARIA elements that a click operates, carries a copy out through the browser's copy
//! event, and draws on the GPU (WebGPU or WebGL2) when it asks for it.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, RendererKind, WindowSpec};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

#[wasm_bindgen(inline_js = r#"
export function pointer(target, type, x, y, buttons) {
  target.dispatchEvent(new PointerEvent(type, {
    bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y,
    pointerId: 1, pointerType: 'mouse', isPrimary: true, buttons: buttons, button: type === 'pointermove' ? -1 : 0,
  }));
}
export function key(target, type, key, code, ctrl) {
  return target.dispatchEvent(new KeyboardEvent(type, { bubbles: true, cancelable: true, key: key, code: code, ctrlKey: ctrl }));
}
export function composition(target, type, data) {
  target.dispatchEvent(new CompositionEvent(type, { bubbles: true, cancelable: true, data: data }));
}
export function clipboard(target, type, text) {
  const dt = new DataTransfer();
  if (text !== null) dt.setData('text/plain', text);
  target.dispatchEvent(new ClipboardEvent(type, { bubbles: true, cancelable: true, clipboardData: dt }));
  return dt.getData('text/plain');
}
export function syntheticClipboardReadable() {
  const dt = new DataTransfer();
  dt.setData('text/plain', 'probe');
  let seen = '';
  const f = (e) => { seen = e.clipboardData ? e.clipboardData.getData('text/plain') : ''; e.preventDefault(); e.stopPropagation(); };
  window.addEventListener('paste', f, true);
  document.body.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: dt }));
  window.removeEventListener('paste', f, true);
  return seen === 'probe';
}
export function contextClick(target, x, y) {
  const init = { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y,
    pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 2 };
  target.dispatchEvent(new PointerEvent('pointerdown', { ...init, buttons: 2 }));
  target.dispatchEvent(new PointerEvent('pointerup', { ...init, buttons: 0 }));
  return !target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2 }));
}
export function nextFrame() {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}
export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
export function pixelAfterFrame(canvas, x, y) {
  // Runs after the bridge's own frame callback (registered earlier), before the browser
  // composites, while a GPU canvas still holds the frame just drawn.
  return new Promise((resolve) => requestAnimationFrame(() => {
    const c = document.createElement('canvas');
    c.width = canvas.width; c.height = canvas.height;
    const g = c.getContext('2d');
    g.drawImage(canvas, 0, 0);
    resolve(Array.from(g.getImageData(x, y, 1, 1).data));
  }));
}
export function context(canvas) {
  for (const kind of ['webgpu', 'webgl2']) {
    try { if (canvas.getContext(kind)) return kind; } catch (e) {}
  }
  return 'none';
}
"#)]
extern "C" {
    fn pointer(target: &web_sys::EventTarget, kind: &str, x: f64, y: f64, buttons: u16);
    fn key(target: &web_sys::EventTarget, kind: &str, key: &str, code: &str, ctrl: bool) -> bool;
    fn composition(target: &web_sys::EventTarget, kind: &str, data: &str);
    fn clipboard(target: &web_sys::EventTarget, kind: &str, text: Option<String>) -> String;
    #[wasm_bindgen(js_name = syntheticClipboardReadable)]
    fn synthetic_clipboard_readable() -> bool;
    /// A right click; true when the page's context menu was cancelled.
    #[wasm_bindgen(js_name = contextClick)]
    fn context_click(target: &web_sys::EventTarget, x: f64, y: f64) -> bool;
    #[wasm_bindgen(js_name = nextFrame)]
    fn next_frame() -> js_sys::Promise;
    fn sleep(ms: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = pixelAfterFrame)]
    fn pixel_after_frame(canvas: &web_sys::HtmlCanvasElement, x: u32, y: u32) -> js_sys::Promise;
    fn context(canvas: &web_sys::HtmlCanvasElement) -> String;
}

async fn frames(n: u32) {
    for _ in 0..n {
        let _ = JsFuture::from(next_frame()).await;
    }
}

async fn wait_ms(ms: u32) {
    let _ = JsFuture::from(sleep(ms)).await;
}

/// Waits (a frame at a time, up to `limit` frames) for `ok`.
async fn until(limit: u32, ok: impl Fn() -> bool) -> bool {
    for _ in 0..limit {
        if ok() {
            return true;
        }
        frames(1).await;
        // Background work runs in slices between frames.
        wait_ms(1).await;
    }
    ok()
}

fn document() -> web_sys::Document {
    web_sys::window().and_then(|w| w.document()).must("a document")
}

fn query(sel: &str) -> Option<web_sys::HtmlElement> {
    document().query_selector(sel).ok().flatten().and_then(|e| e.dyn_into().ok())
}

/// The client-pixel center of a mirror element.
fn center(el: &web_sys::Element) -> (f64, f64) {
    let r = el.get_bounding_client_rect();
    (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0)
}

/// One pixel of a 2D canvas, as RGBA.
fn pixel(ctx: &web_sys::CanvasRenderingContext2d, x: f64, y: f64) -> [u8; 4] {
    let data = ctx.get_image_data(x, y, 1.0, 1.0).must("pixels").data().0;
    let Ok(px) = <[u8; 4]>::try_from(data.as_slice()) else { unreachable!("one RGBA pixel: {data:?}") };
    px
}

/// The mirror element showing exactly `text`.
fn mirrored_text(text: &str) -> Option<web_sys::Element> {
    let all = document().query_selector_all("[data-node]").ok()?;
    (0..all.length()).filter_map(|i| all.item(i)).filter_map(|n| n.dyn_into::<web_sys::Element>().ok()).find(
        |e| {
            e.child_element_count() == 0 && e.text_content().as_deref() == Some(text)
                || e.get_attribute("aria-label").as_deref() == Some(text)
        },
    )
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

#[derive(Default)]
struct Seen {
    taps: Cell<u32>,
    typed: RefCell<String>,
    app: RefCell<Option<App>>,
}

fn start(config: AppConfig, seen: Rc<Seen>) {
    let s = seen.clone();
    let run = remedy_ui::run(config, move |app: &App| {
        *s.app.borrow_mut() = Some(app.clone());
        let s2 = s.clone();
        app.open(WindowSpec::new("Browser test"), move |ui, _| {
            let field = w::text_input(ui, "Message");
            let f2 = field.clone();
            let s3 = s2.clone();
            let field = field.on_change(move || *s3.typed.borrow_mut() = f2.value());
            let s4 = s2.clone();
            // Shown, so a press changes what is drawn.
            let shown = remedy_ui::reactive::Signal::new(0u32);
            let col = w::column(ui)
                .gap(12.0)
                .padding(remedy_ui::core::Insets { top: 16.0, end: 16.0, bottom: 16.0, start: 16.0 })
                .child(w::button(ui, "Tap me", move || {
                    s4.taps.set(s4.taps.get() + 1);
                    shown.update(|n| *n += 1);
                }))
                .child(ui.el("label").text(move || format!("Taps: {}", shown.get())))
                .child(field.el())
                .child(w::context_menu(&ui.el("label").text("Menu here"), || {
                    vec![w::MenuItem::new("Do something", || {})]
                }));
            let _ = ui.root().child(col);
        });
    });
    run.must("the app starts in the page");
}

fn renderer(seen: &Seen) -> Option<RendererKind> {
    seen.app.borrow().as_ref().and_then(|a| a.windows().first().and_then(|w| w.renderer()))
}

#[wasm_bindgen_test]
async fn an_app_runs_in_the_page() {
    let theme_bg = AppConfig::new("x").theme.colors.bg.to_rgba8();
    let seen = Rc::new(Seen::default());
    start(AppConfig::new("Browser test").cpu_only(), seen.clone());
    assert!(remedy_ui::web::is_running());

    // A frame: the window's CPU canvas holds the theme's background where nothing is drawn.
    assert!(until(120, || renderer(&seen) == Some(RendererKind::Cpu)).await, "the window drew a frame");
    frames(3).await;
    let surface = query("[data-remedy-window] > div, [data-remedy-window]").must("the window's element");
    let canvas: web_sys::HtmlCanvasElement = document()
        .query_selector("[data-remedy-window] canvas")
        .ok()
        .flatten()
        .must("a canvas")
        .dyn_into()
        .must("a canvas");
    assert!(canvas.width() > 0 && canvas.height() > 0, "the canvas has the window's device size");
    let ctx: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok().flatten().must("the 2D context").dyn_into().must("a 2D context");
    let corner_px = pixel(&ctx, f64::from(canvas.width() - 2), f64::from(canvas.height() - 2));
    assert!(close(corner_px, theme_bg), "the background is the theme's: {corner_px:?} vs {theme_bg:?}");

    // The accessibility mirror: the button is an ARIA button named by its label.
    assert!(
        until(60, || query("[role=button][aria-label='Tap me']").is_some()).await,
        "the mirror shows the button"
    );
    let button = query("[role=button][aria-label='Tap me']").must("the mirrored button");
    let field = query("[role=textbox][aria-label='Message']").must("the mirrored text field");
    // Something is drawn where the button is.
    let (bx, by) = center(&button);
    let cr = canvas.get_bounding_client_rect();
    let scale = f64::from(canvas.width()) / cr.width();
    let at = pixel(&ctx, ((bx - cr.left()) * scale).floor(), ((by - cr.top()) * scale).floor());
    assert!(!close(at, theme_bg), "the button is drawn on the canvas: {at:?}");

    // A screen reader's click on the mirror reaches the button.
    button.click();
    assert!(until(30, || seen.taps.get() == 1).await, "the mirror's click pressed the button");

    // A real pointer press and release on the canvas presses it too.
    let target: web_sys::EventTarget = canvas.clone().into();
    pointer(&target, "pointerdown", bx, by, 1);
    pointer(&target, "pointerup", bx, by, 0);
    assert!(until(30, || seen.taps.get() == 2).await, "the pointer pressed the button");

    // Clicking the field starts text input in the hidden field at its caret.
    let (fx, fy) = center(&field);
    pointer(&target, "pointerdown", fx, fy, 1);
    pointer(&target, "pointerup", fx, fy, 0);
    assert!(
        until(30, || query("textarea[data-remedy-text]:focus").is_some()).await,
        "the hidden field took focus"
    );
    let text: web_sys::EventTarget = query("textarea[data-remedy-text]").must("the hidden field").into();
    let shown = query("textarea[data-remedy-text]").must("the hidden field");
    let named = until(30, || shown.get_attribute("aria-label").as_deref() == Some("Message")).await;
    assert!(named, "the field is named for screen readers: {:?}", shown.get_attribute("aria-label"));

    // Keys type.
    for (k, c) in [("h", "KeyH"), ("i", "KeyI")] {
        let not_cancelled = key(&text, "keydown", k, c, false);
        assert!(!not_cancelled, "a typed key is the app's, not the hidden field's");
        key(&text, "keyup", k, c, false);
    }
    assert!(until(30, || *seen.typed.borrow() == "hi").await, "keys typed: {:?}", seen.typed.borrow());

    // An input method composes and commits.
    composition(&text, "compositionstart", "");
    composition(&text, "compositionupdate", "にほ");
    composition(&text, "compositionupdate", "にほん");
    composition(&text, "compositionend", "日本");
    assert!(
        until(30, || *seen.typed.borrow() == "hi日本").await,
        "the composition committed: {:?}",
        seen.typed.borrow()
    );

    // A paste brings the browser's clipboard text in, and the browser's copy event carries the
    // app's copy out. Firefox hides the data of clipboard events a page makes itself (it is
    // only readable in trusted ones), so there the check is what the browser lets a test see.
    let readable = synthetic_clipboard_readable();
    let ctrl_v = key(&text, "keydown", "v", "KeyV", true);
    assert!(ctrl_v, "the paste shortcut reaches the browser, so it raises its paste event");
    clipboard(&text, "paste", Some(" pasted".into()));
    key(&text, "keyup", "v", "KeyV", true);
    let expected = if readable { "hi日本 pasted" } else { "hi日本" };
    let pasted = until(30, || *seen.typed.borrow() == expected).await;
    let now = seen.typed.borrow().clone();
    assert!(pasted, "after the paste: {now:?}");
    web_sys::console::log_1(
        &format!("synthetic clipboard data readable: {readable}; field now {now:?}").into(),
    );

    // Select all, then the browser's copy event carries the selection out.
    key(&text, "keydown", "a", "KeyA", true);
    key(&text, "keyup", "a", "KeyA", true);
    frames(2).await;
    let copied = clipboard(&text, "copy", None);
    if readable {
        assert_eq!(copied, expected, "the copy event carried the app's copy");
    }

    // The browser's context menu: it opens where the app does nothing with a right click,
    // and stays away where the app opens its own.
    key(&text, "keydown", "Escape", "Escape", false);
    key(&text, "keyup", "Escape", "Escape", false);
    frames(2).await;
    let (ex, ey) = (cr.right() - 4.0, cr.bottom() - 4.0);
    assert!(!context_click(&target, ex, ey), "a right click the app ignores keeps the browser's menu");
    let menu_here = until(30, || mirrored_text("Menu here").is_some()).await;
    assert!(menu_here, "the mirror shows the context menu's target");
    let (mx, my) = center(&mirrored_text("Menu here").must("the target"));
    assert!(context_click(&target, mx, my), "the app's own context menu keeps the browser's away");
    assert!(until(30, || mirrored_text("Do something").is_some()).await, "the app's menu opened");
    key(&target, "keydown", "Escape", "Escape", false);
    key(&target, "keyup", "Escape", "Escape", false);

    // Quitting takes the app out of the page.
    let app = seen.app.borrow().clone().must("the app");
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await, "quitting ends the app and empties the page");
    assert!(query("[data-remedy-window]").is_none(), "the window left the page");
    let _ = surface;

    // The GPU: an app that asks for it draws on WebGPU or WebGL2, made asynchronously.
    let seen = Rc::new(Seen::default());
    start(AppConfig::new("Browser GPU test").gpu_first(), seen.clone());
    let decided = until(600, || renderer(&seen) == Some(RendererKind::Gpu)).await;
    let gpu: web_sys::HtmlCanvasElement = document()
        .query_selector("canvas[data-raw-handle]")
        .ok()
        .flatten()
        .must("the GPU canvas")
        .dyn_into()
        .must("a canvas");
    let kind = context(&gpu);
    web_sys::console::log_1(&format!("GPU renderer: {decided}, canvas context {kind}").into());
    assert!(decided, "the window moved to the GPU once the browser made the device");
    assert!(kind == "webgpu" || kind == "webgl2", "the GPU canvas draws with {kind}");
    assert_eq!(
        gpu.style().get_property_value("display").ok().as_deref(),
        Some("block"),
        "the GPU canvas shows"
    );
    // Read a frame back from the GPU canvas: the app changes, the bridge draws, and the
    // picture is taken in the same frame.
    let button = query("[role=button][aria-label='Tap me']").must("the mirrored button");
    button.click();
    let px =
        JsFuture::from(pixel_after_frame(&gpu, gpu.width() - 2, gpu.height() - 2)).await.must("GPU pixels");
    let px: Vec<u8> = js_sys::Array::from(&px).iter().filter_map(|v| v.as_f64()).map(|v| v as u8).collect();
    let Ok(px) = <[u8; 4]>::try_from(px.as_slice()) else { unreachable!("one RGBA pixel: {px:?}") };
    assert!(close(px, theme_bg), "the GPU frame shows the theme's background: {px:?} vs {theme_bg:?}");
    let app = seen.app.borrow().clone().must("the app");
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
}
