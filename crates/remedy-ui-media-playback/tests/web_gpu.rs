//! Pictures on the GPU in a real browser (run with the web bridge's `tools/web-test.sh`): with
//! a GPU renderer in the page, the browser's decoded `VideoFrame`s are handed over as they are
//! and never copied out (`copyTo` is never called), are closed once their pictures go, read
//! back in the right colors for a renderer that cannot import them, and are drawn by the GPU
//! renderer (WebGPU or WebGL2) in the right colors at their times, through seeks.
//!
//! Its own test binary: which renderer takes pictures on the GPU is said for the whole page.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use remedy_ui::{App, AppConfig, RendererKind, WindowSpec};
use remedy_ui_engine::paint::{GpuAdapter, GpuTexture};
use remedy_ui_media_playback::{
    Decoder, MediaHost, MediaSource, Picture, PixelFormat, PlaybackState, Player, PlayerOptions, Sample,
    WebBackend, platform_host_with,
};
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

const CLIP: &[u8] = include_bytes!("fixtures/clip.mp4");

#[wasm_bindgen(inline_js = r#"
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function blobUrl(bytes) { return URL.createObjectURL(new Blob([bytes], { type: 'video/mp4' })); }
// Counts, from now on, every VideoFrame.copyTo and close in the page, and each way a frame
// reaches a texture: imported (WebGPU), copied on the GPU (WebGPU: the frame, or a canvas it
// was drawn into), given to texSubImage2D (WebGL2), or read back from a 2D canvas (the CPU's
// way). Only calls that succeed are counted.
export function spyOnFrames() {
  if (window.__frames) return;
  const counts = { copyTo: 0, close: 0, imported: 0, gpuCopied: 0, canvasCopied: 0, texImage: 0, readBack: 0, gpuErrors: 0 };
  window.__frames = counts;
  // Every WebGPU device made from now on reports its uncaught errors here too (wgpu's own
  // handler keeps them to itself).
  if (window.GPUAdapter) {
    const request = GPUAdapter.prototype.requestDevice;
    GPUAdapter.prototype.requestDevice = async function (...a) {
      const device = await request.apply(this, a);
      const i = this.info || {};
      console.log(`WebGPU adapter: ${i.vendor} ${i.architecture} ${i.device} ${i.description}`);
      device.addEventListener('uncapturederror', (e) => { counts.gpuErrors++; console.log(`WebGPU error: ${e.error.message}`); });
      return device;
    };
  }
  const wrap = (proto, name, key, test) => {
    if (!proto || !proto[name]) return;
    const f = proto[name];
    proto[name] = function (...a) { const r = f.apply(this, a); if (!test || test(a)) counts[key]++; return r; };
  };
  const frame = (v) => v instanceof VideoFrame;
  wrap(VideoFrame.prototype, 'copyTo', 'copyTo');
  wrap(VideoFrame.prototype, 'close', 'close');
  wrap(window.GPUDevice && GPUDevice.prototype, 'importExternalTexture', 'imported');
  const canvas = (v) => window.OffscreenCanvas && v instanceof OffscreenCanvas;
  wrap(window.GPUQueue && GPUQueue.prototype, 'copyExternalImageToTexture', 'gpuCopied', (a) => a[0] && (frame(a[0].source) || canvas(a[0].source)));
  wrap(window.GPUQueue && GPUQueue.prototype, 'copyExternalImageToTexture', 'canvasCopied', (a) => a[0] && canvas(a[0].source));
  wrap(WebGL2RenderingContext.prototype, 'texSubImage2D', 'texImage', (a) => frame(a[a.length - 1]));
  wrap(window.OffscreenCanvasRenderingContext2D && OffscreenCanvasRenderingContext2D.prototype, 'getImageData', 'readBack');
}
// Test-only: a WebGPU device that cannot import frames, so the renderer copies them instead
// (`copyExternalImageToTexture`), as on a browser without external textures.
export function hideImport() {
  if (window.GPUDevice) delete GPUDevice.prototype.importExternalTexture;
}
export function probe() {
  const c = document.createElement('canvas');
  let gl = null; try { gl = c.getContext('webgl2'); } catch (e) { gl = String(e); }
  return `navigator.gpu=${!!navigator.gpu} webgl2=${gl ? 'yes' : 'no'} OffscreenCanvas=${typeof OffscreenCanvas}`;
}
// GPU canvases keep what was last drawn, so a paused player's picture reads back after the
// frame it was drawn in (the page clears it otherwise): WebGL2's with preserveDrawingBuffer.
export function preserveWebgl() {
  const get = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (kind, opts) {
    if (kind === 'webgl2') opts = { ...(opts || {}), preserveDrawingBuffer: true };
    return get.call(this, kind, opts);
  };
  // A WebGPU canvas keeps nothing: each frame drawn on one is copied once its task is done.
  if (window.GPUCanvasContext) {
    const current = GPUCanvasContext.prototype.getCurrentTexture;
    GPUCanvasContext.prototype.getCurrentTexture = function (...a) {
      const canvas = this.canvas;
      queueMicrotask(() => {
        const snap = canvas.__snapshot || (canvas.__snapshot = document.createElement('canvas'));
        snap.width = canvas.width; snap.height = canvas.height;
        snap.getContext('2d').drawImage(canvas, 0, 0);
      });
      return current.apply(this, a);
    };
  }
}
export function frameCount(what) { return window.__frames[what]; }
export function context(canvas) {
  for (const kind of ['webgpu', 'webgl2']) {
    try { if (canvas.getContext(kind)) return kind; } catch (e) {}
  }
  return 'none';
}
// A pixel of the largest element labelled `label` on `canvas`, at mid-height. A WebGL2
// canvas is read with its own context's readPixels (drawing it into another canvas stalls a
// software GPU), from the drawing buffer it keeps; a WebGPU one from the copy of the last
// frame drawn on it.
export function pixelUnder(canvas, label) {
  return new Promise((resolve) => requestAnimationFrame(() => {
    let best = null, area = 0;
    for (const el of document.querySelectorAll(`[aria-label="${label}"]`)) {
      const r = el.getBoundingClientRect();
      if (r.width * r.height > area) { area = r.width * r.height; best = r; }
    }
    if (!best) { resolve([]); return; }
    const c = canvas.getBoundingClientRect();
    // A fifth of the way across: clear of anything the player shows in its middle.
    const x = Math.floor((best.left + best.width / 5 - c.left) * canvas.width / c.width);
    const y = Math.floor((best.top + best.height / 2 - c.top) * canvas.height / c.height);
    let gl = null;
    try { gl = canvas.getContext('webgl2'); } catch (e) {}
    if (gl) {
      const bound = gl.getParameter(gl.READ_FRAMEBUFFER_BINDING);
      const buffer = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
      const px = new Uint8Array(4);
      gl.readPixels(x, canvas.height - 1 - y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, bound);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
      resolve(Array.from(px));
      return;
    }
    const snap = canvas.__snapshot;
    if (!snap) { console.log('no snapshot of the WebGPU canvas'); resolve([0, 0, 0, 0]); return; }
    resolve(Array.from(snap.getContext('2d').getImageData(x, y, 1, 1).data));
  }));
}
"#)]
extern "C" {
    fn sleep(ms: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = blobUrl)]
    fn blob_url(bytes: &[u8]) -> String;
    #[wasm_bindgen(js_name = spyOnFrames)]
    fn spy_on_frames();
    #[wasm_bindgen(js_name = frameCount)]
    fn frame_count(what: &str) -> u32;
    fn context(canvas: &web_sys::HtmlCanvasElement) -> String;
    fn probe() -> String;
    #[wasm_bindgen(js_name = preserveWebgl)]
    fn preserve_webgl();
    #[wasm_bindgen(js_name = hideImport)]
    fn hide_import();
    #[wasm_bindgen(js_name = pixelUnder)]
    fn pixel_under(canvas: &web_sys::HtmlCanvasElement, label: &str) -> js_sys::Promise;
}

