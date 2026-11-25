//! Media in a browser page: the browser's own decoders through WebCodecs (`VideoDecoder`,
//! `AudioDecoder`), fed compressed samples by the MP4 reader, and sound through WebAudio,
//! whose clock (what has been heard) the player shows pictures against.
//!
//! The browser decodes asynchronously: samples arrive in its callbacks and the decoder
//! reports `Pending` until they do, waking the player's decoding worker when they come. A
//! source given by URL is fetched without blocking first ([`Decoder::poll_ready`]); a file
//! the user picked or dropped is read from the web bridge's file store.
//!
//! Decoded pictures stay on the GPU when the page draws with the GPU renderer (it announces
//! [`GpuAdapter::Browser`](remedy_ui_paint::GpuAdapter::Browser)): each `VideoFrame` is handed
//! over as it is ([`GpuTexture::WebVideoFrame`](remedy_ui_paint::GpuTexture::WebVideoFrame)),
//! for the renderer to import, and closed when the last clone of its picture goes. Otherwise
//! (the CPU renderer, or more frames held than the browser's decoder can spare) its pixels are
//! copied out with `copyTo` and the frame is closed at once.
//!
//! The unsafe lines in this module say the decoder, the sound output and a picture on the GPU
//! may be "sent" or shared: a page built without shared memory has a single thread, so
//! nothing is ever sent anywhere. A build with threads (`atomics`) refuses to compile rather
//! than be wrong.

#[cfg(target_feature = "atomics")]
compile_error!("the web media host is for pages with one thread; a threaded build needs workers of its own");

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use js_sys::{Array, Float32Array, Object, Promise, Reflect, Uint8Array};
use remedy_ui_paint::{CpuPixels, GpuAdapter, GpuPicture, GpuTexture};
use wasm_bindgen::JsCast;
use wasm_bindgen::convert::IntoWasmAbi;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture,
    PixelFormat, Plane, Sample, VideoInfo, YuvMatrix,
};
use crate::mp4::{self, Kind, Mp4};

// HLS: the crate's reader, fetched by the page and decoded by these WebCodecs decoders.
#[path = "web_hls.rs"]
mod hls_web;

// Pages without WebCodecs: a hidden media element, HLS through Media Source Extensions.
#[path = "web_mse.rs"]
mod mse;

// A picture-in-picture window of the page's own for pictures the engine draws.
#[path = "web_pip.rs"]
pub(crate) mod pip;

#[wasm_bindgen]
extern "C" {
    type VideoDecoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &Object) -> Result<VideoDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &VideoDecoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn decode(this: &VideoDecoder, chunk: &EncodedVideoChunk) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn flush(this: &VideoDecoder) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn reset(this: &VideoDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn close(this: &VideoDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    fn decode_queue_size(this: &VideoDecoder) -> u32;

    type EncodedVideoChunk;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &Object) -> Result<EncodedVideoChunk, JsValue>;

    type VideoFrame;
    #[wasm_bindgen(method, getter)]
    fn format(this: &VideoFrame) -> Option<String>;
    #[wasm_bindgen(method, getter)]
    fn timestamp(this: &VideoFrame) -> f64;
    #[wasm_bindgen(method, getter)]
    fn duration(this: &VideoFrame) -> Option<f64>;
    #[wasm_bindgen(method, getter, js_name = visibleRect)]
    fn visible_rect(this: &VideoFrame) -> JsValue;
    #[wasm_bindgen(method, getter, js_name = displayWidth)]
    fn display_width(this: &VideoFrame) -> u32;
    #[wasm_bindgen(method, getter, js_name = displayHeight)]
    fn display_height(this: &VideoFrame) -> u32;
    #[wasm_bindgen(method, getter, js_name = colorSpace)]
    fn color_space(this: &VideoFrame) -> JsValue;
    #[wasm_bindgen(method, catch, js_name = allocationSize)]
    fn allocation_size(this: &VideoFrame) -> Result<u32, JsValue>;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    fn copy_to(this: &VideoFrame, dest: &Uint8Array) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = close)]
    fn close_frame(this: &VideoFrame);

    /// A 2D canvas's context (an `OffscreenCanvas`'s), which reads a frame's pixels back
    /// without waiting.
    type Canvas2d;
    #[wasm_bindgen(method, catch, js_name = drawImage)]
    fn draw_frame(this: &Canvas2d, frame: &VideoFrame, x: f64, y: f64) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch, js_name = getImageData)]
    fn get_image_data(this: &Canvas2d, x: f64, y: f64, w: f64, h: f64) -> Result<JsValue, JsValue>;

    type AudioDecoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &Object) -> Result<AudioDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &AudioDecoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn decode(this: &AudioDecoder, chunk: &EncodedAudioChunk) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn flush(this: &AudioDecoder) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn reset(this: &AudioDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn close(this: &AudioDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    fn decode_queue_size(this: &AudioDecoder) -> u32;

    type EncodedAudioChunk;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &Object) -> Result<EncodedAudioChunk, JsValue>;

    type AudioData;
    #[wasm_bindgen(method, getter, js_name = numberOfFrames)]
    fn number_of_frames(this: &AudioData) -> u32;
    #[wasm_bindgen(method, getter, js_name = numberOfChannels)]
    fn number_of_channels(this: &AudioData) -> u32;
    #[wasm_bindgen(method, getter)]
    fn timestamp(this: &AudioData) -> f64;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    fn copy_to(this: &AudioData, dest: &Float32Array, options: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, js_name = close)]
    fn close_data(this: &AudioData);
}

