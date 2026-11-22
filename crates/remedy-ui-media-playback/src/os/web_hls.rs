//! HLS in a page: the crate's own reader ([`crate::hls`]) takes the playlist and its MPEG-TS
//! segments apart, the page fetches what it asks for, and the browser's WebCodecs decoders
//! decode the H.264 access units (Annex B, so configured without a description) and the AAC
//! or MP3 frames. Decoded samples come out through the MP4 path's output handling.

use super::{
    AudioDecoder, EncodedAudioChunk, EncodedVideoChunk, Shared, Slot, VideoDecoder, audio_output, call,
    has_global, js_error, obj, video_output,
};
use crate::hls::{
    AudioCodec, AudioConfig, Elementary, Hls, Unit, VideoConfig, WAIT_SLICE, Want, WantKind, answered,
    first_byte, load, max_fetch, quiet, timed_out,
};
use crate::host::{Decoder, MediaError, MediaInfo, Sample};
use js_sys::{Object, Promise, Uint8Array};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// The browser's decoders for the compressed streams of an HLS source.
pub(super) struct WebElementary {
    out: Shared,
    video: Option<(VideoDecoder, Object)>,
    audio: Option<(AudioDecoder, Object)>,
    _callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
}

// SAFETY: as for the MP4 decoder: one thread in a page without shared memory.
unsafe impl Send for WebElementary {}

fn us(d: Duration) -> JsValue {
    JsValue::from_f64(d.as_secs_f64() * 1_000_000.0)
}

impl WebElementary {
    pub(super) fn new(out: Shared) -> WebElementary {
        WebElementary { out, video: None, audio: None, _callbacks: Vec::new() }
    }

    /// Whether the decoder `unit` goes to is still busy with the last two (as the MP4 feed).
    pub(super) fn busy(&self, unit: &Unit) -> bool {
        match unit {
            Unit::Video { .. } => self.video.as_ref().is_some_and(|(d, _)| d.decode_queue_size() >= 2),
            Unit::Audio { .. } => self.audio.as_ref().is_some_and(|(d, _)| d.decode_queue_size() >= 2),
        }
    }

    /// Everything pushed has come out and been read (after [`Elementary::end`]).
    pub(super) fn drained(&self) -> bool {
        let o = self.out.borrow();
        o.flushed && o.queue.is_empty()
    }
}