async fn wait_ms(ms: u32) {
    let _ = JsFuture::from(sleep(ms)).await;
}

fn host() -> std::sync::Arc<dyn MediaHost> {
    platform_host_with(WebBackend::WebCodecs).must("the browser's media host")
}

/// Which of red, green and blue an RGB color is.
fn hue_of(r: f32, g: f32, b: f32) -> char {
    if r > g && r > b {
        'r'
    } else if g > b {
        'g'
    } else {
        'b'
    }
}

/// Which of red, green and blue a picture on the GPU is at its middle, read back.
fn hue(p: &Picture) -> char {
    let gpu = p.gpu.as_ref().must("the picture is on the GPU");
    let px = gpu.0.read_back().must("the picture reads back");
    assert_eq!(px.format, PixelFormat::Rgba);
    let (x, y) = ((p.width / 2) as usize, (p.height / 2) as usize);
    let plane = px.planes.first().copied().unwrap_or_default();
    let o = plane.offset + y * plane.stride + x * 4;
    let at = |i: usize| f32::from(px.data.get(o + i).copied().unwrap_or(0));
    hue_of(at(0), at(1), at(2))
}

async fn next(dec: &mut Box<dyn Decoder>) -> Option<Sample> {
    for _ in 0..4000 {
        match dec.poll_read(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => return r.must("decoding"),
            Poll::Pending => wait_ms(2).await,
        }
    }
    unreachable!("the decoder answered within 8 s")
}

