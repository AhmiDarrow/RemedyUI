//! The media element's pictures drawn by the GPU renderer in a real browser (run with the web
//! bridge's `tools/web-test.sh`): the element itself is uploaded as a texture (WebGL2's
//! `texSubImage2D`, WebGPU's `copyExternalImageToTexture` or import), never read back on the
//! CPU, in the right colors through seeks. Built with `REMEDY_MSE_INSECURE=1` the page is at
//! the machine's LAN address: not a secure context, so no `navigator.gpu` and no WebCodecs,
//! and the web bridge draws with WebGL2.
//!
//! Its own test binary: which renderer takes pictures on the GPU is said for the whole page.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::{App, AppConfig, RendererKind, WindowSpec};
use remedy_ui_media_playback::{MediaSource, PlaybackState, Player, PlayerOptions, WebBackend};
use remedy_ui_engine::paint::GpuAdapter;
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
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function secure() { return !!window.isSecureContext; }
export function hasGpu() { return !!navigator.gpu; }
// Counts each way the element reaches a texture, and reads back on the CPU.
export function spy() {
  if (window.__up) return;
  const c = { texImage: 0, gpuCopied: 0, imported: 0, readBack: 0 };
  window.__up = c;
  const video = (v) => v instanceof HTMLVideoElement;
  const wrap = (proto, name, key, test) => {
    if (!proto || !proto[name]) return;
    const f = proto[name];
    proto[name] = function (...a) { const r = f.apply(this, a); if (test(a)) c[key]++; return r; };
  };
  wrap(WebGL2RenderingContext.prototype, 'texSubImage2D', 'texImage', (a) => video(a[a.length - 1]));
  wrap(window.GPUQueue && GPUQueue.prototype, 'copyExternalImageToTexture', 'gpuCopied', (a) => a[0] && video(a[0].source));
  wrap(window.GPUDevice && GPUDevice.prototype, 'importExternalTexture', 'imported', (a) => a[0] && video(a[0].source));
  // The CPU's way: the element drawn into a 2D canvas to be read back.
  wrap(window.OffscreenCanvasRenderingContext2D && OffscreenCanvasRenderingContext2D.prototype, 'drawImage', 'readBack', (a) => video(a[0]));
  wrap(CanvasRenderingContext2D.prototype, 'drawImage', 'readBack', (a) => video(a[0]));
  const get = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (kind, opts) {
    if (kind === 'webgl2') opts = { ...(opts || {}), preserveDrawingBuffer: true };
    return get.call(this, kind, opts);
  };
  const d = document.createElement('div'); d.id = 'remedy'; d.style.cssText = 'width:360px;height:280px';
  document.body.appendChild(d);
}
export function count(k) { return window.__up[k]; }
export function context(canvas) {
  for (const kind of ['webgpu', 'webgl2']) { try { if (canvas.getContext(kind)) return kind; } catch (e) {} }
  return 'none';
}
// A pixel of the largest element labelled `label`, a fifth of the way across at mid-height,
// read from the WebGL2 canvas's kept drawing buffer.
export function pixelUnder(canvas, label) {
  return new Promise((resolve) => requestAnimationFrame(() => {
    let best = null, area = 0;
    for (const el of document.querySelectorAll(`[aria-label="${label}"]`)) {
      const r = el.getBoundingClientRect();
      if (r.width * r.height > area) { area = r.width * r.height; best = r; }
    }
    if (!best) { resolve([]); return; }
    const c = canvas.getBoundingClientRect();
    const x = Math.floor((best.left + best.width / 5 - c.left) * canvas.width / c.width);
    const y = Math.floor((best.top + best.height / 2 - c.top) * canvas.height / c.height);
    let gl = null;
    try { gl = canvas.getContext('webgl2'); } catch (e) {}
    if (!gl) { resolve([0, 0, 0, 0]); return; }
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
    const px = new Uint8Array(4);
    gl.readPixels(x, canvas.height - 1 - y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    resolve(Array.from(px));
  }));
}
"#)]
extern "C" {
    fn sleep(ms: u32) -> js_sys::Promise;
    fn secure() -> bool;
    #[wasm_bindgen(js_name = hasGpu)]
    fn has_gpu() -> bool;
    fn spy();
    fn count(what: &str) -> u32;
    fn context(canvas: &web_sys::HtmlCanvasElement) -> String;
    #[wasm_bindgen(js_name = pixelUnder)]
    fn pixel_under(canvas: &web_sys::HtmlCanvasElement, label: &str) -> js_sys::Promise;
}

async fn wait_ms(ms: u32) {
    let _ = JsFuture::from(sleep(ms)).await;
}

const INSECURE: bool = option_env!("REMEDY_MSE_INSECURE").is_some();
const HLS_DIR: Option<&str> = option_env!("REMEDY_HLS_URL");

fn hue(px: &[u8]) -> char {
    match px {
        [r, g, b, ..] if r > g && r > b => 'r',
        [_, g, b, ..] if g > b => 'g',
        _ => 'b',
    }
}

#[wasm_bindgen_test]
async fn the_gpu_renderer_uploads_the_element_itself() {
    let Some(dir) = HLS_DIR else { return };
    if INSECURE {
        assert!(!secure() && !has_gpu(), "the LAN page is not secure: no navigator.gpu");
    }
    spy();
    let backend = if INSECURE { WebBackend::Auto } else { WebBackend::MediaSource };
    let source = MediaSource::url(format!("{dir}/aac/index.m3u8"));
    let app: Rc<RefCell<Option<App>>> = Rc::default();
    let slot: Rc<RefCell<Option<Player>>> = Rc::default();
    let (a2, s2) = (app.clone(), slot.clone());
    remedy_ui::run(AppConfig::new("Element GPU test").gpu_first(), move |a: &App| {
        *a2.borrow_mut() = Some(a.clone());
        let s3 = s2.clone();
        a.open(WindowSpec::new("Element GPU test"), move |ui, _| {
            let p = Player::open_platform(ui, source, PlayerOptions::default().web_backend(backend));
            let view = remedy_ui_media_playback::player(ui, &p, "GPU element").width(352.0).height(260.0);
            let _ = ui.root().child(view);
            *s3.borrow_mut() = Some(p);
        });
    })
    .must("the app starts");
    let renderer = || app.borrow().as_ref().and_then(|a| a.windows().first().and_then(|w| w.renderer()));
    // The window draws on the CPU until the browser has started its GL, which can take
    // minutes in headless Firefox on a loaded machine: wait for it as long as it takes.
    for _ in 0..36_000 {
        if renderer() == Some(RendererKind::Gpu) {
            break;
        }
        wait_ms(10).await;
    }
    assert_eq!(renderer(), Some(RendererKind::Gpu), "the window draws on the GPU");
    assert_eq!(remedy_ui_engine::paint::gpu_video_adapter(), Some(GpuAdapter::Browser));
    let canvas: web_sys::HtmlCanvasElement = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("canvas[data-raw-handle]").ok().flatten())
        .must("the GPU canvas")
        .dyn_into()
        .must("a canvas");
    let kind = context(&canvas);
    if INSECURE {
        assert_eq!(kind, "webgl2", "without navigator.gpu the bridge draws with WebGL2");
    }
    let p = slot.borrow().clone().must("the player");
    for _ in 0..1500 {
        if p.state() == PlaybackState::Paused {
            break;
        }
        wait_ms(10).await;
    }
    assert_eq!(p.state(), PlaybackState::Paused, "opened: {:?}", p.error());
    let read_back = count("readBack");
    for (ms, want) in [(500u64, 'r'), (1500, 'g'), (2500, 'b')] {
        let before = p.stats().frames_shown;
        p.seek(Duration::from_millis(ms));
        let mut seen = Vec::new();
        for _ in 0..600 {
            if p.stats().frames_shown > before {
                wait_ms(100).await;
                let px = JsFuture::from(pixel_under(&canvas, "GPU element")).await.must("a pixel");
                seen = js_sys::Array::from(&px).iter().filter_map(|v| v.as_f64()).map(|v| v as u8).collect();
                break;
            }
            wait_ms(10).await;
        }
        web_sys::console::log_1(&format!("{kind}: at {ms} ms the screen shows {seen:?}").into());
        if kind == "webgl2" {
            assert_eq!(hue(&seen), want, "the picture at {ms} ms on the canvas: {seen:?}");
        }
    }
    p.play();
    for _ in 0..1500 {
        if p.state() == PlaybackState::Ended {
            break;
        }
        wait_ms(10).await;
    }
    assert_eq!(p.state(), PlaybackState::Ended);
    let (tex, copied, imported) = (count("texImage"), count("gpuCopied"), count("imported"));
    web_sys::console::log_1(
        &format!(
            "{kind}: {:?}; texSubImage2D {tex}, copyExternalImageToTexture {copied}, import {imported}",
            p.stats()
        )
        .into(),
    );
    assert!(tex + copied + imported > 0, "the element went to a texture");
    assert_eq!(count("readBack") - read_back, 0, "no picture was read back on the CPU");
    if let Some(a) = app.borrow().as_ref() {
        a.quit();
    }
}