impl Elementary for WebElementary {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        let error_cb = {
            let out = self.out.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
                out.borrow_mut().fail(MediaError::Decode(js_error("the browser's decoder failed", &e)));
            })
        };
        let dequeue = {
            let out = self.out.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |_| out.borrow_mut().wake())
        };
        if let Some(v) = video {
            if !has_global("VideoDecoder") {
                return Err(MediaError::Unsupported("this browser has no WebCodecs video decoder".into()));
            }
            let out = self.out.clone();
            let output =
                Closure::<dyn FnMut(JsValue)>::new(move |f: JsValue| video_output(&out, f.unchecked_into()));
            let init = obj(&[("output", output.as_ref().clone()), ("error", error_cb.as_ref().clone())]);
            let dec = VideoDecoder::new(&init)
                .map_err(|e| MediaError::Unsupported(js_error("video decoding", &e)))?;
            let _ = js_sys::Reflect::set(&dec, &"ondequeue".into(), dequeue.as_ref());
            // No description: the access units come as an Annex B stream with their SPS and PPS.
            let config = obj(&[
                ("codec", JsValue::from_str(&v.codec)),
                ("codedWidth", JsValue::from(v.width)),
                ("codedHeight", JsValue::from(v.height)),
                ("optimizeForLatency", JsValue::TRUE),
            ]);
            dec.configure(&config)
                .map_err(|e| MediaError::Unsupported(js_error(&format!("decoding {}", v.codec), &e)))?;
            self._callbacks.push(output);
            self.video = Some((dec, config));
        }
        if let Some(a) = audio {
            if !has_global("AudioDecoder") {
                return Err(MediaError::Unsupported("this browser has no WebCodecs sound decoder".into()));
            }
            let out = self.out.clone();
            let output = Closure::<dyn FnMut(JsValue)>::new(move |d: JsValue| {
                let data: super::AudioData = d.unchecked_into();
                if let Err(e) = audio_output(&out, &data) {
                    out.borrow_mut().fail(e);
                }
                data.close_data();
            });
            let init = obj(&[("output", output.as_ref().clone()), ("error", error_cb.as_ref().clone())]);
            let dec = AudioDecoder::new(&init)
                .map_err(|e| MediaError::Unsupported(js_error("sound decoding", &e)))?;
            let _ = js_sys::Reflect::set(&dec, &"ondequeue".into(), dequeue.as_ref());
            let mut pairs = vec![
                ("codec", JsValue::from_str(&a.codec_string)),
                ("sampleRate", JsValue::from(a.sample_rate)),
                ("numberOfChannels", JsValue::from(a.channels)),
            ];
            if let AudioCodec::Aac { asc } = &a.codec {
                pairs.push(("description", Uint8Array::from(asc.as_slice()).into()));
            }
            let config = obj(&pairs);
            dec.configure(&config).map_err(|e| {
                MediaError::Unsupported(js_error(&format!("decoding {}", a.codec_string), &e))
            })?;
            self._callbacks.push(output);
            self.audio = Some((dec, config));
        }
        self._callbacks.push(error_cb);
        self._callbacks.push(dequeue);
        Ok(())
    }

    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        match unit {
            Unit::Video { pts, keyframe, data, .. } => {
                let Some((dec, _)) = &self.video else { return Ok(()) };
                let init = obj(&[
                    ("type", JsValue::from_str(if keyframe { "key" } else { "delta" })),
                    ("timestamp", us(pts)),
                    ("data", Uint8Array::from(data.as_slice()).into()),
                ]);
                let chunk = EncodedVideoChunk::new(&init)
                    .map_err(|e| MediaError::Decode(js_error("a picture", &e)))?;
                dec.decode(&chunk).map_err(|e| MediaError::Decode(js_error("decoding a picture", &e)))
            }
            Unit::Audio { pts, data } => {
                let Some((dec, _)) = &self.audio else { return Ok(()) };
                let init = obj(&[
                    ("type", JsValue::from_str("key")),
                    ("timestamp", us(pts)),
                    ("data", Uint8Array::from(data.as_slice()).into()),
                ]);
                let chunk =
                    EncodedAudioChunk::new(&init).map_err(|e| MediaError::Decode(js_error("sound", &e)))?;
                dec.decode(&chunk).map_err(|e| MediaError::Decode(js_error("decoding sound", &e)))
            }
        }
    }

    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        let mut o = self.out.borrow_mut();
        if let Some(e) = o.error.take() {
            return Err(e);
        }
        if matches!(o.queue.front(), Some(Slot::Ready(_)))
            && let Some(Slot::Ready(s)) = o.queue.pop_front()
        {
            return Ok(Some(s));
        }
        Ok(None)
    }

    fn end(&mut self) -> Result<(), MediaError> {
        let flushes: Vec<Promise> = self
            .video
            .as_ref()
            .and_then(|(d, _)| d.flush().ok())
            .into_iter()
            .chain(self.audio.as_ref().and_then(|(d, _)| d.flush().ok()))
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
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        {
            let mut o = self.out.borrow_mut();
            o.generation += 1;
            o.queue.clear();
            o.flushed = false;
            o.error = None;
        }
        let seeking = |e: JsValue| MediaError::Decode(js_error("seeking", &e));
        if let Some((d, config)) = &self.video {
            d.reset().map_err(seeking)?;
            d.configure(config).map_err(seeking)?;
        }
        if let Some((d, config)) = &self.audio {
            d.reset().map_err(seeking)?;
            d.configure(config).map_err(seeking)?;
        }
        Ok(())
    }
}

impl Drop for WebElementary {
    fn drop(&mut self) {
        if let Some((d, _)) = &self.video {
            let _ = d.close();
        }
        if let Some((d, _)) = &self.audio {
            let _ = d.close();
        }
    }
}

/// What the page's fetches share with the decoder: the reader, whether a fetch is under way,
/// and when the last playlist and segment fetches ended (the page's clock, in milliseconds).
pub(super) struct Reader {
    pub(super) hls: Hls,
    fetching: bool,
    fetched: (f64, f64),
}

impl Reader {
    pub(super) fn new(url: &str) -> Rc<RefCell<Reader>> {
        let now = js_sys::Date::now();
        Rc::new(RefCell::new(Reader { hls: Hls::new(url), fetching: false, fetched: (now, now) }))
    }
}