fn obj(pairs: &[(&str, JsValue)]) -> Object {
    let o = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn js_error(what: &str, e: &JsValue) -> String {
    let msg = get(e, "message").as_string().or_else(|| e.as_string()).unwrap_or_else(|| format!("{e:?}"));
    format!("{what}: {msg}")
}

fn has_global(name: &str) -> bool {
    Reflect::has(&js_sys::global(), &JsValue::from_str(name)).unwrap_or(false)
}

/// Microseconds (WebCodecs' time unit) as a duration.
fn micros(us: f64) -> Duration {
    Duration::from_secs_f64(us.max(0.0) / 1_000_000.0)
}

/// Where a decoded sample waits: decoded pictures are copied out asynchronously, so their
/// place in the order is kept until the copy lands.
enum Slot {
    Ready(Sample),
    Copying(u64),
}

/// What the browser's callbacks hand back.
#[derive(Default)]
struct Outputs {
    queue: VecDeque<Slot>,
    error: Option<MediaError>,
    waker: Option<Waker>,
    /// Bumped by each seek: copies started before it are dropped.
    generation: u64,
    next_copy: u64,
    /// Both decoders have flushed everything after the last input.
    flushed: bool,
    /// The source has been fetched and the decoders configured (or that failed).
    ready: Option<Result<(), MediaError>>,
    /// Frames handed over on the GPU and not yet closed.
    held: Rc<Cell<usize>>,
}

impl Outputs {
    fn wake(&mut self) {
        if let Some(w) = self.waker.take() {
            w.wake();
        }
    }

    fn fail(&mut self, e: MediaError) {
        if self.error.is_none() {
            self.error = Some(e);
        }
        self.wake();
    }
}

type Shared = Rc<RefCell<Outputs>>;

/// The source's bytes as they arrive: all at once for a file, a chunk at a time while a URL
/// is fetched.
#[derive(Default)]
struct Bytes {
    data: Vec<u8>,
    /// Every byte is in.
    complete: bool,
    /// The decoder is gone: a fetch under way stops.
    dropped: bool,
}

type SharedBytes = Rc<RefCell<Bytes>>;

/// The parts that exist once the source's movie box is in hand.
struct Open {
    data: SharedBytes,
    mp4: Mp4,
    video: Option<(usize, VideoDecoder, Object)>,
    audio: Option<(usize, AudioDecoder, Object)>,
    next_video: usize,
    next_audio: usize,
    info: MediaInfo,
    flushing: bool,
    _callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
}

/// A source decoded by the browser.
struct WebDecoder {
    out: Shared,
    open: Rc<RefCell<Option<Open>>>,
    bytes: SharedBytes,
    info: MediaInfo,
}

// SAFETY: a page without shared memory runs everything on its one thread (the build with
// `atomics` does not compile, above), so the decoder never leaves the thread that made it.
unsafe impl Send for WebDecoder {}

fn picture(frame: &VideoFrame, layout: &JsValue, bytes: Arc<[u8]>) -> Result<Picture, MediaError> {
    let rect = frame.visible_rect();
    let width = get(&rect, "width").as_f64().unwrap_or(0.0) as u32;
    let height = get(&rect, "height").as_f64().unwrap_or(0.0) as u32;
    let planes_js = Array::from(layout);
    let plane = |i: u32| {
        let p = planes_js.get(i);
        Plane {
            offset: get(&p, "offset").as_f64().unwrap_or(0.0) as usize,
            stride: get(&p, "stride").as_f64().unwrap_or(0.0) as usize,
        }
    };
    let (matrix, full_range) = color(frame);
    let format = frame.format().unwrap_or_default();
    let (format, planes, data) = match format.as_str() {
        "I420" | "I420A" => (PixelFormat::I420, [plane(0), plane(1), plane(2)], bytes),
        "NV12" => (PixelFormat::Nv12, [plane(0), plane(1), Plane::default()], bytes),
        "RGBA" | "RGBX" | "BGRA" | "BGRX" => {
            let p = plane(0);
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for y in 0..height as usize {
                let row = bytes.get(p.offset + y * p.stride..).unwrap_or_default();
                for px in row.as_chunks::<4>().0.iter().take(width as usize) {
                    let [a, b, c, d] = *px;
                    let alpha = if format.ends_with('X') { 255 } else { d };
                    let (r, bl) = if format.starts_with('B') { (c, a) } else { (a, c) };
                    rgba.extend_from_slice(&[r, b, bl, alpha]);
                }
            }
            let stride = width as usize * 4;
            (
                PixelFormat::Rgba,
                [Plane { offset: 0, stride }, Plane::default(), Plane::default()],
                Arc::from(rgba),
            )
        }
        other => {
            return Err(MediaError::Decode(format!(
                "the browser's decoder gave pictures in {other}, which is not shown"
            )));
        }
    };
    Ok(Picture { width, height, format, planes, matrix, full_range, data, gpu: None })
}

/// The most decoded frames handed over on the GPU at once. A browser's decoder has a pool
/// of a limited size and waits for frames to be closed before it decodes more; past this, frames
/// are copied out and closed at once, so pictures the player holds ahead never stall it. It
/// covers the player's look-ahead at 30 frames a second.
const HELD_FRAMES: usize = 24;

/// A decoded `VideoFrame` handed to the renderer as it is: closed when the last clone of its
/// picture is dropped.
struct WebFrame {
    frame: VideoFrame,
    width: u32,
    height: u32,
    held: Rc<Cell<usize>>,
}

// SAFETY: as for the decoder: one thread in a page without shared memory, so the frame is
// never used from another.
unsafe impl Send for WebFrame {}
// SAFETY: as above.
unsafe impl Sync for WebFrame {}

impl GpuPicture for WebFrame {
    fn texture(&self) -> GpuTexture {
        let frame: &JsValue = &self.frame;
        GpuTexture::WebVideoFrame { frame: frame.into_abi(), width: self.width, height: self.height }
    }

    /// The pixels through a 2D canvas, which draws the frame and reads it back at once (the
    /// CPU renderer cannot wait for `copyTo`'s promise).
    fn read_back(&self) -> Option<CpuPixels> {
        let (w, h) = (self.width, self.height);
        if !has_global("OffscreenCanvas") {
            return None;
        }
        let canvas = Reflect::construct(
            &get(&js_sys::global(), "OffscreenCanvas").into(),
            &Array::of2(&JsValue::from(w), &JsValue::from(h)),
        )
        .ok()?;
        let get_context: js_sys::Function = get(&canvas, "getContext").dyn_into().ok()?;
        let ctx = get_context.call1(&canvas, &JsValue::from_str("2d")).ok()?;
        if ctx.is_null() || ctx.is_undefined() {
            return None;
        }
        let ctx: Canvas2d = ctx.unchecked_into();
        ctx.draw_frame(&self.frame, 0.0, 0.0).ok()?;
        let image = ctx.get_image_data(0.0, 0.0, f64::from(w), f64::from(h)).ok()?;
        let data: Uint8Array = Uint8Array::new(&get(&image, "data"));
        let stride = w as usize * 4;
        Some(CpuPixels {
            format: PixelFormat::Rgba,
            planes: [Plane { offset: 0, stride }, Plane::default(), Plane::default()],
            data: Arc::from(data.to_vec()),
        })
    }
}

impl Drop for WebFrame {
    fn drop(&mut self) {
        self.frame.close_frame();
        self.held.set(self.held.get().saturating_sub(1));
    }
}

/// `frame` as a picture on the GPU, when a renderer takes it there and the decoder can spare it.
fn gpu_picture(out: &Shared, frame: VideoFrame) -> Result<Picture, VideoFrame> {
    let held = out.borrow().held.clone();
    if remedy_ui_paint::gpu_video_adapter() != Some(GpuAdapter::Browser) || held.get() >= HELD_FRAMES {
        return Err(frame);
    }
    let (width, height) = (frame.display_width(), frame.display_height());
    if width == 0 || height == 0 {
        return Err(frame);
    }
    let (matrix, full_range) = color(&frame);
    held.set(held.get() + 1);
    let picture = Arc::new(WebFrame { frame, width, height, held });
    Ok(Picture::on_gpu(width, height, matrix, full_range, picture))
}

/// The frame's YUV matrix and range.
fn color(frame: &VideoFrame) -> (YuvMatrix, bool) {
    let color = frame.color_space();
    let full_range = get(&color, "fullRange").as_bool().unwrap_or(false);
    let matrix = match get(&color, "matrix").as_string().as_deref() {
        Some("bt470bg" | "smpte170m") => YuvMatrix::Bt601,
        _ => YuvMatrix::Bt709,
    };
    (matrix, full_range)
}

fn video_output(out: &Shared, frame: VideoFrame) {
    // Handed over as it is when the GPU renderer takes it: ready at once, in its place.
    let at = micros(frame.timestamp());
    let duration = frame.duration().map(micros);
    let frame = match gpu_picture(out, frame) {
        Ok(picture) => {
            let mut o = out.borrow_mut();
            o.queue.push_back(Slot::Ready(Sample::Video { at, duration, picture }));
            o.wake();
            return;
        }
        Err(frame) => frame,
    };
    let (seq, generation) = {
        let mut o = out.borrow_mut();
        let seq = o.next_copy;
        o.next_copy += 1;
        o.queue.push_back(Slot::Copying(seq));
        (seq, o.generation)
    };
    let out = out.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let copied = async {
            let size = frame.allocation_size().map_err(|e| js_error("sizing a picture", &e))?;
            copy_out(&frame, size).await
        }
        .await;
        frame.close_frame();
        let mut o = out.borrow_mut();
        if o.generation != generation {
            return;
        }
        let place = o.queue.iter().position(|s| matches!(s, Slot::Copying(n) if *n == seq));
        match copied {
            Ok(picture) => {
                if let Some(i) = place
                    && let Some(slot) = o.queue.get_mut(i)
                {
                    *slot = Slot::Ready(Sample::Video { at, duration, picture });
                }
            }
            Err(e) => {
                if let Some(i) = place {
                    o.queue.remove(i);
                }
                o.fail(MediaError::Decode(e));
            }
        }
        o.wake();
    });
}

