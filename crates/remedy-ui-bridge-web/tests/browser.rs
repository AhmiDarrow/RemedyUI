//! The web bridge in a real browser (run with `wasm-bindgen-test-runner`, see
//! `tools/web-test.sh`): an app boots in the page, draws a frame that is read back from its
//! canvas, takes pointer, key, input-method and paste input, mirrors its accessibility tree
//! into ARIA elements that a click operates, carries a copy out through the browser's copy
//! event, hands shares to `navigator.share` (a recording stand-in for the sheet, since a test's
//! calls are no click), and draws on the GPU (WebGPU or WebGL2) when it asks for it.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui::host::{ShareFile, ShareOptions, ShareOutcome};
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, InputPurpose, RendererKind, WindowSpec};
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
export function stubShare(files, fail) {
  window.__shares = [];
  const own = { configurable: true, writable: true };
  Object.defineProperty(navigator, 'share', { ...own, value: (data) => {
    window.__shares.push({
      title: data.title ?? null,
      text: data.text ?? null,
      files: data.files ? data.files.map((f) => f.name + '|' + f.type + '|' + f.size) : null,
    });
    return fail ? Promise.reject(new DOMException('stubbed', fail)) : Promise.resolve();
  } });
  Object.defineProperty(navigator, 'canShare', { ...own, value: (data) => files || !data.files });
  Object.defineProperty(navigator, 'userActivation', { ...own, value: { isActive: true } });
}
export function unstubShare() {
  delete navigator.share;
  delete navigator.canShare;
  delete navigator.userActivation;
}
export function sharesSeen() {
  return JSON.stringify(window.__shares || []);
}
export function context(canvas) {
  for (const kind of ['webgpu', 'webgl2']) {
    try { if (canvas.getContext(kind)) return kind; } catch (e) {}
  }
  return 'none';
}
export function mouse(target, type, x, y, button, buttons, ctrl) {
  const init = {
    bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, button: button, buttons: buttons,
    ctrlKey: ctrl, pointerId: 1, pointerType: 'mouse', isPrimary: true,
  };
  const E = type.startsWith('pointer') ? PointerEvent : MouseEvent;
  return target.dispatchEvent(new E(type, init));
}
export function recordOpens() {
  window.__opened = [];
  if (window.__realOpen) return;
  window.__realOpen = window.open.bind(window);
  window.open = (...a) => { const w = window.__realOpen(...a); window.__opened.push(w); return w; };
}
export function openedHashes() {
  return (window.__opened || []).map(w => { try { return w ? w.location.hash : 'blocked'; } catch (e) { return 'other'; } }).join(' ');
}
export function closeOpened() {
  (window.__opened || []).forEach(w => { if (w) w.close(); });
  window.__opened = [];
}
export function insertText(text) {
  // As automation, a paste from the context menu or a password manager inserts: no keys.
  return document.execCommand('insertText', false, text);
}
export function hidePage(persisted) {
  window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: !!persisted }));
}
"#)]
extern "C" {
    fn pointer(target: &web_sys::EventTarget, kind: &str, x: f64, y: f64, buttons: u16);
    /// A mouse or pointer event with its button, held buttons and Ctrl; false when the
    /// page cancelled it.
    fn mouse(
        target: &web_sys::EventTarget,
        kind: &str,
        x: f64,
        y: f64,
        button: i16,
        buttons: u16,
        ctrl: bool,
    ) -> bool;
    /// Starts recording the windows `window.open` opens.
    #[wasm_bindgen(js_name = recordOpens)]
    fn record_opens();
    /// The address fragment of each window opened since, space-separated.
    #[wasm_bindgen(js_name = openedHashes)]
    fn opened_hashes() -> String;
    #[wasm_bindgen(js_name = closeOpened)]
    fn close_opened();
    /// What the browser sends as the page is put away (and before a reload).
    #[wasm_bindgen(js_name = hidePage)]
    fn hide_page_as(cached: bool);
    /// Text inserted into the focused field without key presses; false if the browser refused.
    #[wasm_bindgen(js_name = insertText)]
    fn insert_text(text: &str) -> bool;
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
    /// Replaces the browser's share sheet with one that records what it gets and answers
    /// at once (with `fail` as the error's name when set), as if a click had just happened.
    #[wasm_bindgen(js_name = stubShare)]
    fn stub_share(files: bool, fail: Option<String>);
    #[wasm_bindgen(js_name = unstubShare)]
    fn unstub_share();
    #[wasm_bindgen(js_name = sharesSeen)]
    fn shares_seen() -> String;
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
    caps: Cell<Option<remedy_ui::reactive::Signal<remedy_ui::PointerCaps>>>,
    /// The username and password fields, for text inserted without keys.
    login: RefCell<Option<(remedy_ui::TextField, remedy_ui::TextField)>>,
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
            s2.caps.set(Some(ui.pointer_caps()));
            let purposed = |label: &str, p| w::text_input(ui, label).purpose(p).el().height(32.0);
            let user = w::text_input(ui, "User")
                .purpose(InputPurpose::Username)
                .enter_key(remedy_ui::EnterKey::Next);
            let pass = w::secret_input(ui, "Pass").purpose(InputPurpose::CurrentPassword);
            *s2.login.borrow_mut() = Some((user.clone(), pass.clone()));
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
                .child(user.el().height(32.0))
                .child(pass.el().height(32.0))
                .child(purposed("Code", InputPurpose::OneTimeCode))
                .child(purposed("Site", InputPurpose::Url))
                .child(purposed("Find", InputPurpose::Search))
                .child(
                    ui.text_field("text_area", remedy_ui::FieldKind::Multi).label("Notes").el().height(48.0),
                )
                .child(w::context_menu(&ui.el("label").text("Menu here"), || {
                    vec![w::MenuItem::new("Do something", || {})]
                }))
                .child(w::select(
                    ui,
                    "Fruit",
                    vec!["Apple".into(), "Pear".into()],
                    remedy_ui::reactive::Signal::new(0),
                ));
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

    // The user's languages are the browser's, in its order.
    let languages: Vec<String> = web_sys::window()
        .must("a window")
        .navigator()
        .languages()
        .iter()
        .filter_map(|l| l.as_string())
        .collect();
    assert!(!languages.is_empty(), "the browser lists a language");
    let preferred = seen.app.borrow().as_ref().map(App::preferred_locales).must("the app");
    assert_eq!(preferred, languages, "navigator.languages");
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

    // The window's own icon is the page's icon (the window's title is the page's), and goes
    // again when the window gives it back.
    let window = seen.app.borrow().as_ref().and_then(|a| a.windows().first().cloned()).must("the window");
    let icon = remedy_ui::host::WindowIcon::new(2, [251, 191, 36, 255].repeat(4)).must("2 × 2");
    window.set_icon(Some(icon));
    let page_icon = || {
        document()
            .query_selector("link[rel=icon][data-remedy-icon]")
            .ok()
            .flatten()
            .and_then(|l| l.get_attribute("href"))
    };
    assert!(
        until(30, || page_icon().is_some_and(|h| h.starts_with("data:image/png"))).await,
        "the page shows the window's icon: {:?}",
        page_icon()
    );
    window.set_icon(None);
    assert!(until(30, || page_icon().is_none()).await, "and the page's own again");

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
        until(30, || query("input[data-remedy-text]:focus").is_some()).await,
        "the hidden field took focus"
    );
    let text: web_sys::EventTarget = query("input[data-remedy-text]").must("the hidden field").into();
    let shown = query("input[data-remedy-text]").must("the hidden field");
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
    assert!(until(30, || mirrored_text("Do something").is_none()).await, "Escape closed the menu");

    // A list box opened from the keyboard: Escape where the browser sends it (the focused
    // option's mirror) closes it, and focus goes back to the select.
    let active = || document().active_element().and_then(|e| e.get_attribute("aria-label"));
    let fruit = query("[role=combobox][aria-label='Fruit']").must("the mirrored select");
    let (fx, fy) = center(&fruit);
    pointer(&target, "pointerdown", fx, fy, 1);
    pointer(&target, "pointerup", fx, fy, 0);
    assert!(until(30, || query("[role=option]").is_some()).await, "the list opened");
    let on_option =
        || document().active_element().and_then(|e| e.get_attribute("role")).as_deref() == Some("option");
    assert!(until(30, on_option).await, "focus is on an option's mirror");
    let option: web_sys::EventTarget = document().active_element().must("a focused option").into();
    key(&option, "keydown", "Escape", "Escape", false);
    key(&option, "keyup", "Escape", "Escape", false);
    assert!(until(30, || query("[role=option]").is_none()).await, "Escape closed the list");
    assert!(until(30, || active().as_deref() == Some("Fruit")).await, "focus went back: {:?}", active());
    key(&target, "keydown", "Escape", "Escape", false);
    key(&target, "keyup", "Escape", "Escape", false);
    frames(2).await;

    // Each field's purpose reaches the hidden field: soft keyboards pick their keys from it,
    // and password managers find the username and the current password.
    let attrs = |el: &web_sys::HtmlElement| {
        ["type", "inputmode", "autocomplete", "enterkeyhint", "autocapitalize"]
            .map(|a| el.get_attribute(a).unwrap_or_default())
            .join(" ")
    };
    for (label, sel, want) in [
        ("User", "input", "text  username next off"),
        ("Pass", "input", "password  current-password done off"),
        ("Code", "input", "text numeric one-time-code done off"),
        ("Site", "input", "url url off go off"),
        ("Find", "input", "search search off search "),
        ("Notes", "textarea", "  off enter "),
        ("Message", "input", "text  off done "),
    ] {
        // A search field is a search box to assistive technology; the others text boxes.
        let role = if label == "Find" { "searchbox" } else { "textbox" };
        let mirrored = query(&format!("[role={role}][aria-label='{label}']")).must("the mirrored field");
        let (x, y) = center(&mirrored);
        pointer(&target, "pointerdown", x, y, 1);
        pointer(&target, "pointerup", x, y, 0);
        let hidden = format!("{sel}[data-remedy-text]:focus");
        let ok = until(30, || {
            query(&hidden).is_some_and(|h| h.get_attribute("aria-label").as_deref() == Some(label))
        })
        .await;
        assert!(ok, "{label}: the hidden {sel} took focus");
        let got = attrs(&query(&hidden).must("the hidden field"));
        assert_eq!(got, want, "{label}: type inputmode autocomplete enterkeyhint autocapitalize");
        // Text inserted without keys (automation, a paste from the browser's menu, a password
        // manager filling the form) reaches plain and secret fields alike.
        if matches!(label, "User" | "Pass") {
            assert!(insert_text("ab1"), "{label}: the browser inserted the text");
            let (user, pass) = seen.login.borrow().clone().must("the login fields");
            let field = if label == "User" { user } else { pass };
            assert!(until(30, || field.len() == 3).await, "{label}: inserted text reached the field");
            if label == "Pass" {
                assert_eq!(field.take_secret().expose(), "ab1", "the secret holds the inserted text");
                let cleared = until(30, || {
                    query(&hidden).is_some_and(|h| {
                        h.dyn_ref::<web_sys::HtmlInputElement>().is_some_and(|i| i.value().is_empty())
                    })
                })
                .await;
                assert!(cleared, "the hidden password field does not keep the secret");
            }
        }
    }
    key(&target, "keydown", "Escape", "Escape", false);
    key(&target, "keyup", "Escape", "Escape", false);

    // The pointer's caps come from the page's media queries.
    let win = web_sys::window().must("a window");
    let mq = |q: &str| win.match_media(q).ok().flatten().is_some_and(|m| m.matches());
    let caps = seen.caps.get().must("the caps").get_untracked().ok();
    let page = remedy_ui::PointerCaps::new(mq("(hover: hover)"), mq("(pointer: fine)"));
    web_sys::console::log_1(&format!("pointer caps {caps:?}, page {page:?}").into());
    assert_eq!(caps, Some(page), "the caps match (hover: hover) and (pointer: fine)");

    // Sharing. The test's calls are no click, so the browser's own sheet refuses: either it
    // has none (desktop Firefox, Chrome on Linux) or the page may not act for the person now.
    let app = seen.app.borrow().clone().must("the app");
    let outcomes: Rc<RefCell<Vec<ShareOutcome>>> = Rc::default();
    let share = |options: ShareOptions| {
        let o = outcomes.clone();
        app.share(None, options, move |outcome| o.borrow_mut().push(outcome));
    };
    let recipe = ShareOptions::new()
        .title("Lavender")
        .text("Lavender soap: 500 g oils")
        .file(ShareFile::new("Lavender.alex-soap.json", "application/json", br#"{"oils":500}"#.to_vec()));
    let has_sheet = js_sys::Reflect::has(&web_sys::window().must("a window").navigator(), &"share".into())
        .unwrap_or(false);
    share(recipe.clone());
    assert!(until(30, || outcomes.borrow().len() == 1).await, "the share ended");
    let refused = outcomes.borrow_mut().remove(0);
    web_sys::console::log_1(
        &format!("navigator.share present: {has_sheet}; outside a click: {refused:?}").into(),
    );
    let ShareOutcome::Unsupported(why) = refused else {
        unreachable!("refused outside a click: {refused:?}")
    };
    assert!(why.reason.contains(if has_sheet { "click or key press" } else { "navigator.share" }), "{why}");
    // A sheet that takes files gets them as File objects beside the title and text.
    stub_share(true, None);
    share(recipe.clone());
    assert!(until(30, || outcomes.borrow().len() == 1).await, "the stubbed share ended");
    assert_eq!(outcomes.borrow_mut().remove(0), ShareOutcome::Shared);
    assert_eq!(
        shares_seen(),
        r#"[{"title":"Lavender","text":"Lavender soap: 500 g oils","files":["Lavender.alex-soap.json|application/json|12"]}]"#
    );
    // One that takes no files gets the text alone, unless the app requires the files.
    stub_share(false, None);
    share(recipe.clone());
    share(recipe.clone().require_files(true));
    assert!(until(30, || outcomes.borrow().len() == 2).await, "both shares ended");
    // The refusal comes at once, the sheet's answer when its promise settles.
    assert!(
        matches!(outcomes.borrow().first(), Some(ShareOutcome::Unsupported(u)) if u.reason.contains("files"))
    );
    assert_eq!(outcomes.borrow().get(1), Some(&ShareOutcome::Shared));
    assert_eq!(shares_seen(), r#"[{"title":"Lavender","text":"Lavender soap: 500 g oils","files":null}]"#);
    outcomes.borrow_mut().clear();
    // The person closing the sheet is a cancel; any other refusal says the browser's reason.
    stub_share(true, Some("AbortError".into()));
    share(recipe.clone());
    assert!(until(30, || outcomes.borrow().len() == 1).await);
    assert_eq!(outcomes.borrow_mut().remove(0), ShareOutcome::Cancelled);
    stub_share(true, Some("NotAllowedError".into()));
    share(recipe);
    assert!(until(30, || outcomes.borrow().len() == 1).await);
    let denied = outcomes.borrow_mut().remove(0);
    assert!(
        matches!(&denied, ShareOutcome::Unsupported(u) if u.reason.contains("NotAllowedError")),
        "{denied:?}"
    );
    unstub_share();

    // Quitting takes the app out of the page.
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await, "quitting ends the app and empties the page");
    assert!(query("[data-remedy-window]").is_none(), "the window left the page");
    let _ = surface;

    // The GPU: an app that asks for it draws on WebGPU or WebGL2, made asynchronously.
    let seen = Rc::new(Seen::default());
    start(AppConfig::new("Browser GPU test").gpu_first(), seen.clone());
    // The page keeps drawing on the CPU while the browser starts its GL, which can take
    // minutes on a loaded machine (headless Firefox): wait for it as long as it takes. The
    // page's thread is never stalled meanwhile: `getContext("webgl2")` blocks until the GL
    // is up, so the renderer waits for a worker's context first (a page froze for minutes
    // before). Making the device itself (pipelines, in a debug build) takes a few seconds.
    watch_gaps();
    let decided = until(36_000, || renderer(&seen) == Some(RendererKind::Gpu)).await;
    let gap = worst_gap();
    web_sys::console::log_1(&format!("longest stall while the GPU was made: {gap:.0} ms").into());
    assert!(gap < 8000.0, "the page's thread was never stalled for long ({gap:.0} ms)");
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

#[wasm_bindgen(inline_js = r#"
export function storageGet(key) { return window.localStorage.getItem(key); }
export function storageRemove(key) { window.localStorage.removeItem(key); }
"#)]
extern "C" {
    #[wasm_bindgen(js_name = storageGet)]
    fn storage_get(key: &str) -> Option<String>;
    #[wasm_bindgen(js_name = storageRemove)]
    fn storage_remove(key: &str);
}

type Persisting = Rc<RefCell<Option<(App, remedy_ui::reactive::Signal<String>)>>>;

/// An app with a persisted draft, as Craft Calc keeps its open recipe.
fn persisting_app(name: &str, seen: Persisting) {
    let run = remedy_ui::run(AppConfig::new(name).cpu_only(), move |app: &App| {
        let (s, a) = (seen.clone(), app.clone());
        app.open(WindowSpec::new("Persist").persist_geometry("main"), move |ui, _| {
            let draft = remedy_ui::reactive::Signal::new(String::new()).persist("draft");
            *s.borrow_mut() = Some((a.clone(), draft));
            let _ = ui.root().child(ui.el("label").text(move || draft.get()));
        });
    });
    run.must("the app starts in the page");
}

#[wasm_bindgen_test]
async fn persisted_state_and_documents_live_in_local_storage_across_runs() {
    use remedy_ui::reactive::Persist;
    const NAME: &str = "Browser persist test";
    const DRAFT: &str = "remedy:Browser persist test:draft";
    const RECIPES: &str = "remedy:Browser persist test:recipes";
    for k in [DRAFT, RECIPES, "remedy:Browser persist test:remedy-ui.windows"] {
        storage_remove(k);
    }

    // The first run: a persisted signal and a document are saved in the page's storage.
    let seen: Persisting = Rc::default();
    persisting_app(NAME, seen.clone());
    assert!(until(120, || seen.borrow().is_some()).await, "the window was built");
    let (app, draft) = seen.borrow().clone().must("the app");
    assert_eq!(app.data_dir(), None, "a page has no folder");
    let store = app.persist_store().must("a page has a persist store");
    assert_eq!(store.storage_key_of("draft").as_deref(), Some(DRAFT));
    draft.set("Castile soap".into());
    assert!(
        until(240, || storage_get(DRAFT).is_some_and(|v| v.contains("Castile soap"))).await,
        "the debounced write reached localStorage between frames"
    );
    let saved = storage_get(DRAFT).must("the entry");
    assert!(saved.contains("\"version\": 1"), "the same envelope as a file: {saved}");
    let recipes = vec!["Castile".to_owned(), "Bastille".to_owned()];
    store.save(&Persist::new("recipes"), &recipes).must("a document saves");
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
    assert!(storage_get(RECIPES).is_some(), "quitting wrote what was pending");
    drop((app, draft, store));

    // The next run, as after a reload: everything comes back from the page's storage.
    let again: Persisting = Rc::default();
    persisting_app(NAME, again.clone());
    assert!(until(120, || again.borrow().is_some()).await, "the window was built again");
    let (app, draft) = again.borrow().clone().must("the app");
    assert_eq!(draft.get(), "Castile soap", "the persisted signal came back");
    let store = app.persist_store().must("a page has a persist store");
    let back: Option<Vec<String>> = store.load(&Persist::new("recipes")).must("the document reads back");
    assert_eq!(back, Some(recipes));

    // More than the browser lets an origin keep: the write fails and is reported.
    let big = "x".repeat(16 * 1024 * 1024);
    store.save(&Persist::new("too big"), &big).must("queued");
    let flushed = store.flush();
    let errors = store.take_errors();
    assert!(flushed.is_err(), "the browser refused the write");
    assert!(
        errors
            .iter()
            .any(|e| e.starts_with("remedy:Browser persist test:too big")
                && e.to_lowercase().contains("quota")),
        "the refusal is reported: {errors:?}"
    );
    store.remove("too big").must("removed");
    store.flush().must("nothing left to refuse");

    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
    for k in [DRAFT, RECIPES, "remedy:Browser persist test:remedy-ui.windows"] {
        storage_remove(k);
    }
}

type Routed = Rc<RefCell<Option<(App, remedy_ui::router::Router)>>>;

#[wasm_bindgen_test]
async fn routes_live_in_the_address_and_the_browser_history() {
    use remedy_ui::router::Router;
    let window = web_sys::window().must("a window");
    let history = window.history().must("history");
    // Opened at a deep link.
    history.replace_state_with_url(&JsValue::NULL, "", Some("#/settings/7")).must("the address");
    let seen: Routed = Rc::default();
    let s = seen.clone();
    let run = remedy_ui::run(AppConfig::new("Browser router test").cpu_only(), move |app: &App| {
        let a = app.clone();
        app.open(WindowSpec::new("Router"), move |ui, window| {
            let router = Router::for_window(window, "/home");
            router.set_title(|r| format!("Routed {}", r.path()));
            let (me, ui2) = (router.clone(), ui.clone());
            let view = router.view(ui, move |route| {
                w::column(&ui2)
                    .child(w::text(&ui2, format!("Page {}", route.path())))
                    .child(me.link_to(&ui2, "/home", "Home"))
            });
            let _ = ui.root().child(view);
            *s.borrow_mut() = Some((a.clone(), router));
        });
    });
    run.must("the app starts in the page");
    assert!(until(120, || seen.borrow().is_some()).await, "the window was built");
    let (app, router) = seen.borrow().clone().must("the router");
    let location = window.location();
    assert_eq!(router.route_untracked().path(), "/settings/7", "the deep link at load");
    assert!(until(60, || document().title() == "Routed /settings/7").await, "the page's title");

    // A link in the accessibility mirror is a real <a href>.
    assert!(until(120, || query("a[href='#/home']").is_some()).await, "the link's href in the mirror");

    // A push is a history entry and an address.
    let before = history.length().must("length");
    router.push("/browse/TV");
    assert_eq!(location.hash().must("hash"), "#/browse/TV");
    assert_eq!(history.length().must("length"), before + 1);
    assert!(until(60, || mirrored_text("Page /browse/TV").is_some()).await, "the page changed");

    // The browser's back button (history.back) brings the route back, and forward again.
    history.back().must("back");
    assert!(until(120, || router.route_untracked().path() == "/settings/7").await, "back");
    assert!(until(60, || document().title() == "Routed /settings/7").await);
    history.forward().must("forward");
    assert!(until(120, || router.route_untracked().path() == "/browse/TV").await, "forward");

    // The router's own back goes through the browser's history.
    assert!(router.back());
    assert!(until(120, || location.hash().ok().as_deref() == Some("#/settings/7")).await);

    // An address typed in (a fragment navigation) routes too.
    location.set_hash("/server").must("hash");
    assert!(until(120, || router.route_untracked().path() == "/server").await, "an edited address");

    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
    history.replace_state_with_url(&JsValue::NULL, "", Some("#")).must("reset");
}

/// The app, its router and the current page's scroll container.
type Paged = Rc<RefCell<Option<(App, remedy_ui::router::Router, Option<remedy_ui::view::El>)>>>;

/// An app whose routed pages have a link to `/settings` and a tall scrolling list.
fn paged_app(name: &str, seen: Paged) {
    use remedy_ui::layout::Length;
    use remedy_ui::router::Router;
    let run = remedy_ui::run(AppConfig::new(name).cpu_only(), move |app: &App| {
        let a = app.clone();
        app.open(WindowSpec::new("Paged"), move |ui, window| {
            let router = Router::for_window(window, "/home");
            let (me, ui2, s) = (router.clone(), ui.clone(), seen.clone());
            let view = router.view(ui, move |route| {
                let rows = (0..80).fold(w::column(&ui2), |c, i| c.child(w::text(&ui2, format!("row {i}"))));
                let scroll = w::scroll(&ui2).child(rows).layout(|l| l.height = Length::Px(160.0));
                me.remember_scroll(&scroll);
                if let Some(entry) = s.borrow_mut().as_mut() {
                    entry.2 = Some(scroll.clone());
                }
                w::column(&ui2)
                    .grow(1.0)
                    .child(w::text(&ui2, format!("Page {}", route.path())))
                    .child(me.link_to(&ui2, "/settings", "Settings"))
                    .child(scroll)
            });
            let _ = ui.root().child(view);
            *seen.borrow_mut() = Some((a.clone(), router, None));
        });
    });
    run.must("the app starts in the page");
}

/// Where the current page's list is scrolled to, as the window shows it.
fn shown_scroll(seen: &Paged) -> Option<f32> {
    let s = seen.borrow();
    let (app, _, scroller) = s.as_ref()?;
    let scroller = scroller.as_ref()?;
    let ui = app.windows().first()?.ui()?;
    Some(ui.scroll_offset(scroller.id()).y)
}

#[wasm_bindgen_test]
async fn a_reload_brings_the_entries_back_in_their_places() {
    use remedy_ui::core::Vec2;
    const NAME: &str = "Browser router reload test";
    let history = web_sys::window().must("a window").history().must("history");
    history.replace_state_with_url(&JsValue::NULL, "", Some("#/home")).must("the address");

    let seen: Paged = Rc::default();
    paged_app(NAME, seen.clone());
    assert!(until(120, || seen.borrow().is_some()).await, "the window was built");
    let (app, router) = seen.borrow().as_ref().map(|(a, r, _)| (a.clone(), r.clone())).must("the app");
    router.push("/browse");
    assert!(until(120, || mirrored_text("Page /browse").is_some()).await, "the page changed");
    let scroller = seen.borrow().as_ref().and_then(|s| s.2.clone()).must("the list");
    scroller.scroll_to(Vec2::new(0.0, 150.0));
    assert!(until(120, || shown_scroll(&seen).is_some_and(|y| (y - 150.0).abs() < 1.0)).await, "scrolled");
    router.push("/settings");
    assert!(until(120, || mirrored_text("Page /settings").is_some()).await);

    // The page is put away and loaded again: a new app in the same history entry.
    hide_page_as(false);
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
    drop((app, router, scroller));
    let again: Paged = Rc::default();
    paged_app(NAME, again.clone());
    assert!(until(120, || again.borrow().is_some()).await, "the window was built again");
    let (app, router) = again.borrow().as_ref().map(|(a, r, _)| (a.clone(), r.clone())).must("the app");
    assert_eq!(router.route_untracked().path(), "/settings");
    assert_eq!(router.position(), (2, 3), "every entry is back, this one in its place");

    // Back past the reload lands on the entry before, at its scroll position.
    history.back().must("back");
    assert!(until(120, || router.route_untracked().path() == "/browse").await, "back to /browse");
    assert_eq!(router.position(), (1, 3));
    assert_eq!(router.saved_scroll(), Vec2::new(0.0, 150.0));
    assert!(
        until(120, || shown_scroll(&again).is_some_and(|y| (y - 150.0).abs() < 1.0)).await,
        "the list is shown where it was: {:?}",
        shown_scroll(&again)
    );
    assert!(router.back());
    assert!(until(120, || router.route_untracked().path() == "/home").await);
    assert_eq!(router.position(), (0, 3));

    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
    history.replace_state_with_url(&JsValue::NULL, "", Some("#")).must("reset");
}

#[wasm_bindgen_test]
async fn corners_draw_one_by_one_and_the_app_hears_the_page_go_at_once() {
    use remedy_ui::a11y::{Role, Semantics};
    use remedy_ui::core::{Color, CornerRadii};
    use remedy_ui::layout::Length;
    let theme_bg = AppConfig::new("x").theme.colors.bg.to_rgba8();
    let left: Rc<RefCell<Vec<remedy_ui::Leaving>>> = Rc::default();
    let held: Rc<RefCell<Option<App>>> = Rc::default();
    let (l, h) = (left.clone(), held.clone());
    let run = remedy_ui::run(AppConfig::new("Corners test").cpu_only(), move |app: &App| {
        app.on_leave(move |why| l.borrow_mut().push(why));
        *h.borrow_mut() = Some(app.clone());
        app.open(WindowSpec::new("Corners"), |ui, _| {
            // Round at the top only (CSS `border-radius: 24px 24px 0 0`).
            let top = CornerRadii { top_start: 24.0, top_end: 24.0, ..CornerRadii::ZERO };
            let look =
                remedy_ui::style::StylePatch::default().background(Color::rgb8(200, 30, 30)).corners(top);
            let card = ui
                .el("card")
                .style(look)
                .layout(|l| {
                    l.width = Length::Px(120.0);
                    l.height = Length::Px(80.0);
                })
                .semantics(Semantics::new(Role::Group).label("Card"));
            let _ = ui.root().child(card);
        });
    });
    run.must("the app starts in the page");
    assert!(until(120, || query("[aria-label='Card']").is_some()).await, "the card is mirrored");
    frames(3).await;
    let canvas: web_sys::HtmlCanvasElement = document()
        .query_selector("[data-remedy-window] canvas")
        .ok()
        .flatten()
        .must("a canvas")
        .dyn_into()
        .must("a canvas");
    let ctx: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok().flatten().must("the 2D context").dyn_into().must("a 2D context");
    let card = query("[aria-label='Card']").must("the card").get_bounding_client_rect();
    let cr = canvas.get_bounding_client_rect();
    let scale = f64::from(canvas.width()) / cr.width();
    let at =
        |x: f64, y: f64| pixel(&ctx, ((x - cr.left()) * scale).floor(), ((y - cr.top()) * scale).floor());
    let red = [200, 30, 30, 255];
    assert!(close(at(card.left() + 1.0, card.top() + 1.0), theme_bg), "the top start corner is round");
    assert!(close(at(card.right() - 2.0, card.top() + 1.0), theme_bg), "so is the top end");
    assert!(close(at(card.left() + 1.0, card.bottom() - 2.0), red), "the bottom start corner is square");
    assert!(close(at(card.right() - 2.0, card.bottom() - 2.0), red), "so is the bottom end");

    // The page going: the app hears it inside the browser's `pagehide`, before it returns,
    // while a keepalive request could still go out.
    // Into the back-forward cache it is frozen; closed for good it is closing.
    hide_page_as(true);
    assert_eq!(left.borrow().last(), Some(&remedy_ui::Leaving::Frozen), "told at once, frozen");
    hide_page_as(false);
    assert_eq!(left.borrow().last(), Some(&remedy_ui::Leaving::Closing), "told at once, closing");
    let app = held.borrow_mut().take().must("the app");
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
}

#[wasm_bindgen_test]
async fn a_dashed_border_draws_dashes_and_a_maximum_length_cuts_input() {
    use remedy_ui::a11y::{Role, Semantics};
    use remedy_ui::core::Color;
    use remedy_ui::layout::Length;
    use remedy_ui::style::{BorderStyle, StylePatch};
    let theme_bg = AppConfig::new("x").theme.colors.bg.to_rgba8();
    let typed: Rc<RefCell<String>> = Rc::default();
    let held: Rc<RefCell<Option<App>>> = Rc::default();
    let (t, h) = (typed.clone(), held.clone());
    let run = remedy_ui::run(AppConfig::new("Limits test").cpu_only(), move |app: &App| {
        *h.borrow_mut() = Some(app.clone());
        let t = t.clone();
        app.open(WindowSpec::new("Limits"), move |ui, _| {
            let look = StylePatch::default()
                .border(Color::rgb8(255, 60, 60))
                .border_width(2.0)
                .border_style(BorderStyle::Dashed);
            let card = ui
                .el("dashed_box")
                .style(look)
                .layout(|l| {
                    l.width = Length::Px(120.0);
                    l.height = Length::Px(40.0);
                })
                .semantics(Semantics::new(Role::Group).label("Dashed"));
            let field = w::text_input(ui, "Short").max_length(4);
            let f2 = field.clone();
            let field = field.on_change(move || *t.borrow_mut() = f2.value());
            let col = w::column(ui)
                .gap(12.0)
                .padding(remedy_ui::core::Insets { top: 16.0, end: 16.0, bottom: 16.0, start: 16.0 })
                .child(card)
                .child(field.el().height(32.0));
            let _ = ui.root().child(col);
        });
    });
    run.must("the app starts in the page");
    assert!(until(120, || query("[aria-label='Dashed']").is_some()).await, "the card is mirrored");
    frames(3).await;
    let canvas: web_sys::HtmlCanvasElement = document()
        .query_selector("[data-remedy-window] canvas")
        .ok()
        .flatten()
        .must("a canvas")
        .dyn_into()
        .must("a canvas");
    let ctx: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok().flatten().must("the 2D context").dyn_into().must("a 2D context");
    let card = query("[aria-label='Dashed']").must("the card").get_bounding_client_rect();
    let cr = canvas.get_bounding_client_rect();
    let scale = f64::from(canvas.width()) / cr.width();
    let at =
        |x: f64, y: f64| pixel(&ctx, ((x - cr.left()) * scale).floor(), ((y - cr.top()) * scale).floor());
    // Dashes 6 long with gaps as long, from the top start corner.
    let dash = at(card.left() + 3.0, card.top() + 1.0);
    assert!(dash[0] > 200 && dash[1] < 100, "a dash: {dash:?}");
    assert!(close(at(card.left() + 9.0, card.top() + 1.0), theme_bg), "a gap");
    assert!(close(at(card.left() + 60.0, card.top() + 20.0), theme_bg), "inside, nothing");

    // The field holds four characters: an input method's text and a paste are cut to fit.
    let target: web_sys::EventTarget = canvas.clone().into();
    let (fx, fy) = center(&query("[aria-label='Short']").must("the field"));
    pointer(&target, "pointerdown", fx, fy, 1);
    pointer(&target, "pointerup", fx, fy, 0);
    assert!(until(30, || query("input[data-remedy-text]:focus").is_some()).await, "the field edits");
    let text: web_sys::EventTarget = query("input[data-remedy-text]").must("the hidden field").into();
    key(&text, "keydown", "a", "KeyA", false);
    key(&text, "keyup", "a", "KeyA", false);
    composition(&text, "compositionstart", "");
    composition(&text, "compositionupdate", "にほんごです");
    composition(&text, "compositionend", "日本語です");
    assert!(until(30, || *typed.borrow() == "a日本語").await, "the commit was cut: {:?}", typed.borrow());
    key(&text, "keydown", "Backspace", "Backspace", false);
    key(&text, "keyup", "Backspace", "Backspace", false);
    let readable = synthetic_clipboard_readable();
    key(&text, "keydown", "v", "KeyV", true);
    clipboard(&text, "paste", Some("xyz".into()));
    key(&text, "keyup", "v", "KeyV", true);
    let want = if readable { "a日本x" } else { "a日本" };
    assert!(until(30, || *typed.borrow() == want).await, "the paste was cut: {:?}", typed.borrow());
    let app = held.borrow_mut().take().must("the app");
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
}

#[wasm_bindgen_test]
async fn link_clicks_open_tabs_and_mouse_buttons_move_once() {
    let window = web_sys::window().must("a window");
    let history = window.history().must("history");
    let location = window.location();
    history.replace_state_with_url(&JsValue::NULL, "", Some("#/home")).must("the address");
    let seen: Paged = Rc::default();
    paged_app("Browser router click test", seen.clone());
    assert!(until(120, || seen.borrow().is_some()).await, "the window was built");
    let (app, router) = seen.borrow().as_ref().map(|(a, r, _)| (a.clone(), r.clone())).must("the app");
    assert!(until(120, || query("a[href='#/settings']").is_some()).await, "the link in the mirror");
    let link = query("a[href='#/settings']").must("the link");
    let canvas: web_sys::EventTarget = document()
        .query_selector("[data-remedy-window] canvas")
        .ok()
        .flatten()
        .must("the window's canvas")
        .into();
    let (lx, ly) = center(&link);
    record_opens();

    // Ctrl-click on the drawn link: a new tab at the route, this page staying where it was.
    let _ = mouse(&canvas, "pointermove", lx, ly, -1, 0, true);
    let _ = mouse(&canvas, "pointerdown", lx, ly, 0, 1, true);
    let _ = mouse(&canvas, "pointerup", lx, ly, 0, 0, true);
    assert!(
        until(120, || opened_hashes() == "#/settings").await,
        "a tab opened at the route: {}",
        opened_hashes()
    );
    close_opened();
    assert_eq!(router.route_untracked().path(), "/home");

    // A middle click on it: another.
    let _ = mouse(&canvas, "pointerdown", lx, ly, 1, 4, false);
    let _ = mouse(&canvas, "pointerup", lx, ly, 1, 0, false);
    let _ = mouse(&canvas, "auxclick", lx, ly, 1, 0, false);
    assert!(until(120, || opened_hashes() == "#/settings").await, "middle click: {}", opened_hashes());
    close_opened();
    assert_eq!(router.route_untracked().path(), "/home");

    // The mirror's <a> (a screen reader's, or the person's on it): Ctrl-click and middle click.
    let _ = mouse(&link, "click", lx, ly, 0, 0, true);
    assert!(until(120, || opened_hashes() == "#/settings").await, "mirror Ctrl-click: {}", opened_hashes());
    close_opened();
    let _ = mouse(&link, "auxclick", lx, ly, 1, 0, false);
    assert!(until(120, || opened_hashes() == "#/settings").await, "mirror middle click: {}", opened_hashes());
    close_opened();
    assert_eq!(router.route_untracked().path(), "/home");
    assert_eq!(location.hash().must("hash"), "#/home");

    // The mouse's back and forward buttons move exactly one entry, and the browser's own
    // move on their release is cancelled.
    router.push("/browse");
    router.push("/server");
    assert_eq!(router.position(), (2, 3));
    let r = web_sys::Element::from(JsValue::from(canvas.clone())).get_bounding_client_rect();
    let (bx, by) = (r.left() + 8.0, r.bottom() - 8.0);
    let _ = mouse(&canvas, "pointerdown", bx, by, 3, 8, false);
    let _ = mouse(&canvas, "pointerup", bx, by, 3, 0, false);
    assert!(!mouse(&canvas, "mouseup", bx, by, 3, 0, false), "the browser's back is cancelled");
    assert!(!mouse(&canvas, "auxclick", bx, by, 3, 0, false));
    assert!(until(120, || location.hash().ok().as_deref() == Some("#/browse")).await, "back once");
    wait_ms(300).await;
    assert_eq!(location.hash().must("hash"), "#/browse", "not twice");
    assert_eq!(router.position(), (1, 3));
    let _ = mouse(&canvas, "pointerdown", bx, by, 4, 16, false);
    let _ = mouse(&canvas, "pointerup", bx, by, 4, 0, false);
    assert!(!mouse(&canvas, "mouseup", bx, by, 4, 0, false), "the browser's forward is cancelled");
    assert!(until(120, || location.hash().ok().as_deref() == Some("#/server")).await, "forward once");
    wait_ms(300).await;
    assert_eq!(router.position(), (2, 3));

    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
    history.replace_state_with_url(&JsValue::NULL, "", Some("#")).must("reset");
}

#[wasm_bindgen(inline_js = r#"
export function plugPad() {
  // A standard-mapping pad the page cannot tell from a real one: Gamepad and GamepadButton
  // objects whose state the test sets.
  const own = (o, k, v) => Object.defineProperty(o, k, { configurable: true, writable: true, value: v });
  const buttons = Array.from({ length: 17 }, () => own(Object.create(GamepadButton.prototype), 'pressed', false));
  const pad = Object.create(Gamepad.prototype);
  own(pad, 'connected', true);
  own(pad, 'mapping', 'standard');
  own(pad, 'buttons', buttons);
  own(pad, 'axes', [0, 0, 0, 0]);
  window.__pad = pad;
  Object.defineProperty(navigator, 'getGamepads', { configurable: true, writable: true,
    value: () => [window.__pad] });
  window.dispatchEvent(new Event('gamepadconnected'));
}
export function padButton(i, pressed) { window.__pad.buttons[i].pressed = pressed; }
export function padStick(x, y) { window.__pad.axes = [x, y, 0, 0]; }
export function unplugPad() { window.__pad = null; delete navigator.getGamepads; }
"#)]
extern "C" {
    /// Plugs in a stand-in standard-mapping gamepad.
    #[wasm_bindgen(js_name = plugPad)]
    fn plug_pad();
    #[wasm_bindgen(js_name = padButton)]
    fn pad_button(index: u32, pressed: bool);
    #[wasm_bindgen(js_name = padStick)]
    fn pad_stick(x: f64, y: f64);
    #[wasm_bindgen(js_name = unplugPad)]
    fn unplug_pad();
}

#[derive(Default)]
struct PadSeen {
    tiles: RefCell<Vec<remedy_ui::El>>,
    pressed: RefCell<Vec<usize>>,
    app: RefCell<Option<App>>,
}

impl PadSeen {
    fn focused(&self) -> Option<usize> {
        self.tiles.borrow().iter().position(remedy_ui::El::is_focused)
    }
}

#[wasm_bindgen_test]
async fn a_held_gamepad_direction_repeats_and_a_presses_once() {
    let seen = Rc::new(PadSeen::default());
    let s = seen.clone();
    let run = remedy_ui::run(AppConfig::new("Gamepad test").cpu_only(), move |app: &App| {
        *s.app.borrow_mut() = Some(app.clone());
        app.set_gamepad_tuning(
            remedy_ui::input::GamepadTuning::default()
                .with_repeat_delay(std::time::Duration::from_millis(150))
                .with_repeat_interval(std::time::Duration::from_millis(40)),
        );
        let s2 = s.clone();
        app.open(WindowSpec::new("Gamepad"), move |ui, _| {
            ui.set_spatial_navigation(true);
            let mut col = w::column(ui).gap(4.0);
            for i in 0..6 {
                let s3 = s2.clone();
                let tile =
                    w::button(ui, format!("Tile {i}"), move || s3.pressed.borrow_mut().push(i)).height(32.0);
                s2.tiles.borrow_mut().push(tile.clone());
                col = col.child(tile);
            }
            let _ = ui.root().child(col);
        });
    });
    run.must("the app starts in the page");
    assert!(until(120, || seen.tiles.borrow().len() == 6).await, "the window was built");
    frames(3).await;

    plug_pad();
    // Held down: the first press starts at the first tile, then repeats walk to the last.
    pad_button(13, true);
    assert!(until(30, || seen.focused() == Some(0)).await, "the first press: {:?}", seen.focused());
    assert!(until(120, || seen.focused() == Some(5)).await, "held, it repeats: {:?}", seen.focused());
    pad_button(13, false);
    frames(2).await;
    // The stick up, past its dead zone, and back: one step, and no repeat once let go.
    pad_stick(0.0, -0.9);
    assert!(until(30, || seen.focused() == Some(4)).await, "the stick moves focus: {:?}", seen.focused());
    pad_stick(0.0, 0.1);
    wait_ms(400).await;
    assert_eq!(seen.focused(), Some(4), "let go, it stops");
    // A held A presses the focused tile once.
    pad_button(0, true);
    assert!(until(30, || !seen.pressed.borrow().is_empty()).await, "A pressed the tile");
    wait_ms(400).await;
    pad_button(0, false);
    frames(2).await;
    assert_eq!(*seen.pressed.borrow(), [4], "pressed once, not repeated");
    unplug_pad();

    let app = seen.app.borrow().clone().must("the app");
    app.quit();
    assert!(until(30, || !remedy_ui::web::is_running()).await);
}

#[wasm_bindgen(inline_js = r#"
export function watchGaps() {
  window.__worstGap = 0;
  let last = performance.now();
  window.__gapTimer = setInterval(() => {
    const now = performance.now();
    window.__worstGap = Math.max(window.__worstGap, now - last);
    last = now;
  }, 20);
}
export function worstGap() {
  clearInterval(window.__gapTimer);
  return window.__worstGap;
}
"#)]
extern "C" {
    /// Starts noting the longest time the page's thread went without running a 20 ms timer.
    #[wasm_bindgen(js_name = watchGaps)]
    fn watch_gaps();
    /// The longest gap since `watch_gaps`, in milliseconds.
    #[wasm_bindgen(js_name = worstGap)]
    fn worst_gap() -> f64;
}