/// Starts fetching what `reader` wants next, unless a fetch is under way; `done` hears how
/// the delivery went. A fetch not due yet is waited for in short slices, asking again after
/// each what is wanted (a seek or a segment now wanted goes first).
pub(super) fn fetch_next(reader: &Rc<RefCell<Reader>>, done: Rc<dyn Fn(Result<(), MediaError>)>) {
    {
        let mut r = reader.borrow_mut();
        if r.fetching || r.hls.wanted().is_none() {
            return;
        }
        r.fetching = true;
    }
    let reader = reader.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let want = loop {
            let due = reader.borrow().due();
            match due {
                None => {
                    reader.borrow_mut().fetching = false;
                    return;
                }
                Some((want, wait)) if wait.is_zero() => break want,
                Some((_, wait)) => sleep(wait.min(WAIT_SLICE)).await,
            }
        };
        let got = fetch_all(&want.url).await;
        let mut r = reader.borrow_mut();
        match want.kind {
            WantKind::Playlist => r.fetched.0 = js_sys::Date::now(),
            WantKind::Segment(_) => r.fetched.1 = js_sys::Date::now(),
        }
        let delivered = r.hls.deliver(&want, got);
        r.fetching = false;
        drop(r);
        done(delivered);
    });
}

impl Reader {
    /// What is wanted now, and how long until it is due.
    fn due(&self) -> Option<(Want, Duration)> {
        let want = self.hls.wanted()?;
        let since = Duration::from_secs_f64(((js_sys::Date::now() - self.last(&want)) / 1000.0).max(0.0));
        let wait = want.after.saturating_sub(since);
        Some((want, wait))
    }
}

impl Reader {
    /// When the last fetch of `want`'s kind ended.
    fn last(&self, want: &Want) -> f64 {
        match want.kind {
            WantKind::Playlist => self.fetched.0,
            WantKind::Segment(_) => self.fetched.1,
        }
    }
}

/// An HLS source played in a page: the reader's wants fetched one at a time, its units fed to
/// the browser's decoders while they are not busy.
pub(super) struct HlsDecoder {
    out: Shared,
    reader: Rc<RefCell<Reader>>,
    el: Option<WebElementary>,
    /// A unit taken from the reader while its decoder was busy.
    pending: Option<Unit>,
    ending: bool,
    info: MediaInfo,
}

// SAFETY: as for the MP4 decoder: one thread in a page without shared memory.
unsafe impl Send for HlsDecoder {}

/// Waits `d` on the page's timer.
async fn sleep(d: Duration) {
    if d.is_zero() {
        return;
    }
    let ms = JsValue::from_f64(d.as_secs_f64() * 1000.0);
    let p = Promise::new(&mut |resolve, _| {
        let _ = call(&js_sys::global(), "setTimeout", &[resolve.into(), ms.clone()]);
    });
    let _ = JsFuture::from(p).await;
}

/// Fetches `url` whole. Aborted, as a timeout the reader retries, when no answer comes for
/// `first_byte()`, the body goes quiet for `quiet()` (a stalled download, started again), or
/// the whole takes `load()`.
async fn fetch_all(url: &str) -> Result<Vec<u8>, MediaError> {
    let not_found = |e: JsValue| MediaError::NotFound(js_error(url, &e));
    let win = web_sys::window().ok_or_else(|| MediaError::Unsupported("no browser window".into()))?;
    let abort = web_sys::AbortController::new().map_err(not_found)?;
    let init = web_sys::RequestInit::new();
    init.set_signal(Some(&abort.signal()));
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    // When bytes last came (the start, until the answer) and how long they may stop; NaN once done.
    let watch = Rc::new(Cell::new((js_sys::Date::now(), first_byte())));
    let timed = Rc::new(Cell::new(None::<&'static str>));
    {
        let (watch, timed, abort, started) =
            (watch.clone(), timed.clone(), abort.clone(), js_sys::Date::now());
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                sleep(WAIT_SLICE * 2).await;
                let (last, limit) = watch.get();
                let now = js_sys::Date::now();
                let why = if last.is_nan() {
                    return;
                } else if now - started > ms(load()) {
                    "the download took too long"
                } else if now - last > ms(limit) {
                    "nothing arrived"
                } else {
                    continue;
                };
                timed.set(Some(why));
                abort.abort();
                return;
            }
        });
    }
    let got = async {
        let resp: web_sys::Response = JsFuture::from(win.fetch_with_str_and_init(url, &init))
            .await
            .map_err(not_found)?
            .unchecked_into();
        if !resp.ok() {
            return Err(answered(url, u32::from(resp.status())));
        }
        watch.set((js_sys::Date::now(), quiet()));
        let Some(body) = resp.body() else { return Ok(Vec::new()) };
        let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().unchecked_into();
        let mut out = Vec::new();
        loop {
            let chunk = JsFuture::from(reader.read()).await.map_err(not_found)?;
            if js_sys::Reflect::get(&chunk, &"done".into()).is_ok_and(|d| d.is_truthy()) {
                return Ok(out);
            }
            let value = js_sys::Reflect::get(&chunk, &"value".into()).map_err(not_found)?;
            // Copied once, straight into `out` (`to_vec` then `extend` copied each chunk twice).
            let bytes = value.unchecked_into::<Uint8Array>();
            let at = out.len();
            out.resize(at + bytes.length() as usize, 0);
            if let Some(to) = out.get_mut(at..) {
                bytes.copy_to(to);
            }
            if out.len() > max_fetch() {
                return Err(MediaError::Format(format!("{url}: over {} bytes", max_fetch())));
            }
            watch.set((js_sys::Date::now(), quiet()));
        }
    }
    .await;
    watch.set((f64::NAN, quiet()));
    got.map_err(|e| timed.get().map_or(e, |why| timed_out(url, why)))
}