/// For measurements: a `VideoFrame` copied out as the decoder copies each picture.
pub(crate) async fn copy_frame(frame: &JsValue) -> Result<Picture, String> {
    let frame: &VideoFrame = frame.unchecked_ref();
    let size = frame.allocation_size().map_err(|e| js_error("sizing a picture", &e))?;
    copy_out(frame, size).await
}

/// `frame`'s `size` bytes copied once, by `copyTo`, straight into the picture's own buffer in
/// wasm memory (was: into a JS array, out of it into a `Vec`, and the `Vec` into the
/// picture's `Arc`: three copies).
async fn copy_out(frame: &VideoFrame, size: u32) -> Result<Picture, String> {
    let copying = |e: JsValue| js_error("copying a picture", &e);
    let len = size as usize;
    // SAFETY: zeroed bytes are valid `u8`s.
    let mut data: Arc<[u8]> = unsafe { Arc::<[u8]>::new_zeroed_slice(len).assume_init() };
    let Some(bytes) = Arc::get_mut(&mut data) else { unreachable!("a new Arc is not shared") };
    // SAFETY: a view over `data`'s bytes, which nothing reads or writes until the copy has
    // resolved; `data` outlives it.
    let view = unsafe { Uint8Array::view_mut_raw(bytes.as_mut_ptr(), len) };
    let layout = JsFuture::from(frame.copy_to(&view).map_err(copying)?).await.map_err(copying)?;
    // Memory that grew while the copy ran detaches the view's buffer, and what was copied
    // after may be lost: copied again into a JS array, then into the picture.
    if view.byte_length() as usize != len {
        let buf = Uint8Array::new_with_length(size);
        let layout = JsFuture::from(frame.copy_to(&buf).map_err(copying)?).await.map_err(copying)?;
        let Some(bytes) = Arc::get_mut(&mut data) else { unreachable!("not shared yet") };
        buf.copy_to(bytes);
        return picture(frame, &layout, data).map_err(|e| e.to_string());
    }
    picture(frame, &layout, data).map_err(|e| e.to_string())
}

fn audio_output(out: &Shared, data: &AudioData) -> Result<(), MediaError> {
    let frames = data.number_of_frames() as usize;
    let channels = data.number_of_channels().max(1) as usize;
    let mut planes = Vec::with_capacity(channels);
    for ch in 0..channels {
        let buf = Float32Array::new_with_length(frames as u32);
        let opts =
            obj(&[("planeIndex", JsValue::from(ch as u32)), ("format", JsValue::from_str("f32-planar"))]);
        data.copy_to(&buf, &opts).map_err(|e| MediaError::Decode(js_error("reading decoded sound", &e)))?;
        planes.push(buf.to_vec());
    }
    let mut samples = Vec::with_capacity(frames * channels);
    for f in 0..frames {
        for p in &planes {
            samples.push(p.get(f).copied().unwrap_or(0.0));
        }
    }
    let mut o = out.borrow_mut();
    o.queue.push_back(Slot::Ready(Sample::Audio { at: micros(data.timestamp()), samples }));
    o.wake();
    Ok(())
}

/// Makes and configures the decoders for `data`.
fn open(data: SharedBytes, mp4: Mp4, out: &Shared) -> Result<Open, MediaError> {
    let mut callbacks: Vec<Closure<dyn FnMut(JsValue)>> = Vec::new();
    let error_cb = {
        let out = out.clone();
        Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
            out.borrow_mut().fail(MediaError::Decode(js_error("the browser's decoder failed", &e)));
        })
    };
    let dequeue = {
        let out = out.clone();
        Closure::<dyn FnMut(JsValue)>::new(move |_| out.borrow_mut().wake())
    };
    let video = match mp4.video().and_then(|i| mp4.tracks.get(i).map(|t| (i, t))) {
        Some((i, t)) => {
            if !has_global("VideoDecoder") {
                return Err(MediaError::Unsupported("this browser has no WebCodecs video decoder".into()));
            }
            let out2 = out.clone();
            let output =
                Closure::<dyn FnMut(JsValue)>::new(move |f: JsValue| video_output(&out2, f.unchecked_into()));
            let init = obj(&[("output", output.as_ref().clone()), ("error", error_cb.as_ref().clone())]);
            let dec = VideoDecoder::new(&init)
                .map_err(|e| MediaError::Unsupported(js_error("video decoding", &e)))?;
            let _ = Reflect::set(&dec, &"ondequeue".into(), dequeue.as_ref());
            let Kind::Video { width, height } = t.kind else {
                return Err(MediaError::Format("not video".into()));
            };
            let config = obj(&[
                ("codec", JsValue::from_str(&t.codec)),
                ("description", Uint8Array::from(t.description.as_slice()).into()),
                ("codedWidth", JsValue::from(width)),
                ("codedHeight", JsValue::from(height)),
                ("optimizeForLatency", JsValue::TRUE),
            ]);
            dec.configure(&config)
                .map_err(|e| MediaError::Unsupported(js_error(&format!("decoding {}", t.codec), &e)))?;
            callbacks.push(output);
            Some((i, dec, config))
        }
        None => None,
    };
    let audio = match mp4.audio().and_then(|i| mp4.tracks.get(i).map(|t| (i, t))) {
        Some((i, t)) => {
            if !has_global("AudioDecoder") {
                return Err(MediaError::Unsupported("this browser has no WebCodecs sound decoder".into()));
            }
            let out2 = out.clone();
            let output = Closure::<dyn FnMut(JsValue)>::new(move |d: JsValue| {
                let data: AudioData = d.unchecked_into();
                if let Err(e) = audio_output(&out2, &data) {
                    out2.borrow_mut().fail(e);
                }
                data.close_data();
            });
            let init = obj(&[("output", output.as_ref().clone()), ("error", error_cb.as_ref().clone())]);
            let dec = AudioDecoder::new(&init)
                .map_err(|e| MediaError::Unsupported(js_error("sound decoding", &e)))?;
            let _ = Reflect::set(&dec, &"ondequeue".into(), dequeue.as_ref());
            let Kind::Audio { sample_rate, channels } = t.kind else {
                return Err(MediaError::Format("not sound".into()));
            };
            let config = obj(&[
                ("codec", JsValue::from_str(&t.codec)),
                ("description", Uint8Array::from(t.description.as_slice()).into()),
                ("sampleRate", JsValue::from(sample_rate)),
                ("numberOfChannels", JsValue::from(channels)),
            ]);
            dec.configure(&config)
                .map_err(|e| MediaError::Unsupported(js_error(&format!("decoding {}", t.codec), &e)))?;
            callbacks.push(output);
            Some((i, dec, config))
        }
        None => None,
    };
    callbacks.push(error_cb);
    callbacks.push(dequeue);
    let length = mp4.tracks.iter().map(mp4::Track::length).max();
    let info = MediaInfo {
        duration: length,
        seekable: true,
        growing: false,
        video: video.as_ref().and_then(|(i, ..)| mp4.tracks.get(*i)).and_then(|t| match t.kind {
            Kind::Video { width, height } => {
                let secs = t.length().as_secs_f64();
                let frame_rate = (secs > 0.0).then(|| t.samples.len() as f64 / secs);
                Some(VideoInfo { width, height, frame_rate })
            }
            Kind::Audio { .. } | Kind::Text { .. } => None,
        }),
        audio: audio.as_ref().and_then(|(i, ..)| mp4.tracks.get(*i)).and_then(|t| match t.kind {
            Kind::Audio { sample_rate, channels } => Some(AudioFormat { sample_rate, channels }),
            Kind::Video { .. } | Kind::Text { .. } => None,
        }),
        text_tracks: Vec::new(),
    };
    Ok(Open {
        data,
        mp4,
        video,
        audio,
        next_video: 0,
        next_audio: 0,
        info,
        flushing: false,
        _callbacks: callbacks,
    })
}

