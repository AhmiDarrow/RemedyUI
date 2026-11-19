//! Media in a browser page: the browser's own decoders through WebCodecs (`VideoDecoder`,
//! `AudioDecoder`), fed compressed samples by the MP4 reader, and sound through WebAudio,
//! whose clock (what has been heard) the player shows pictures against.
//!
//! The browser decodes asynchronously: samples arrive in its callbacks and the decoder
//! reports `Pending` until they do, waking the player's decoding worker when they come. A
//! source given by URL is fetched without blocking first ([`Decoder::poll_ready`]); a file
//! the user picked or dropped is read from the web bridge's file store.
//!
//! The one unsafe line in this module says the decoder and the sound output may be "sent":
//! a page built without shared memory has a single thread, so nothing is ever sent anywhere.
//! A build with threads (`atomics`) refuses to compile rather than be wrong.

#[cfg(target_feature = "atomics")]
compile_error!("the web media host is for pages with one thread; a threaded build needs workers of its own");

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use js_sys::{Array, Float32Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture,
    PixelFormat, Plane, Sample, VideoInfo, YuvMatrix,
};
use crate::mp4::{self, Kind, Mp4};

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
    #[wasm_bindgen(method, getter, js_name = colorSpace)]
    fn color_space(this: &VideoFrame) -> JsValue;
    #[wasm_bindgen(method, catch, js_name = allocationSize)]
    fn allocation_size(this: &VideoFrame) -> Result<u32, JsValue>;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    fn copy_to(this: &VideoFrame, dest: &Uint8Array) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = close)]
    fn close_frame(this: &VideoFrame);

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

/// The parts that exist once the source is in hand.
struct Open {
    data: Rc<[u8]>,
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
    info: MediaInfo,
}

// SAFETY: a page without shared memory runs everything on its one thread (the build with
// `atomics` does not compile, above), so the decoder never leaves the thread that made it.
unsafe impl Send for WebDecoder {}

fn picture(frame: &VideoFrame, layout: &JsValue, bytes: Vec<u8>) -> Result<Picture, MediaError> {
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
    let color = frame.color_space();
    let full_range = get(&color, "fullRange").as_bool().unwrap_or(false);
    let matrix = match get(&color, "matrix").as_string().as_deref() {
        Some("bt470bg" | "smpte170m") => YuvMatrix::Bt601,
        _ => YuvMatrix::Bt709,
    };
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
            (PixelFormat::Rgba, [Plane { offset: 0, stride }, Plane::default(), Plane::default()], rgba)
        }
        other => {
            return Err(MediaError::Decode(format!(
                "the browser's decoder gave pictures in {other}, which is not shown"
            )));
        }
    };
    Ok(Picture { width, height, format, planes, matrix, full_range, data: Arc::from(data) })
}