impl HlsDecoder {
    pub(super) fn open(url: &str, out: Shared) -> HlsDecoder {
        let reader = Reader::new(url);
        let mut dec =
            HlsDecoder { out, reader, el: None, pending: None, ending: false, info: MediaInfo::default() };
        dec.fetch();
        dec
    }

    /// Starts fetching what the reader wants next ([`fetch_next`]).
    fn fetch(&mut self) {
        let out = self.out.clone();
        fetch_next(
            &self.reader,
            Rc::new(move |delivered| match delivered {
                Ok(()) => out.borrow_mut().wake(),
                Err(e) => out.borrow_mut().fail(e),
            }),
        );
    }

    /// Sets the decoders up once the reader knows the streams, feeds them, and fetches more.
    fn pump(&mut self) -> Result<(), MediaError> {
        if self.el.is_none() {
            let r = self.reader.borrow();
            if r.hls.started() {
                let mut el = WebElementary::new(self.out.clone());
                el.configure(r.hls.video(), r.hls.audio())?;
                self.el = Some(el);
            }
        }
        if self.el.is_some() {
            // A growing playlist's length grows.
            self.info = self.reader.borrow().hls.info();
        }
        if let Some(el) = self.el.as_mut() {
            loop {
                let unit = match self.pending.take() {
                    Some(u) => u,
                    None => match self.reader.borrow_mut().hls.next_unit() {
                        Some(u) => u,
                        None => break,
                    },
                };
                if el.busy(&unit) {
                    self.pending = Some(unit);
                    break;
                }
                el.push(unit)?;
            }
            if self.pending.is_none() && !self.ending && self.reader.borrow().hls.ended() {
                self.ending = true;
                el.end()?;
            }
        }
        self.fetch();
        Ok(())
    }
}

impl Decoder for HlsDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MediaError>> {
        self.out.borrow_mut().waker = Some(cx.waker().clone());
        if let Err(e) = self.pump() {
            return Poll::Ready(Err(e));
        }
        if let Some(e) = self.out.borrow_mut().error.take() {
            return Poll::Ready(Err(e));
        }
        if self.el.is_some() { Poll::Ready(Ok(())) } else { Poll::Pending }
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        // The browser decodes asynchronously; nothing can wait for it on the page's thread.
        match self.poll_read(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(MediaError::Decode(
                "the browser's decoder answers later; this decoder is read with poll_read".into(),
            )),
        }
    }

    fn poll_read(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Sample>, MediaError>> {
        self.out.borrow_mut().waker = Some(cx.waker().clone());
        if let Some(el) = self.el.as_mut()
            && let Some(s) = el.pull().transpose()
        {
            return Poll::Ready(s.map(Some));
        }
        if let Err(e) = self.pump() {
            return Poll::Ready(Err(e));
        }
        match self.el.as_mut() {
            Some(el) => match el.pull() {
                Ok(Some(s)) => Poll::Ready(Ok(Some(s))),
                Err(e) => Poll::Ready(Err(e)),
                Ok(None) if self.ending && el.drained() => Poll::Ready(Ok(None)),
                Ok(None) => Poll::Pending,
            },
            None => Poll::Pending,
        }
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        let el = self.el.as_mut().ok_or_else(|| MediaError::Decode("the source is not open yet".into()))?;
        self.reader.borrow_mut().hls.seek(to);
        el.flush()?;
        self.pending = None;
        self.ending = false;
        self.fetch();
        Ok(())
    }

    /// Also keeps the fetches going while nothing is read (paused), so a growing playlist
    /// is asked again and its length grows.
    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        if self.el.is_some() {
            self.info = self.reader.borrow().hls.info();
        }
        self.fetch();
        Some(self.reader.borrow().hls.buffered())
    }
}