impl Open {
    fn chunk_init(&self, track: usize, i: usize) -> Option<Object> {
        let t = self.mp4.tracks.get(track)?;
        let s = t.samples.get(i)?;
        let start = usize::try_from(s.offset).ok()?;
        let data = self.data.borrow();
        let bytes = data.data.get(start..start + s.size as usize)?;
        let us = |d: Duration| JsValue::from_f64(d.as_secs_f64() * 1_000_000.0);
        Some(obj(&[
            ("type", JsValue::from_str(if s.sync { "key" } else { "delta" })),
            ("timestamp", us(t.time(s.pts))),
            ("duration", us(t.time(i64::from(s.duration)))),
            ("data", Uint8Array::from(bytes).into()),
        ]))
    }

    /// Whether sample `i` of `track` is still to arrive (a fetch under way).
    fn waiting_for(&self, track: usize, i: usize) -> bool {
        let data = self.data.borrow();
        !data.complete
            && self
                .mp4
                .tracks
                .get(track)
                .and_then(|t| t.samples.get(i))
                .is_some_and(|s| s.offset.saturating_add(u64::from(s.size)) > data.data.len() as u64)
    }

    /// What has arrived, as a stretch of the media from its start: up to the first sample of
    /// the video or sound track whose bytes are still to come.
    fn held(&self) -> Vec<std::ops::Range<Duration>> {
        let data = self.data.borrow();
        let length = self.info.duration.unwrap_or_default();
        if data.complete {
            return vec![Duration::ZERO..length];
        }
        let arrived = data.data.len() as u64;
        let tracks = [self.video.as_ref().map(|v| v.0), self.audio.as_ref().map(|a| a.0)];
        let until = tracks
            .into_iter()
            .flatten()
            .filter_map(|t| self.mp4.tracks.get(t))
            .map(|t| {
                t.samples
                    .iter()
                    .find(|s| s.offset.saturating_add(u64::from(s.size)) > arrived)
                    .map_or(t.length(), |s| t.time(s.pts))
            })
            .min()
            .unwrap_or_default()
            .min(length);
        if until.is_zero() { Vec::new() } else { vec![Duration::ZERO..until] }
    }

    fn next_time(&self, track: usize, i: usize) -> Option<i64> {
        let t = self.mp4.tracks.get(track)?;
        let s = t.samples.get(i)?;
        // Compared across tracks in microseconds.
        Some((t.time(s.pts).as_micros()) as i64)
    }

    /// Feeds the decoders the next samples, earliest first, while they are not busy; true
    /// once every sample has gone in.
    fn feed(&mut self) -> Result<bool, MediaError> {
        loop {
            // The earliest sample goes next, in time order across the tracks, so neither runs
            // ahead of the other; it waits while its decoder is still busy with the last two
            // (a decoder works on one while the next waits, which keeps it fed and no more).
            let v = self.video.as_ref().and_then(|(t, _, _)| self.next_time(*t, self.next_video));
            let a = self.audio.as_ref().and_then(|(t, _, _)| self.next_time(*t, self.next_audio));
            let take_video = match (v, a) {
                (Some(v), Some(a)) => v <= a,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let busy = if take_video {
                self.video.as_ref().is_some_and(|(t, d, _)| {
                    d.decode_queue_size() >= 2 || self.waiting_for(*t, self.next_video)
                })
            } else {
                self.audio.as_ref().is_some_and(|(t, d, _)| {
                    d.decode_queue_size() >= 2 || self.waiting_for(*t, self.next_audio)
                })
            };
            // A sample still downloading waits too: each chunk that arrives wakes the reader.
            if busy {
                break;
            }
            if take_video {
                let Some((t, dec, _)) = self.video.as_ref() else { break };
                let init = self
                    .chunk_init(*t, self.next_video)
                    .ok_or_else(|| MediaError::Format("a picture outside the file".into()))?;
                let chunk = EncodedVideoChunk::new(&init)
                    .map_err(|e| MediaError::Decode(js_error("a picture", &e)))?;
                dec.decode(&chunk).map_err(|e| MediaError::Decode(js_error("decoding a picture", &e)))?;
                self.next_video += 1;
            } else {
                let Some((t, dec, _)) = self.audio.as_ref() else { break };
                let init = self
                    .chunk_init(*t, self.next_audio)
                    .ok_or_else(|| MediaError::Format("sound outside the file".into()))?;
                let chunk =
                    EncodedAudioChunk::new(&init).map_err(|e| MediaError::Decode(js_error("sound", &e)))?;
                dec.decode(&chunk).map_err(|e| MediaError::Decode(js_error("decoding sound", &e)))?;
                self.next_audio += 1;
            }
        }
        let video_done = self.video.as_ref().is_none_or(|(t, ..)| {
            self.mp4.tracks.get(*t).is_none_or(|tr| self.next_video >= tr.samples.len())
        });
        let audio_done = self.audio.as_ref().is_none_or(|(t, ..)| {
            self.mp4.tracks.get(*t).is_none_or(|tr| self.next_audio >= tr.samples.len())
        });
        Ok(video_done && audio_done)
    }
}

impl Decoder for WebDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MediaError>> {
        let mut o = self.out.borrow_mut();
        match o.ready.clone() {
            Some(Ok(())) => {
                drop(o);
                if let Some(op) = self.open.borrow().as_ref() {
                    self.info = op.info.clone();
                }
                Poll::Ready(Ok(()))
            }
            Some(Err(e)) => Poll::Ready(Err(e)),
            None => {
                o.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        // The browser decodes asynchronously; nothing can wait for it on the page's thread.
        let waker = std::task::Waker::noop();
        match self.poll_read(&mut Context::from_waker(waker)) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(MediaError::Decode(
                "the browser's decoder answers later; this decoder is read with poll_read".into(),
            )),
        }
    }