async fn picture_at(dec: &mut Box<dyn Decoder>, t: Duration) -> (Duration, Picture) {
    loop {
        match next(dec).await {
            Some(Sample::Video { at, picture, .. }) if at >= t => return (at, picture),
            Some(_) => {}
            None => unreachable!("a picture at {t:?} before the end"),
        }
    }
}

/// The decoder alone, then a player drawn by the GPU renderer, one after the other: which
/// renderer takes pictures on the GPU is said for the whole page.
#[wasm_bindgen_test]
async fn frames_stay_on_the_gpu_from_the_decoder_to_the_screen() {
    web_sys::console::log_1(&probe().into());
    decoded_frames_are_handed_over_without_a_copy().await;
    the_gpu_renderer_shows_frames_imported_where_they_are().await;
}

/// The decoder alone, as a page with a GPU renderer has it: pictures stay frames on the GPU.
async fn decoded_frames_are_handed_over_without_a_copy() {
    spy_on_frames();
    remedy_ui_engine::paint::set_gpu_video_adapter(Some(GpuAdapter::Browser));
    let host = host();
    let mut dec = host.open(&MediaSource::url(blob_url(CLIP))).must("opening the clip");
    let mut ready = false;
    for _ in 0..2000 {
        if let Poll::Ready(r) = dec.poll_ready(&mut Context::from_waker(std::task::Waker::noop())) {
            r.must("the decoders take the clip");
            ready = true;
            break;
        }
        wait_ms(2).await;
    }
    assert!(ready, "the clip opened");
    let (copies, closes) = (frame_count("copyTo"), frame_count("close"));

    let (at, red) = picture_at(&mut dec, Duration::ZERO).await;
    assert!(at < Duration::from_millis(50), "{at:?}");
    assert!(red.data.is_empty(), "a picture on the GPU has no bytes");
    let texture = red.gpu.as_ref().must("on the GPU").0.texture();
    assert!(
        matches!(texture, GpuTexture::WebVideoFrame { width: 176, height: 100, .. }),
        "a VideoFrame as shown: {texture:?}"
    );
    assert_eq!((red.width, red.height), (176, 100));
    assert_eq!(hue(&red), 'r', "the first second is red");
    dec.seek(Duration::from_millis(1500)).must("seeking to 1.5 s");
    let (at, green) = picture_at(&mut dec, Duration::from_millis(1500)).await;
    assert!(at < Duration::from_millis(1600), "{at:?}");
    assert_eq!(hue(&green), 'g', "1.5 s is green");
    dec.seek(Duration::from_millis(2500)).must("seeking to 2.5 s");
    let (_, blue) = picture_at(&mut dec, Duration::from_millis(2500)).await;
    assert_eq!(hue(&blue), 'b', "2.5 s is blue");
    let mut shown = 3;
    while let Some(s) = next(&mut dec).await {
        if let Sample::Video { picture, .. } = s {
            assert!(picture.gpu.is_some(), "every picture is on the GPU");
            shown += 1;
        }
    }
    assert_eq!(frame_count("copyTo") - copies, 0, "no picture was copied out");
    drop((red, green, blue));
    drop(dec);
    // Every frame handed over is closed once its picture is gone (the decoder's own
    // pictures from before the seeks included).
    let closed = frame_count("close") - closes;
    web_sys::console::log_1(&format!("pictures {shown}, frames closed {closed}").into());
    assert!(closed >= shown, "{closed} frames closed for {shown} pictures kept");
    remedy_ui_engine::paint::set_gpu_video_adapter(None);
}

