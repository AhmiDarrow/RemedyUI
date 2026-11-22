//! A page's picture-in-picture window where the picture is the engine's own (WebCodecs): the
//! Document Picture-in-Picture API (Chrome) opens a small always-on-top window, and each
//! picture the player shows is drawn into a canvas in it, a frame on the GPU as it is
//! (`drawImage` takes it), pixels in memory through `putImageData`.

use super::{call, get, has_global, obj};
use crate::host::Picture;
use crate::player::Player;
use js_sys::{Array, Reflect, Uint8ClampedArray};
use remedy_ui_paint::{GpuTexture, VideoFrame};
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::convert::FromWasmAbi;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Whether this page can open a Document Picture-in-Picture window.
pub(crate) fn supported() -> bool {
    has_global("documentPictureInPicture")
}

/// The window's canvas and its 2D context.
struct Canvas {
    canvas: JsValue,
    ctx: JsValue,
}

// SAFETY: a page without shared memory runs everything on its one thread (web.rs refuses a
// build with `atomics`), so the canvas never leaves the thread that made it.
unsafe impl Send for Canvas {}
// SAFETY: as above.
unsafe impl Sync for Canvas {}

impl Canvas {
    fn size(&self, w: u32, h: u32) {
        for (k, v) in [("width", w), ("height", h)] {
            if get(&self.canvas, k).as_f64() != Some(f64::from(v)) {
                let _ = Reflect::set(&self.canvas, &JsValue::from_str(k), &v.into());
            }
        }
    }

    fn draw(&self, p: &Picture) {
        self.size(p.width, p.height);
        if let Some(g) = &p.gpu
            && let GpuTexture::WebVideoFrame { frame, .. } = g.0.texture()
        {
            // SAFETY: `frame` is the handle of the picture's own reference to its frame (or
            // element), alive while `p` is (borrowed for this call); wrapped so it is never
            // dropped here.
            let source = ManuallyDrop::new(unsafe { JsValue::from_abi(frame) });
            let _ = call(&self.ctx, "drawImage", &[JsValue::clone(&source), 0.into(), 0.into()]);
            return;
        }
        let frame = VideoFrame {
            stream: 0,
            id: 0,
            width: p.width,
            height: p.height,
            format: p.format,
            matrix: p.matrix,
            full_range: p.full_range,
            planes: p.planes,
            data: p.data.clone(),
            gpu: p.gpu.clone(),
        };
        let Some(rgba) = frame.to_rgba() else { return };
        let data = Uint8ClampedArray::from(rgba.as_slice());
        let class: js_sys::Function = get(&js_sys::global(), "ImageData").unchecked_into();
        if let Ok(image) = Reflect::construct(&class, &Array::of3(&data, &p.width.into(), &p.height.into())) {
            let _ = call(&self.ctx, "putImageData", &[image, 0.into(), 0.into()]);
        }
    }
}

/// Opens the window for `player` (sized to its picture, as the browser allows) and hands it
/// the player's pictures; the player hears when the person closes it. A refusal (no recent
/// press in the page) opens nothing.
pub(crate) fn open(player: &Player) {
    let (w, h) = player.info().and_then(|i| i.video).map_or((480, 270), |v| {
        let w = 480u32;
        (w, (u64::from(w) * u64::from(v.height.max(1)) / u64::from(v.width.max(1))) as u32)
    });
    let dpip = get(&js_sys::global(), "documentPictureInPicture");
    let Ok(asked) = call(&dpip, "requestWindow", &[obj(&[("width", w.into()), ("height", h.into())]).into()])
    else {
        return;
    };
    let player = player.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let Ok(win) = JsFuture::from(js_sys::Promise::from(asked)).await else { return };
        let doc = get(&win, "document");
        let body = get(&doc, "body");
        let _ = Reflect::set(&get(&body, "style"), &"cssText".into(), &"margin:0;background:#000;overflow:hidden".into());
        let Ok(canvas) = call(&doc, "createElement", &["canvas".into()]) else { return };
        let _ = Reflect::set(
            &get(&canvas, "style"),
            &"cssText".into(),
            &"display:block;width:100vw;height:100vh;object-fit:contain".into(),
        );
        let _ = call(&body, "appendChild", std::slice::from_ref(&canvas));
        let Ok(ctx) = call(&canvas, "getContext", &["2d".into()]) else { return };
        let target = Arc::new(Canvas { canvas, ctx });
        let mirror: crate::engine::Mirror = Arc::new(move |p: &Picture| target.draw(p));
        // The person closing the window (or the page going) brings the picture back.
        let closed = Rc::new(Closure::<dyn FnMut(JsValue)>::new({
            let player = player.clone();
            // Later: this closure goes with the window it hears.
            move |_| {
                let p = player.clone();
                wasm_bindgen_futures::spawn_local(async move { p.pip_closed() });
            }
        }));
        let _ = call(&win, "addEventListener", &["pagehide".into(), closed.as_ref().as_ref().clone()]);
        let close = Box::new(move || {
            let _ = call(&win, "removeEventListener", &["pagehide".into(), closed.as_ref().as_ref().clone()]);
            let _ = call(&win, "close", &[]);
        });
        player.pip_opened(close, Some(mirror));
    });
}