    fn poll_read(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Sample>, MediaError>> {
        {
            let mut o = self.out.borrow_mut();
            o.waker = Some(cx.waker().clone());
            if let Some(e) = o.error.take() {
                return Poll::Ready(Err(e));
            }
            if matches!(o.queue.front(), Some(Slot::Ready(_)))
                && let Some(Slot::Ready(s)) = o.queue.pop_front()
            {
                return Poll::Ready(Ok(Some(s)));
            }
        }
        let mut open = self.open.borrow_mut();
        let Some(op) = open.as_mut() else { return Poll::Pending };
        let all_in = match op.feed() {
            Ok(done) => done,
            Err(e) => return Poll::Ready(Err(e)),
        };
        if all_in && !op.flushing {
            op.flushing = true;
            let flushes: Vec<Promise> = op
                .video
                .as_ref()
                .and_then(|(_, d, _)| d.flush().ok())
                .into_iter()
                .chain(op.audio.as_ref().and_then(|(_, d, _)| d.flush().ok()))
                .collect();
            let (out, generation) = (self.out.clone(), self.out.borrow().generation);
            wasm_bindgen_futures::spawn_local(async move {
                for f in flushes {
                    // A reset (a seek) rejects a flush under way; the generation tells.
                    let _ = JsFuture::from(f).await;
                }
                let mut o = out.borrow_mut();
                if o.generation == generation {
                    o.flushed = true;
                    o.wake();
                }
            });
        }
        let o = self.out.borrow();
        if all_in && o.flushed && o.queue.is_empty() {
            return Poll::Ready(Ok(None));
        }
        Poll::Pending
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        let mut open = self.open.borrow_mut();
        let Some(op) = open.as_mut() else {
            return Err(MediaError::Decode("the source is not open yet".into()));
        };
        {
            let mut o = self.out.borrow_mut();
            o.generation += 1;
            o.queue.clear();
            o.flushed = false;
            o.error = None;
        }
        op.flushing = false;
        // Pictures from the key frame at or before the target; sound from the target.
        if let Some((t, dec, config)) = op.video.as_ref() {
            let tr = op.mp4.tracks.get(*t).ok_or_else(|| MediaError::Format("no video track".into()))?;
            op.next_video = tr
                .samples
                .iter()
                .enumerate()
                .filter(|(_, s)| s.sync && tr.time(s.pts) <= to)
                .map(|(i, _)| i)
                .next_back()
                .unwrap_or(0);
            dec.reset().map_err(|e| MediaError::Decode(js_error("seeking", &e)))?;
            dec.configure(config).map_err(|e| MediaError::Decode(js_error("seeking", &e)))?;
        }
        if let Some((t, dec, config)) = op.audio.as_ref() {
            let tr = op.mp4.tracks.get(*t).ok_or_else(|| MediaError::Format("no sound track".into()))?;
            op.next_audio = tr
                .samples
                .iter()
                .position(|s| tr.time(s.pts + i64::from(s.duration)) > to)
                .unwrap_or(tr.samples.len());
            dec.reset().map_err(|e| MediaError::Decode(js_error("seeking", &e)))?;
            dec.configure(config).map_err(|e| MediaError::Decode(js_error("seeking", &e)))?;
        }
        Ok(())
    }

    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        self.open.borrow().as_ref().map(Open::held)
    }
}

impl Drop for WebDecoder {
    fn drop(&mut self) {
        self.bytes.borrow_mut().dropped = true;
        if let Some(op) = self.open.borrow_mut().take() {
            if let Some((_, d, _)) = &op.video {
                let _ = d.close();
            }
            if let Some((_, d, _)) = &op.audio {
                let _ = d.close();
            }
        }
    }
}

/// Calls `o.method(args…)`.
fn call(o: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let f: js_sys::Function = get(o, method).dyn_into()?;
    f.apply(o, &args.iter().collect::<Array>())
}

/// The length of the body a response brings, when its headers say and nothing (a content
/// encoding) makes the bytes read differ from it.
fn body_length(resp: &web_sys::Response) -> Option<u64> {
    let headers = get(resp, "headers");
    let header =
        |name: &str| call(&headers, "get", &[JsValue::from_str(name)]).ok().and_then(|v| v.as_string());
    if header("content-encoding").is_some_and(|e| !e.trim().eq_ignore_ascii_case("identity")) {
        return None;
    }
    header("content-length").and_then(|l| l.trim().parse().ok())
}

/// Fetches `url` without blocking, a chunk at a time into `bytes`, and opens it as soon as
/// its movie box is in (at once for a file made for streaming, its movie box first; at the
/// end otherwise). Samples still to come wait in the feed; each chunk wakes the reader.
fn fetch_and_open(url: String, out: Shared, slot: Rc<RefCell<Option<Open>>>, bytes: SharedBytes) {
    wasm_bindgen_futures::spawn_local(async move {
        let not_found = |e: &JsValue| MediaError::NotFound(js_error(&url, e));
        let opened = |op: Result<Open, MediaError>| {
            let ready = op.map(|op| *slot.borrow_mut() = Some(op));
            let mut o = out.borrow_mut();
            o.ready = Some(ready);
            o.wake();
        };
        let fetched = async {
            let win = web_sys::window().ok_or_else(|| MediaError::Unsupported("no browser window".into()))?;
            let resp = JsFuture::from(win.fetch_with_str(&url)).await.map_err(|e| not_found(&e))?;
            let resp: web_sys::Response = resp.unchecked_into();
            if !resp.ok() {
                return Err(MediaError::NotFound(format!("{url}: the server answered {}", resp.status())));
            }
            let total = body_length(&resp);
            let body = get(&resp, "body");
            if body.is_null() || body.is_undefined() {
                // No readable body (an old browser): all of it at once.
                let buf = resp.array_buffer().map_err(|e| not_found(&e))?;
                let buf = JsFuture::from(buf).await.map_err(|e| not_found(&e))?;
                bytes.borrow_mut().data = Uint8Array::new(&buf).to_vec();
                return Ok(());
            }
            let reader = call(&body, "getReader", &[]).map_err(|e| not_found(&e))?;
            loop {
                if bytes.borrow().dropped {
                    let _ = call(&reader, "cancel", &[]);
                    return Ok(());
                }
                let read = call(&reader, "read", &[]).map_err(|e| not_found(&e))?;
                let chunk = JsFuture::from(Promise::from(read)).await.map_err(|e| not_found(&e))?;
                if get(&chunk, "done").is_truthy() {
                    break;
                }
                let value: Uint8Array = get(&chunk, "value").dyn_into().map_err(|e| not_found(&e))?;
                let mut b = bytes.borrow_mut();
                let at = b.data.len();
                b.data.resize(at + value.length() as usize, 0);
                if let Some(dest) = b.data.get_mut(at..) {
                    value.copy_to(dest);
                }
                let head = match total {
                    Some(len) if slot.borrow().is_none() && out.borrow().ready.is_none() => {
                        mp4::parse_head(&b.data, len).map_err(MediaError::Format)?
                    }
                    _ => None,
                };
                drop(b);
                match head {
                    Some(m) => opened(open(bytes.clone(), m, &out)),
                    None => out.borrow_mut().wake(),
                }
            }
            if let Some(len) = total
                && (bytes.borrow().data.len() as u64) < len
            {
                return Err(MediaError::NotFound(format!(
                    "{url}: the download stopped at {} of {len} bytes",
                    bytes.borrow().data.len()
                )));
            }
            Ok(())
        }
        .await;
        bytes.borrow_mut().complete = true;
        match fetched {
            // Opened once the movie box came in; the last samples are in now.
            Ok(()) if out.borrow().ready.is_some() => out.borrow_mut().wake(),
            Ok(()) => {
                let m = mp4::parse(&bytes.borrow().data).map_err(MediaError::Format);
                opened(m.and_then(|m| open(bytes.clone(), m, &out)));
            }
            Err(e) if out.borrow().ready.is_some() => out.borrow_mut().fail(e),
            Err(e) => opened(Err(e)),
        }
    });
}

/// How much sound is queued ahead in WebAudio: a quarter second, the player's least
/// look-ahead, rides out a late timer (a busy page, a frame's delay) without a gap.
const AUDIO_BUFFER: Duration = Duration::from_millis(250);

/// Sound played through WebAudio: each written block is an `AudioBufferSourceNode` started
/// right after the one before, and what has been heard is read from the context's clock.
struct WebAudio {
    ctx: web_sys::AudioContext,
    format: AudioFormat,
    /// Blocks queued: (context time it starts, its first frame, frames).
    blocks: VecDeque<(f64, u64, u64)>,
    sources: VecDeque<web_sys::AudioBufferSourceNode>,
    /// Frames written since the last flush.
    written: u64,
    /// Frames of blocks already heard and let go.
    heard_base: u64,
    /// Where the next block starts on the context's clock.
    next_time: f64,
    /// How far ahead of the clock sound is scheduled.
    depth: Duration,
}

// SAFETY: as for the decoder: one thread in a page without shared memory.
unsafe impl Send for WebAudio {}

impl WebAudio {
    fn new(format: AudioFormat, latency: crate::host::Latency) -> Result<WebAudio, MediaError> {
        let opts = web_sys::AudioContextOptions::new();
        opts.set_sample_rate(format.sample_rate as f32);
        // Interactive: the browser's shortest output path, and little scheduled ahead.
        let depth = match latency {
            crate::host::Latency::Interactive => {
                let _ = js_sys::Reflect::set(&opts, &"latencyHint".into(), &"interactive".into());
                Duration::from_millis(40)
            }
            _ => {
                let _ = js_sys::Reflect::set(&opts, &"latencyHint".into(), &"playback".into());
                AUDIO_BUFFER
            }
        };
        let ctx = web_sys::AudioContext::new_with_context_options(&opts)
            .or_else(|_| web_sys::AudioContext::new())
            .map_err(|e| MediaError::Device(js_error("WebAudio", &e)))?;
        // Paused until the player resumes it.
        let _ = ctx.suspend();
        let next_time = ctx.current_time();
        Ok(WebAudio {
            ctx,
            format,
            depth,
            blocks: VecDeque::new(),
            sources: VecDeque::new(),
            written: 0,
            heard_base: 0,
            next_time,
        })
    }

