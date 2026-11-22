//! Decoding in a browser page: the browser's own decoder decodes and scales the picture
//! (`createImageBitmap`, off the page's thread), so an image on the canvas has the pixels an
//! `<img>` of the same size shows on the page.

use std::sync::Arc;

use remedy_ui_paint::Image;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    ImageBitmap, ImageBitmapOptions, ImageOrientation, OffscreenCanvasRenderingContext2d, ResizeQuality,
};

use crate::decode::{DecodeError, Decoded, dimensions, fitted, premultiply};

/// Decodes `bytes` with the page's decoder to show in a `box_px` box of device pixels (`None`:
/// at its own size), fitting inside it or (`cover`) covering it, and hands the result to
/// `done` on the page's thread.
pub(crate) fn decode(
    bytes: Arc<[u8]>,
    box_px: Option<(u32, u32)>,
    cover: bool,
    done: impl FnOnce(Result<Decoded, DecodeError>) + 'static,
) {
    wasm_bindgen_futures::spawn_local(async move { done(decode_async(&bytes, box_px, cover).await) });
}

fn js_error(what: &str, e: wasm_bindgen::JsValue) -> DecodeError {
    DecodeError::Malformed(format!("{what}: {}", e.as_string().unwrap_or_else(|| format!("{e:?}"))))
}

/// The options that turn the picture upright by its EXIF orientation and, when `to` is given,
/// scale it there with the browser's best filter (the one it shows `<img>` downscaled with).
fn options(to: Option<(u32, u32)>) -> ImageBitmapOptions {
    let o = ImageBitmapOptions::new();
    o.set_image_orientation(ImageOrientation::FromImage);
    if let Some((w, h)) = to {
        o.set_resize_width(w);
        o.set_resize_height(h);
        o.set_resize_quality(ResizeQuality::High);
    }
    o
}

async fn bitmap(promise: Result<js_sys::Promise, wasm_bindgen::JsValue>) -> Result<ImageBitmap, DecodeError> {
    let promise = promise.map_err(|e| js_error("the browser could not decode it", e))?;
    let value = JsFuture::from(promise).await.map_err(|e| js_error("the browser could not decode it", e))?;
    value.dyn_into::<ImageBitmap>().map_err(|e| js_error("the browser gave no picture", e))
}

async fn decode_async(bytes: &[u8], box_px: Option<(u32, u32)>, cover: bool) -> Result<Decoded, DecodeError> {
    let window = web_sys::window().ok_or_else(|| DecodeError::Io("there is no browser window".into()))?;
    // Read from the header when this build knows the format; the browser may know more.
    let natural = dimensions(bytes).ok();
    let target = |n: (u32, u32)| box_px.map(|b| fitted(n, b, cover)).filter(|t| *t != n);
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(|e| js_error("no blob", e))?;
    let first = natural.and_then(target);
    let mut bm =
        bitmap(window.create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &options(first))).await?;
    let source = natural.unwrap_or((bm.width(), bm.height()));
    if natural.is_none()
        && let Some(t) = target(source)
    {
        // A format only the browser reads: scaled once its size is known.
        let scaled = bitmap(
            window.create_image_bitmap_with_image_bitmap_and_image_bitmap_options(&bm, &options(Some(t))),
        )
        .await;
        bm.close();
        bm = scaled?;
    }
    let (w, h) = (bm.width(), bm.height());
    let pixels = read(&bm, w, h);
    bm.close();
    let mut pixels = pixels?;
    premultiply(&mut pixels);
    Ok(Decoded::new(Image::new(w, h, Arc::from(pixels)), source))
}

/// The bitmap's pixels as straight-alpha RGBA, row by row.
fn read(bm: &ImageBitmap, w: u32, h: u32) -> Result<Vec<u8>, DecodeError> {
    let canvas = web_sys::OffscreenCanvas::new(w, h).map_err(|e| js_error("no canvas", e))?;
    let ctx = canvas
        .get_context("2d")
        .map_err(|e| js_error("no canvas", e))?
        .and_then(|c| c.dyn_into::<OffscreenCanvasRenderingContext2d>().ok())
        .ok_or_else(|| DecodeError::Io("the canvas gave no 2D context".into()))?;
    ctx.draw_image_with_image_bitmap(bm, 0.0, 0.0).map_err(|e| js_error("drawing it failed", e))?;
    let data = ctx
        .get_image_data(0.0, 0.0, f64::from(w), f64::from(h))
        .map_err(|e| js_error("reading it failed", e))?;
    Ok(data.data().0)
}