fn video_output(out: &Shared, frame: VideoFrame) {
    let (seq, generation) = {
        let mut o = out.borrow_mut();
        let seq = o.next_copy;
        o.next_copy += 1;
        o.queue.push_back(Slot::Copying(seq));
        (seq, o.generation)
    };
    let out = out.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let at = micros(frame.timestamp());
        let duration = frame.duration().map(micros);
        let copied = async {
            let size = frame.allocation_size().map_err(|e| js_error("sizing a picture", &e))?;
            let buf = Uint8Array::new_with_length(size);
            let promise = frame.copy_to(&buf).map_err(|e| js_error("copying a picture", &e))?;
            let layout = JsFuture::from(promise).await.map_err(|e| js_error("copying a picture", &e))?;
            picture(&frame, &layout, buf.to_vec()).map_err(|e| e.to_string())
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
fn open(data: Rc<[u8]>, out: &Shared) -> Result<Open, MediaError> {
    let mp4 = mp4::parse(&data).map_err(MediaError::Format)?;
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
        video: video.as_ref().and_then(|(i, ..)| mp4.tracks.get(*i)).and_then(|t| match t.kind {
            Kind::Video { width, height } => {
                let secs = t.length().as_secs_f64();
                let frame_rate = (secs > 0.0).then(|| t.samples.len() as f64 / secs);
                Some(VideoInfo { width, height, frame_rate })
            }
            Kind::Audio { .. } => None,
        }),
        audio: audio.as_ref().and_then(|(i, ..)| mp4.tracks.get(*i)).and_then(|t| match t.kind {
            Kind::Audio { sample_rate, channels } => Some(AudioFormat { sample_rate, channels }),
            Kind::Video { .. } => None,
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
        let bytes = self.data.get(start..start + s.size as usize)?;
        let us = |d: Duration| JsValue::from_f64(d.as_secs_f64() * 1_000_000.0);
        Some(obj(&[
            ("type", JsValue::from_str(if s.sync { "key" } else { "delta" })),
            ("timestamp", us(t.time(s.pts))),
            ("duration", us(t.time(i64::from(s.duration)))),
            ("data", Uint8Array::from(bytes).into()),
        ]))
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
                self.video.as_ref().is_some_and(|(_, d, _)| d.decode_queue_size() >= 2)
            } else {
                self.audio.as_ref().is_some_and(|(_, d, _)| d.decode_queue_size() >= 2)
            };
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
}

impl Drop for WebDecoder {
    fn drop(&mut self) {
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

/// Fetches `url` without blocking, then opens it.
fn fetch_and_open(url: String, out: Shared, slot: Rc<RefCell<Option<Open>>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let fetched = async {
            let win = web_sys::window().ok_or_else(|| MediaError::Unsupported("no browser window".into()))?;
            let resp = JsFuture::from(win.fetch_with_str(&url))
                .await
                .map_err(|e| MediaError::NotFound(js_error(&url, &e)))?;
            let resp: web_sys::Response = resp.unchecked_into();
            if !resp.ok() {
                return Err(MediaError::NotFound(format!("{url}: the server answered {}", resp.status())));
            }
            let buf = resp.array_buffer().map_err(|e| MediaError::NotFound(js_error(&url, &e)))?;
            let buf = JsFuture::from(buf).await.map_err(|e| MediaError::NotFound(js_error(&url, &e)))?;
            Ok(Uint8Array::new(&buf).to_vec())
        }
        .await;
        let result = fetched.and_then(|bytes| open(Rc::from(bytes), &out));
        let ready = match result {
            Ok(op) => {
                *slot.borrow_mut() = Some(op);
                Ok(())
            }
            Err(e) => Err(e),
        };
        let mut o = out.borrow_mut();
        o.ready = Some(ready);
        o.wake();
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
}

// SAFETY: as for the decoder: one thread in a page without shared memory.
unsafe impl Send for WebAudio {}

impl WebAudio {
    fn new(format: AudioFormat) -> Result<WebAudio, MediaError> {
        let opts = web_sys::AudioContextOptions::new();
        opts.set_sample_rate(format.sample_rate as f32);
        let ctx = web_sys::AudioContext::new_with_context_options(&opts)
            .or_else(|_| web_sys::AudioContext::new())
            .map_err(|e| MediaError::Device(js_error("WebAudio", &e)))?;
        // Paused until the player resumes it.
        let _ = ctx.suspend();
        let next_time = ctx.current_time();
        Ok(WebAudio {
            ctx,
            format,
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
        AUDIO_BUFFER
    }

    fn room(&mut self) -> Result<usize, MediaError> {
        let queued = (self.next_time - self.ctx.current_time()).max(0.0);
        let room = (AUDIO_BUFFER.as_secs_f64() - queued).max(0.0);
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

/// The browser's decoders and WebAudio.
struct WebHost;

impl MediaHost for WebHost {
    fn name(&self) -> &str {
        "WebCodecs and WebAudio"
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let out: Shared = Rc::default();
        let slot: Rc<RefCell<Option<Open>>> = Rc::default();
        match source {
            MediaSource::File(path) => {
                let bytes = remedy_ui_bridge_web::files::read(path).ok_or_else(|| {
                    MediaError::NotFound(format!(
                        "{}: a page reads only files the person picked or dropped (or the app added to its file store)",
                        path.display()
                    ))
                })?;
                let op = open(Rc::from(bytes), &out)?;
                let info = op.info.clone();
                *slot.borrow_mut() = Some(op);
                out.borrow_mut().ready = Some(Ok(()));
                Ok(Box::new(WebDecoder { out, open: slot, info }))
            }
            MediaSource::Url(url) => {
                fetch_and_open(url.clone(), out.clone(), slot.clone());
                Ok(Box::new(WebDecoder { out, open: slot, info: MediaInfo::default() }))
            }
        }
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(WebMicrophone::open(format)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(WebAudio::new(format)?))
    }
}

pub(crate) fn host() -> Result<Arc<dyn MediaHost>, MediaError> {
    if !has_global("VideoDecoder") && !has_global("AudioDecoder") {
        return Err(MediaError::Unsupported("this browser has no WebCodecs decoders".into()));
    }
    Ok(Arc::new(WebHost))
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
    fn open(format: AudioFormat) -> Result<WebMicrophone, MediaError> {
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
            if let Err(e) = start_microphone(devices, format, s.clone()).await {
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
    shared: Rc<RefCell<MicShared>>,
) -> Result<(), MediaError> {
    // Sound as the microphone hears it: the voice-call processing would bend a sung note.
    let audio = obj(&[
        ("echoCancellation", JsValue::FALSE),
        ("noiseSuppression", JsValue::FALSE),
        ("autoGainControl", JsValue::FALSE),
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