    /// The context's clock at the speaker (its output latency taken off).
    fn now(&self) -> f64 {
        let latency = get(&self.ctx, "outputLatency").as_f64().unwrap_or(0.0);
        self.ctx.current_time() - latency.max(0.0)
    }
}

impl AudioOutput for WebAudio {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn buffer(&self) -> Duration {
        self.depth
    }

    fn room(&mut self) -> Result<usize, MediaError> {
        let queued = (self.next_time - self.ctx.current_time()).max(0.0);
        let room = (self.depth.as_secs_f64() - queued).max(0.0);
        Ok((room * f64::from(self.format.sample_rate)) as usize)
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let ch = usize::from(self.format.channels.max(1));
        let frames = samples.len() / ch;
        if frames == 0 {
            return Ok(());
        }
        let fail = |e: JsValue| MediaError::Device(js_error("WebAudio", &e));
        let buf =
            self.ctx.create_buffer(ch as u32, frames as u32, self.format.sample_rate as f32).map_err(fail)?;
        for c in 0..ch {
            let plane: Vec<f32> = samples.iter().skip(c).step_by(ch).copied().collect();
            buf.copy_to_channel(&plane, c as i32).map_err(fail)?;
        }
        let src = self.ctx.create_buffer_source().map_err(fail)?;
        src.set_buffer(Some(&buf));
        src.connect_with_audio_node(&self.ctx.destination()).map_err(fail)?;
        let start = self.next_time.max(self.ctx.current_time());
        src.start_with_when(start).map_err(fail)?;
        self.blocks.push_back((start, self.written, frames as u64));
        self.sources.push_back(src);
        self.written += frames as u64;
        self.next_time = start + frames as f64 / f64::from(self.format.sample_rate);
        Ok(())
    }

    fn played(&mut self) -> Result<u64, MediaError> {
        let now = self.now();
        let rate = f64::from(self.format.sample_rate);
        // Blocks heard to the end are let go.
        while let Some(&(start, first, frames)) = self.blocks.front() {
            if start + frames as f64 / rate > now {
                break;
            }
            self.heard_base = first + frames;
            self.blocks.pop_front();
            self.sources.pop_front();
        }
        Ok(match self.blocks.front() {
            Some(&(start, first, frames)) if now > start => {
                first + (((now - start) * rate) as u64).min(frames)
            }
            _ => self.heard_base,
        })
    }

    fn pause(&mut self) -> Result<(), MediaError> {
        let _ = self.ctx.suspend();
        Ok(())
    }