fn renderer(app: &Rc<RefCell<Option<App>>>) -> Option<RendererKind> {
    app.borrow().as_ref().and_then(|a| a.windows().first().and_then(|w| w.renderer()))
}

/// The hue under the player's picture on the GPU canvas.
async fn hue_on_screen(canvas: &web_sys::HtmlCanvasElement) -> (char, Vec<u8>) {
    let px = JsFuture::from(pixel_under(canvas, "GPU clip")).await.must("a pixel");
    let px: Vec<u8> = js_sys::Array::from(&px).iter().filter_map(|v| v.as_f64()).map(|v| v as u8).collect();
    let Ok([r, g, b, _]) = <[u8; 4]>::try_from(px.as_slice()) else {
        unreachable!("the picture is on the page")
    };
    (hue_of(f32::from(r), f32::from(g), f32::from(b)), px)
}

/// A player in a page drawn by the GPU renderer: the pictures shown are the frames
/// themselves, imported on the GPU, in the right colors through seeks.
async fn the_gpu_renderer_shows_frames_imported_where_they_are() {
    spy_on_frames();
    preserve_webgl();
    // Built with `REMEDY_WEB_GPU_NO_IMPORT=1`, the page's WebGPU cannot import frames, so the
    // copy that browsers without external textures get is checked in any browser.
    let no_import = option_env!("REMEDY_WEB_GPU_NO_IMPORT").is_some();
    if no_import {
        hide_import();
    }
    let url = blob_url(CLIP);
    let app: Rc<RefCell<Option<App>>> = Rc::default();
    let slot: Rc<RefCell<Option<Player>>> = Rc::default();
    let (a2, s2) = (app.clone(), slot.clone());
    remedy_ui::run(AppConfig::new("GPU playback test").gpu_first(), move |a: &App| {
        *a2.borrow_mut() = Some(a.clone());
        let (s3, url) = (s2.clone(), url.clone());
        a.open(WindowSpec::new("GPU playback test"), move |ui, _| {
            let p = Player::open(
                ui,
                platform_host_with(WebBackend::WebCodecs).must("host"),
                MediaSource::url(url),
                PlayerOptions::default(),
            );
            let view = remedy_ui_media_playback::player(ui, &p, "GPU clip").width(352.0).height(260.0);
            let _ = ui.root().child(view);
            *s3.borrow_mut() = Some(p);
        });
    })
    .must("the app starts");
    let mut gpu = false;
    // The window draws on the CPU until the browser has started its GL, which can take
    // minutes in headless Firefox on a loaded machine: wait for it as long as it takes.
    for _ in 0..36_000 {
        if renderer(&app) == Some(RendererKind::Gpu) {
            gpu = true;
            break;
        }
        wait_ms(10).await;
    }
    assert!(gpu, "the window draws on the GPU");
    let canvas: web_sys::HtmlCanvasElement = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("canvas[data-raw-handle]").ok().flatten())
        .must("the GPU canvas")
        .dyn_into()
        .must("a canvas");
    let kind = context(&canvas);
    assert!(
        remedy_ui_engine::paint::gpu_video_adapter() == Some(GpuAdapter::Browser),
        "the {kind} renderer takes pictures on the GPU"
    );
    let p = slot.borrow().clone().must("the player");
    let count = |what: &str| frame_count(what);
    let (copies, imported, gpu_copied, tex_image, read_back) =
        (count("copyTo"), count("imported"), count("gpuCopied"), count("texImage"), count("readBack"));
    // A seek while paused shows the picture there; each color is checked on screen.
    for (ms, want) in [(500u64, 'r'), (1500, 'g'), (2500, 'b'), (700, 'r')] {
        for _ in 0..600 {
            if matches!(p.state(), PlaybackState::Paused) {
                break;
            }
            wait_ms(10).await;
        }
        let before = p.stats().frames_shown;
        p.seek(Duration::from_millis(ms));
        let mut seen = ('?', Vec::new());
        for _ in 0..300 {
            // Read once the picture there has been shown (each read waits on the GPU).
            if (p.position().as_secs_f64() - ms as f64 / 1000.0).abs() < 0.001
                && p.stats().frames_shown > before
            {
                wait_ms(50).await;
                seen = hue_on_screen(&canvas).await;
                break;
            }
            wait_ms(10).await;
        }
        web_sys::console::log_1(
            &format!(
                "{kind}: at {ms} ms the screen shows {:?} (so far imported {}, GPU-copied {}, texSubImage2D {}, read back {})",
                seen.1,
                count("imported") - imported,
                count("gpuCopied") - gpu_copied,
                count("texImage") - tex_image,
                count("readBack") - read_back
            )
            .into(),
        );
        assert_eq!(seen.0, want, "the picture at {ms} ms on the {kind} canvas: {:?}", seen.1);
    }
    // Playing to the end.
    p.play();
    for _ in 0..800 {
        if p.state() == PlaybackState::Ended {
            break;
        }
        wait_ms(10).await;
    }
    assert_eq!(p.state(), PlaybackState::Ended, "the clip plays to the end");
    let stats = p.stats();
    let copied = count("copyTo") - copies;
    let canvas_copied = count("canvasCopied");
    let (imported, gpu_copied, tex_image, read_back) = (
        count("imported") - imported,
        count("gpuCopied") - gpu_copied,
        count("texImage") - tex_image,
        count("readBack") - read_back,
    );
    web_sys::console::log_1(
        &format!(
            "{kind}: {stats:?}; copyTo {copied}, importExternalTexture {imported}, \
             copyExternalImageToTexture {gpu_copied} ({canvas_copied} from a canvas), texSubImage2D {tex_image}, read back {read_back}"
        )
        .into(),
    );
    assert!(stats.frames_shown > 0, "{stats:?}");
    assert_eq!(copied, 0, "no picture was copied out on the GPU path");
    let on_gpu = imported + gpu_copied + tex_image;
    assert!(on_gpu > 0, "pictures reached the screen through the GPU");
    if kind == "webgl2" {
        assert_eq!((imported, gpu_copied), (0, 0), "WebGL2 has no WebGPU device");
        assert_eq!(read_back, 0, "every picture went to texSubImage2D");
    } else {
        // WebGPU: the first pictures may come before the browser has said whether it imports
        // frames (the pipeline's error scope), and those are read back.
        assert_eq!(tex_image, 0, "WebGPU has no WebGL2 context");
        assert!(read_back <= 2, "{read_back} pictures read back");
        if no_import {
            assert_eq!(imported, 0, "nothing is imported where WebGPU cannot");
        }
    }
    assert_eq!(count("gpuErrors"), 0, "WebGPU reported no errors");
    if let Some(a) = app.borrow().as_ref() {
        a.quit();
    }
}