    fn resume(&mut self) -> Result<(), MediaError> {
        // A browser starts sound only after the person has used the page; until then the
        // clock stands still and the player follows the wall clock.
        let _ = self.ctx.resume();
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        for s in self.sources.drain(..) {
            #[allow(deprecated)]
            let _ = s.stop();
        }
        self.blocks.clear();
        self.written = 0;
        self.heard_base = 0;
        let _ = self.ctx.suspend();
        self.next_time = self.ctx.current_time();
        Ok(())
    }
}

impl Drop for WebAudio {
    fn drop(&mut self) {
        let _ = self.ctx.close();
    }
}

/// The browser's decoders and WebAudio, or its media element ([`mse`]).
struct WebHost {
    /// Plays through the media element rather than WebCodecs.
    element: bool,
}

impl WebHost {
    /// A source played by the media element.
    fn open_element(source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Ok(Box::new(match source {
            MediaSource::File(path) => mse::ElementDecoder::bytes(&read_file(path)?)?,
            MediaSource::Bytes(b) => mse::ElementDecoder::bytes(&b.data)?,
            MediaSource::Url(url) if crate::hls::is_hls(source) => mse::ElementDecoder::hls(url)?,
            MediaSource::Url(url) => mse::ElementDecoder::plain(url)?,
        }))
    }
}

/// A file the person picked or dropped, from the web bridge's file store.
fn read_file(path: &std::path::Path) -> Result<Vec<u8>, MediaError> {
    remedy_ui_bridge_web::files::read(path).ok_or_else(|| {
        MediaError::NotFound(format!(
            "{}: a page reads only files the person picked or dropped (or the app added to its file store)",
            path.display()
        ))
    })
}

impl MediaHost for WebHost {
    fn name(&self) -> &str {
        if self.element {
            "the browser's media element (Media Source Extensions)"
        } else {
            "WebCodecs and WebAudio"
        }
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        if self.element {
            return WebHost::open_element(source);
        }
        let out: Shared = Rc::default();
        let slot: Rc<RefCell<Option<Open>>> = Rc::default();
        match source {
            MediaSource::File(_) | MediaSource::Bytes(_) => {
                let bytes = match source {
                    MediaSource::Bytes(b) => b.data.to_vec(),
                    MediaSource::File(path) => read_file(path)?,
                    MediaSource::Url(_) => Vec::new(),
                };
                let m = mp4::parse(&bytes).map_err(MediaError::Format)?;
                let data: SharedBytes =
                    Rc::new(RefCell::new(Bytes { data: bytes, complete: true, dropped: false }));
                let op = open(data.clone(), m, &out)?;
                let info = op.info.clone();
                *slot.borrow_mut() = Some(op);
                out.borrow_mut().ready = Some(Ok(()));
                Ok(Box::new(WebDecoder { out, open: slot, bytes: data, info }))
            }
            MediaSource::Url(url) if crate::hls::is_hls(source) => {
                Ok(Box::new(hls_web::HlsDecoder::open(url, out)))
            }
            MediaSource::Url(url) => {
                let bytes: SharedBytes = Rc::default();
                fetch_and_open(url.clone(), out.clone(), slot.clone(), bytes.clone());
                Ok(Box::new(WebDecoder { out, open: slot, bytes, info: MediaInfo::default() }))
            }
        }
    }

    fn opens_bytes(&self) -> bool {
        true
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(WebMicrophone::open(format, true)?))
    }

    fn audio_input_with(&self, format: AudioFormat, raw: bool) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(WebMicrophone::open(format, raw)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(WebAudio::new(format, crate::host::Latency::Playback)?))
    }

    fn audio_output_with(
        &self,
        format: AudioFormat,
        latency: crate::host::Latency,
    ) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(WebAudio::new(format, latency)?))
    }
}

/// Whether this page may use WebCodecs: browsers offer it only to secure pages.
fn webcodecs() -> bool {
    get(&js_sys::global(), "isSecureContext").as_bool().unwrap_or(true)
        && has_global("VideoDecoder")
        && has_global("AudioDecoder")
}

/// Whether the page runs in Firefox.
fn firefox() -> bool {
    let agent = get(&get(&js_sys::global(), "navigator"), "userAgent");
    agent.as_string().is_some_and(|a| a.contains("Firefox/"))
}

pub(crate) fn host(backend: crate::WebBackend) -> Result<Arc<dyn MediaHost>, MediaError> {
    let element = match backend {
        crate::WebBackend::WebCodecs => false,
        crate::WebBackend::MediaSource => true,
        // shortcut: Firefox plays through the media element, as a page's own `<video>` does,
        // because Portal saw WebCodecs stay in Buffering there on its HLS. Not reproduced on
        // this crate's own runs (Firefox headless, CPU and GPU renderers, H.264 at 720p and
        // 1080p with B-frames, AAC, a still frame with and without sound, 2 and 4 s TS
        // segments, EVENT without ENDLIST, a stream clock 600 s in, `replace_source` with a
        // `Timeline`, autoplay allowed and blocked): all played. Firefox gets WebCodecs again
        // once a run that stalls is found and fixed.
        _ => !webcodecs() || firefox(),
    };
    if element && !mse::available() {
        return Err(MediaError::Unsupported("this browser has no media element".into()));
    }
    if !element && !has_global("VideoDecoder") && !has_global("AudioDecoder") {
        return Err(MediaError::Unsupported(
            "this browser has no WebCodecs decoders (browsers offer them only to secure pages)".into(),
        ));
    }
    Ok(Arc::new(WebHost { element }))
}

/// The audio worklet that hands the microphone's sound to the page: interleaved frames, about
/// ten milliseconds at a time (a render quantum is 128 frames, under 3 ms; posting each would
/// wake the page four times as often for nothing).
const MIC_WORKLET: &str = r#"
class RemedyMicrophone extends AudioWorkletProcessor {
  constructor() { super(); this.parts = []; this.frames = 0; this.batch = Math.max(128, Math.round(sampleRate / 100)); }
  process(inputs) {
    const input = inputs[0];
    if (input && input.length > 0 && input[0].length > 0) {
      const ch = input.length, n = input[0].length, out = new Float32Array(n * ch);
      for (let c = 0; c < ch; c++) { const p = input[c]; for (let i = 0; i < n; i++) out[i * ch + c] = p[i]; }
      this.parts.push(out); this.frames += n;
      if (this.frames >= this.batch) {
        const all = new Float32Array(this.frames * ch); let at = 0;
        for (const p of this.parts) { all.set(p, at); at += p.length; }
        this.port.postMessage({ channels: ch, samples: all }, [all.buffer]);
        this.parts = []; this.frames = 0;
      }
    }
    return true;
  }
}
registerProcessor("remedy-microphone", RemedyMicrophone);
"#;

/// What the microphone's page callbacks share with its reader.
#[derive(Default)]
struct MicShared {
    /// Sound in the page's format and the asked-for channels, waiting to be read.
    queue: Vec<f32>,
    /// The context's rate once known, when it is not the asked-for one.
    convert: Option<crate::capture::Converter>,
    error: Option<MediaError>,
    /// What to let go when the microphone closes.
    stream: Option<JsValue>,
    ctx: Option<web_sys::AudioContext>,
    url: Option<String>,
    on_message: Option<Closure<dyn FnMut(JsValue)>>,
    closed: bool,
}

impl MicShared {
    fn release(&mut self) {
        if let Some(stream) = self.stream.take()
            && let Ok(tracks) = call0(&stream, "getTracks")
        {
            for t in Array::from(&tracks).iter() {
                let _ = call0(&t, "stop");
            }
        }
        if let Some(ctx) = self.ctx.take() {
            let _ = ctx.close();
        }
        if let Some(url) = self.url.take() {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
        self.on_message = None;
    }
}

fn call0(o: &JsValue, method: &str) -> Result<JsValue, JsValue> {
    let f: js_sys::Function = get(o, method).dyn_into()?;
    f.call0(o)
}

/// The microphone through `getUserMedia` and an audio worklet. The browser asks the person
/// when it opens; until they answer (and the worklet loads) reads give nothing.
struct WebMicrophone {
    shared: Rc<RefCell<MicShared>>,
    format: AudioFormat,
}

// SAFETY: as for the decoder: one thread in a page without shared memory.
unsafe impl Send for WebMicrophone {}

impl WebMicrophone {
    fn open(format: AudioFormat, raw: bool) -> Result<WebMicrophone, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        let devices = get(&get(&js_sys::global(), "navigator"), "mediaDevices");
        if devices.is_undefined() || !has_global("AudioWorkletNode") {
            return Err(MediaError::Unsupported(
                "this page cannot reach a microphone (browsers allow it only on https: or localhost pages)"
                    .into(),
            ));
        }
        let shared: Rc<RefCell<MicShared>> = Rc::default();
        let s = shared.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = start_microphone(devices, format, raw, s.clone()).await {
                let mut m = s.borrow_mut();
                m.error = Some(e);
                m.release();
            }
        });
        Ok(WebMicrophone { shared, format })
    }
}

/// The error a rejected `getUserMedia` stands for.
fn media_error(e: &JsValue) -> MediaError {
    let text = js_error("the microphone", e);
    match get(e, "name").as_string().as_deref() {
        Some("NotAllowedError" | "SecurityError") => MediaError::Denied(text),
        Some("NotFoundError" | "OverconstrainedError") => MediaError::NotFound(text),
        _ => MediaError::Device(text),
    }
}

async fn start_microphone(
    devices: JsValue,
    format: AudioFormat,
    raw: bool,
    shared: Rc<RefCell<MicShared>>,
) -> Result<(), MediaError> {
    // Sound as the microphone hears it: the voice-call processing would bend a sung note.
    let audio = obj(&[
        ("echoCancellation", JsValue::from(!raw)),
        ("noiseSuppression", JsValue::from(!raw)),
        ("autoGainControl", JsValue::from(!raw)),
        ("channelCount", JsValue::from(u32::from(format.channels))),
    ]);
    let constraints = obj(&[("audio", audio.into()), ("video", JsValue::FALSE)]);
    let get_user_media: js_sys::Function = get(&devices, "getUserMedia")
        .dyn_into()
        .map_err(|e| MediaError::Unsupported(js_error("getUserMedia", &e)))?;
    let promise: Promise = get_user_media
        .call1(&devices, &constraints)
        .map_err(|e| media_error(&e))?
        .dyn_into()
        .map_err(|e| MediaError::Device(js_error("getUserMedia", &e)))?;
    let stream = JsFuture::from(promise).await.map_err(|e| media_error(&e))?;
    if shared.borrow().closed {
        shared.borrow_mut().stream = Some(stream);
        shared.borrow_mut().release();
        return Ok(());
    }
    shared.borrow_mut().stream = Some(stream.clone());

    // A context at the asked-for rate converts in the browser; one that will not take the
    // microphone at that rate (Firefox wants the device's) runs at its own and is converted here.
    let opts = web_sys::AudioContextOptions::new();
    opts.set_sample_rate(format.sample_rate as f32);
    let make = |at_rate: bool| -> Result<(web_sys::AudioContext, JsValue), JsValue> {
        let ctx = if at_rate {
            web_sys::AudioContext::new_with_context_options(&opts)?
        } else {
            web_sys::AudioContext::new()?
        };
        let source: js_sys::Function = get(&ctx, "createMediaStreamSource").dyn_into()?;
        match source.call1(&ctx, &stream) {
            Ok(node) => Ok((ctx, node)),
            Err(e) => {
                let _ = ctx.close();
                Err(e)
            }
        }
    };
    let (ctx, source) =
        make(true).or_else(|_| make(false)).map_err(|e| MediaError::Device(js_error("WebAudio", &e)))?;
    let rate = ctx.sample_rate().round() as u32;
    shared.borrow_mut().ctx = Some(ctx.clone());
    if rate != format.sample_rate {
        shared.borrow_mut().convert =
            Some(crate::capture::Converter::new(AudioFormat::new(rate, format.channels), format));
    }

    let parts = Array::of1(&JsValue::from_str(MIC_WORKLET));
    let bag = obj(&[("type", JsValue::from_str("text/javascript"))]);
    let blob = Reflect::construct(&get(&js_sys::global(), "Blob").into(), &Array::of2(&parts, &bag))
        .map_err(|e| MediaError::Device(js_error("the microphone's worklet", &e)))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob.unchecked_into())
        .map_err(|e| MediaError::Device(js_error("the microphone's worklet", &e)))?;
    shared.borrow_mut().url = Some(url.clone());
    let worklet = get(&ctx, "audioWorklet");
    let add: js_sys::Function = get(&worklet, "addModule")
        .dyn_into()
        .map_err(|e| MediaError::Unsupported(js_error("audio worklets", &e)))?;
    let loading: Promise = add
        .call1(&worklet, &JsValue::from_str(&url))
        .map_err(|e| MediaError::Device(js_error("the microphone's worklet", &e)))?
        .dyn_into()
        .map_err(|e| MediaError::Device(js_error("the microphone's worklet", &e)))?;
    JsFuture::from(loading)
        .await
        .map_err(|e| MediaError::Device(js_error("the microphone's worklet", &e)))?;
    if shared.borrow().closed {
        shared.borrow_mut().release();
        return Ok(());
    }

    let node_opts = obj(&[
        ("numberOfInputs", JsValue::from(1)),
        ("numberOfOutputs", JsValue::from(0)),
        ("channelCount", JsValue::from(u32::from(format.channels))),
        ("channelCountMode", JsValue::from_str("explicit")),
        ("channelInterpretation", JsValue::from_str("speakers")),
    ]);
    let node = Reflect::construct(
        &get(&js_sys::global(), "AudioWorkletNode").into(),
        &Array::of3(&ctx, &JsValue::from_str("remedy-microphone"), &node_opts),
    )
    .map_err(|e| MediaError::Device(js_error("the microphone's worklet", &e)))?;
    let weak = Rc::downgrade(&shared);
    let on_message = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let Some(s) = weak.upgrade() else { return };
        let data = get(&event, "data");
        let samples: Float32Array = match get(&data, "samples").dyn_into() {
            Ok(a) => a,
            Err(_) => return,
        };
        let mut m = s.borrow_mut();
        let m = &mut *m;
        match m.convert.as_mut() {
            None => m.queue.extend(samples.to_vec()),
            Some(c) => c.convert(&samples.to_vec(), &mut m.queue),
        }
    });
    let port = get(&node, "port");
    let _ = Reflect::set(&port, &JsValue::from_str("onmessage"), on_message.as_ref());
    shared.borrow_mut().on_message = Some(on_message);
    let connect: js_sys::Function =
        get(&source, "connect").dyn_into().map_err(|e| MediaError::Device(js_error("WebAudio", &e)))?;
    connect.call1(&source, &node).map_err(|e| MediaError::Device(js_error("WebAudio", &e)))?;
    // A context made outside a press may start suspended; the microphone's own permission
    // lets it run.
    let _ = ctx.resume();
    Ok(())
}

impl AudioInput for WebMicrophone {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn period(&self) -> Duration {
        Duration::from_millis(10)
    }

    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError> {
        let mut m = self.shared.borrow_mut();
        if let Some(e) = m.error.take() {
            return Err(e);
        }
        let ch = usize::from(self.format.channels);
        let whole = m.queue.len() - m.queue.len() % ch;
        into.extend(m.queue.drain(..whole));
        Ok(whole / ch)
    }
}

impl Drop for WebMicrophone {
    fn drop(&mut self) {
        let mut m = self.shared.borrow_mut();
        m.closed = true;
        m.release();
    }
}
