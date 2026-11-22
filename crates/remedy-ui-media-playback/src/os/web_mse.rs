//! Media in a page through a hidden `<video>` element, for pages that may not use WebCodecs
//! (browsers offer it only to secure pages, and a page at a machine's LAN address over plain
//! `http:` is not one), or when the app asks ([`WebBackend::MediaSource`](crate::WebBackend)).
//!
//! The element plays: its sound is the sound, and its clock the clock ([`Playout`]). HLS is
//! read by the crate's own reader ([`crate::hls`]), its MPEG-TS units put back together as
//! fragmented MP4 ([`fmp4`]) and appended to a Media Source Extensions `SourceBuffer` per
//! stream, as far ahead of the playhead as [`MediaLimits::mse_ahead`](crate::MediaLimits)
//! says; what has played is let go past `mse_behind`. Any other source is the element's own
//! (`src`), a file the person picked through a blob URL.
//!
//! Each frame the element shows (`requestVideoFrameCallback`) becomes a picture: on the GPU
//! renderer the element itself, uploaded from as it is ([`GpuTexture::WebVideoFrame`] takes any
//! image source); on the CPU renderer its pixels, drawn into a 2D canvas and read back.

use super::hls_web::{Reader, fetch_next};
use super::{call, get, has_global, js_error, obj};
use crate::hls::{Unit, fmp4};
use crate::host::{
    Decoder, MediaError, MediaInfo, Picture, PixelFormat, Playout, Sample, VideoInfo, YuvMatrix,
};
use js_sys::{Array, Reflect, Uint8Array};
use remedy_ui_paint::{CpuPixels, GpuPicture, GpuTexture, Plane};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use wasm_bindgen::JsCast;
use wasm_bindgen::convert::IntoWasmAbi;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

fn set(o: &JsValue, k: &str, v: impl Into<JsValue>) {
    let _ = Reflect::set(o, &JsValue::from_str(k), &v.into());
}

fn num(o: &JsValue, k: &str) -> f64 {
    get(o, k).as_f64().unwrap_or(0.0)
}

/// `new class(...args)`.
fn construct(class: JsValue, args: &Array) -> Result<JsValue, JsValue> {
    let f: js_sys::Function = class.unchecked_into();
    Reflect::construct(&f, args)
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

fn duration(s: f64) -> Duration {
    if s.is_finite() { Duration::from_secs_f64(s.max(0.0)) } else { Duration::ZERO }
}

/// A `TimeRanges` as media time.
fn ranges(r: &JsValue) -> Vec<std::ops::Range<Duration>> {
    let n = num(r, "length") as u32;
    (0..n)
        .filter_map(|i| {
            let start = call(r, "start", &[i.into()]).ok()?.as_f64()?;
            let end = call(r, "end", &[i.into()]).ok()?.as_f64()?;
            Some(duration(start)..duration(end))
        })
        .collect()
}

/// The Media Source Extensions constructor this browser has: `MediaSource`, or Safari's
/// `ManagedMediaSource` (the only one on an iPhone).
fn media_source_class() -> Option<JsValue> {
    ["MediaSource", "ManagedMediaSource"]
        .into_iter()
        .map(|n| get(&js_sys::global(), n))
        .find(|c| c.is_function() && get(c, "isTypeSupported").is_function())
}

/// Whether a browser's media element can play a page without WebCodecs: always, in a browser.
pub(super) fn available() -> bool {
    has_global("HTMLVideoElement")
}

/// How long an element in its picture-in-picture window waits, once its decoder is gone, for
/// the next stream of the same player (a quality or sound switch, a skip) to take it, so the
/// window stays open across the switch as a page's own `<video>` does.
const PIP_HANDBACK: Duration = Duration::from_secs(2);

/// An element left in its picture-in-picture window by the decoder before: its blob URL, and
/// the timer that lets it go when no decoder takes it.
struct Spare {
    video: JsValue,
    url: Option<String>,
    timer: f64,
    _let_go: Closure<dyn FnMut()>,
}

thread_local! {
    static SPARE: RefCell<Option<Spare>> = const { RefCell::new(None) };
}

/// Takes the element out of the page, its source let go.
fn let_go(video: &JsValue, url: Option<String>) {
    let _ = call(video, "pause", &[]);
    let _ = call(video, "removeAttribute", &["src".into()]);
    let _ = call(video, "load", &[]);
    let _ = call(video, "remove", &[]);
    if let Some(url) = url {
        let _ = call(&get(&js_sys::global(), "URL"), "revokeObjectURL", &[url.into()]);
    }
}

fn in_pip(video: &JsValue) -> bool {
    let el = get(&get(&js_sys::global(), "document"), "pictureInPictureElement");
    !el.is_null() && !el.is_undefined() && js_sys::Object::is(&el, video)
}

/// What the element's events share with the decoder and the playout.
struct State {
    video: JsValue,
    /// Pictures shown since the decoder last read.
    frames: VecDeque<Sample>,
    waker: Option<Waker>,
    error: Option<MediaError>,
    ended: bool,
    /// The element knows the length and size (`loadedmetadata`).
    meta: bool,
    /// Asked to play (the element may still be waiting for data).
    playing: bool,
    /// The page has not been allowed sound yet: played muted until the volume is set.
    policy_muted: bool,
    /// Where the last seek went: the picture shown there is the one for that place, though
    /// its frame may start a little before it (the player drops pictures before a seek's
    /// target).
    floor: f64,
    closed: bool,
}

impl State {
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

type Shared = Rc<RefCell<State>>;

/// The frame the element shows, drawn into a 2D canvas and read back as RGBA (what a CPU
/// renderer takes); `None` when the browser cannot (no frame yet, a canvas refused).
fn draw_rgba(video: &JsValue, (w, h): (u32, u32), slot: &mut Option<(JsValue, JsValue)>) -> Option<Vec<u8>> {
    if slot.is_none() {
        let class = get(&js_sys::global(), "OffscreenCanvas");
        let canvas = if class.is_function() {
            construct(class, &Array::of2(&w.into(), &h.into())).ok()?
        } else {
            call(&get(&js_sys::global(), "document"), "createElement", &["canvas".into()]).ok()?
        };
        let ctx =
            call(&canvas, "getContext", &["2d".into(), obj(&[("willReadFrequently", JsValue::TRUE)]).into()])
                .ok()?;
        if ctx.is_null() || ctx.is_undefined() {
            return None;
        }
        *slot = Some((canvas, ctx));
    }
    let (canvas, ctx) = slot.as_ref()?;
    if num(canvas, "width") as u32 != w || num(canvas, "height") as u32 != h {
        set(canvas, "width", w);
        set(canvas, "height", h);
    }
    call(ctx, "drawImage", &[video.clone(), 0.into(), 0.into()]).ok()?;
    let image = call(ctx, "getImageData", &[0.into(), 0.into(), w.into(), h.into()]).ok()?;
    Some(Uint8Array::new(&get(&image, "data")).to_vec())
}

/// The element while no surface shows it: in the page and over everything, so the browser
/// composites (and calls back for) its frames, a covered element's being skipped; but unseen
/// and out of the way.
const HIDDEN: &str = "position:fixed;left:0;top:0;width:2px;height:2px;opacity:0;pointer-events:none;z-index:2147483647";

/// Shows `video` under the window's canvas at `at` (window logical pixels, which are the CSS
/// pixels of the canvas's box), or out of the way again for `None`.
fn place_element(video: &JsValue, at: Option<remedy_ui_paint::PagePlacement>) {
    let doc = get(&js_sys::global(), "document");
    let style = get(video, "style");
    let Some(at) = at else {
        if get(&style, "cssText").as_string().as_deref() != Some(HIDDEN) {
            set(&style, "cssText", HIDDEN);
            let _ = call(&get(&doc, "body"), "appendChild", std::slice::from_ref(video));
        }
        return;
    };
    // shortcut: the first window's canvas; a page with several windows shows it in that one.
    let Ok(canvas) = call(&doc, "querySelector", &["[data-remedy-window] canvas[data-raw-handle]".into()]) else {
        return;
    };
    let surface = get(&canvas, "parentElement");
    if surface.is_null() || surface.is_undefined() {
        return;
    }
    // Under the canvases (first in their box), where the window leaves the surface clear.
    if !js_sys::Object::is(&get(video, "parentElement"), &surface) {
        let _ = call(&surface, "insertBefore", &[video.clone(), get(&surface, "firstChild")]);
    }
    let (b, v, c) = (at.bounds, at.visible, at.background);
    let css = format!(
        "position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;object-fit:contain;background:rgba({},{},{},{});pointer-events:none;clip-path:inset({}px {}px {}px {}px)",
        b.min.x,
        b.min.y,
        b.width(),
        b.height(),
        (c.r * 255.0).round(),
        (c.g * 255.0).round(),
        (c.b * 255.0).round(),
        c.a,
        (v.min.y - b.min.y).max(0.0),
        (b.max.x - v.max.x).max(0.0),
        (b.max.y - v.max.y).max(0.0),
        (v.min.x - b.min.x).max(0.0),
    );
    if get(&style, "cssText").as_string().as_deref() != Some(css.as_str()) {
        set(&style, "cssText", css.as_str());
    }
}

/// The element as a picture: the page shows it itself under the window (as any page shows its
/// video, decoded and composited by the browser); a renderer that draws it instead uploads the
/// frame it shows (GPU) or reads its pixels (CPU).
struct ElementFrame {
    video: JsValue,
    width: u32,
    height: u32,
}

// SAFETY: a page without shared memory runs everything on its one thread (web.rs refuses a
// build with `atomics`), so the element never leaves the thread that made it.
unsafe impl Send for ElementFrame {}
// SAFETY: as above.
unsafe impl Sync for ElementFrame {}

impl GpuPicture for ElementFrame {
    fn texture(&self) -> GpuTexture {
        GpuTexture::WebVideoFrame { frame: (&self.video).into_abi(), width: self.width, height: self.height }
    }

    fn shows_itself(&self) -> bool {
        true
    }

    fn place(&self, at: Option<remedy_ui_paint::PagePlacement>) {
        place_element(&self.video, at);
    }

    fn read_back(&self) -> Option<CpuPixels> {
        let data = draw_rgba(&self.video, (self.width, self.height), &mut None)?;
        let stride = self.width as usize * 4;
        Some(CpuPixels {
            format: PixelFormat::Rgba,
            planes: [Plane { offset: 0, stride }, Plane::default(), Plane::default()],
            data: Arc::from(data),
        })
    }
}

/// How many shown pictures wait for the decoder to read them. Each is the element's frame at
/// the time it showed (or, on the GPU, the element itself, whose frame is always the newest),
/// so an older one waiting is only ever replaced by a newer one; a few cover a busy turn.
const WAITING_FRAMES: usize = 4;

/// The picture the element shows now, at `at` in the media.
fn shown(s: &Shared, at: f64) {
    let mut st = s.borrow_mut();
    if st.closed {
        return;
    }
    let (w, h) = (num(&st.video, "videoWidth") as u32, num(&st.video, "videoHeight") as u32);
    // While seeking, a frame from before the seek may still come: the one there follows.
    if w == 0 || h == 0 || get(&st.video, "seeking").is_truthy() {
        return;
    }
    // The element itself, on either renderer: a surface has the page show it, and nothing
    // copies its pixels per frame.
    let frame = Arc::new(ElementFrame { video: st.video.clone(), width: w, height: h });
    let picture = Picture::on_gpu(w, h, YuvMatrix::Bt709, false, frame);
    let at = duration(at.max(st.floor));
    st.frames.push_back(Sample::Video { at, duration: None, picture });
    while st.frames.len() > WAITING_FRAMES {
        st.frames.pop_front();
    }
    st.wake();
}

/// The element's frame callbacks (`requestVideoFrameCallback`, or each animation frame where a
/// browser lacks it), asked again after each until the decoder goes.
type FrameCallback = Rc<RefCell<Option<Closure<dyn FnMut(JsValue, JsValue)>>>>;

struct Frames {
    callback: FrameCallback,
    /// The pending request's handle, and whether it is a video frame request.
    handle: Rc<Cell<(f64, bool)>>,
}

impl Frames {
    fn start(s: &Shared) -> Frames {
        let video = s.borrow().video.clone();
        let rvfc = get(&video, "requestVideoFrameCallback").is_function();
        let callback: FrameCallback = Rc::default();
        let handle = Rc::new(Cell::new((0.0, rvfc)));
        let request = {
            let (video, handle) = (video.clone(), handle.clone());
            move |cb: &Closure<dyn FnMut(JsValue, JsValue)>| {
                let id = if rvfc {
                    call(&video, "requestVideoFrameCallback", &[cb.as_ref().clone()])
                } else {
                    call(&js_sys::global(), "requestAnimationFrame", &[cb.as_ref().clone()])
                };
                handle.set((id.ok().and_then(|v| v.as_f64()).unwrap_or(0.0), rvfc));
            }
        };
        let (again, s2) = (callback.clone(), s.clone());
        let last = Cell::new(f64::NAN);
        let req = request.clone();
        *callback.borrow_mut() = Some(Closure::new(move |_now: JsValue, meta: JsValue| {
            if s2.borrow().closed {
                return;
            }
            let at = if rvfc {
                get(&meta, "mediaTime").as_f64()
            } else {
                // Without frame callbacks: a new picture whenever the time moved.
                let t = num(&s2.borrow().video, "currentTime");
                (t != last.get()).then(|| {
                    last.set(t);
                    t
                })
            };
            if let Some(at) = at {
                shown(&s2, at);
            }
            if let Some(cb) = again.borrow().as_ref() {
                req(cb);
            }
        }));
        if let Some(cb) = callback.borrow().as_ref() {
            request(cb);
        }
        Frames { callback, handle }
    }

    fn stop(&self, video: &JsValue) {
        let (id, rvfc) = self.handle.get();
        let _ = if rvfc {
            call(video, "cancelVideoFrameCallback", &[id.into()])
        } else {
            call(&js_sys::global(), "cancelAnimationFrame", &[id.into()])
        };
        self.callback.borrow_mut().take();
    }
}

/// One stream's `SourceBuffer` and what waits to go into it.
struct Buffer {
    sb: JsValue,
    track: fmp4::Track,
    ops: VecDeque<Op>,
    /// What the buffer is busy with: an initialization segment (which a seek must not abort).
    busy_init: bool,
    _listeners: Vec<Listener>,
}

enum Op {
    Init(Uint8Array),
    Append(Uint8Array),
    Remove(f64, f64),
}

impl Buffer {
    fn updating(&self) -> bool {
        get(&self.sb, "updating").is_truthy()
    }

    fn buffered(&self) -> Vec<std::ops::Range<Duration>> {
        ranges(&get(&self.sb, "buffered"))
    }

    /// Starts the next operation when the buffer is free. A full buffer (the browser's quota)
    /// first lets go of what played more than a second ago, then of everything before the
    /// playhead; past that it is an error.
    fn run(&mut self, now: f64) -> Result<(), MediaError> {
        if self.updating() {
            return Ok(());
        }
        self.busy_init = false;
        let Some(op) = self.ops.front() else { return Ok(()) };
        let done = match op {
            Op::Init(b) | Op::Append(b) => call(&self.sb, "appendBuffer", &[b.clone().into()]),
            Op::Remove(from, to) => call(&self.sb, "remove", &[(*from).into(), (*to).into()]),
        };
        match done {
            Ok(_) => {
                self.busy_init = matches!(op, Op::Init(_));
                self.ops.pop_front();
                Ok(())
            }
            Err(e) if get(&e, "name").as_string().as_deref() == Some("QuotaExceededError") => {
                let start = self.buffered().first().map_or(f64::INFINITY, |r| secs(r.start));
                let behind = now - 1.0;
                if start >= behind {
                    return Err(MediaError::Decode(js_error("the browser's media buffer is full", &e)));
                }
                self.ops.push_front(Op::Remove(0.0, behind));
                Ok(())
            }
            Err(e) => Err(MediaError::Decode(js_error("handing the stream to the browser", &e))),
        }
    }
}

/// An event listener, removed when dropped.
struct Listener {
    target: JsValue,
    name: &'static str,
    f: Closure<dyn FnMut(JsValue)>,
}

impl Listener {
    fn on(target: &JsValue, name: &'static str, f: impl FnMut(JsValue) + 'static) -> Listener {
        let f = Closure::<dyn FnMut(JsValue)>::new(f);
        let _ = call(target, "addEventListener", &[name.into(), f.as_ref().clone()]);
        Listener { target: target.clone(), name, f }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = call(&self.target, "removeEventListener", &[self.name.into(), self.f.as_ref().clone()]);
    }
}

/// An HLS stream fed to the element through Media Source Extensions.
struct HlsFeed {
    reader: Rc<RefCell<Reader>>,
    /// The `MediaSource`, and whether it has opened (`sourceopen`).
    source: Option<(JsValue, Rc<Cell<bool>>)>,
    video: Option<Buffer>,
    audio: Option<Buffer>,
    /// Where the newest unit handed over shows.
    ahead: Duration,
    /// The last units have gone to the buffers; the end follows once they are in.
    flushed: bool,
    ended: bool,
    /// The length the element was last told.
    length: f64,
    _listeners: Vec<Listener>,
}

impl HlsFeed {
    fn buffers(&mut self) -> impl Iterator<Item = &mut Buffer> {
        self.video.iter_mut().chain(self.audio.iter_mut())
    }
}

/// A source played by a hidden media element.
pub(super) struct ElementDecoder {
    s: Shared,
    info: MediaInfo,
    hls: Option<HlsFeed>,
    playout: bool,
    object_url: Option<String>,
    frames: Frames,
    _listeners: Vec<Listener>,
    _live: crate::os::Live,
}

// SAFETY: as for `ElementFrame`.
unsafe impl Send for ElementDecoder {}

/// The error the element reports.
fn element_error(video: &JsValue) -> MediaError {
    let e = get(video, "error");
    let message = get(&e, "message").as_string().filter(|m| !m.is_empty());
    let what = match num(&e, "code") as u32 {
        2 => return MediaError::NotFound(message.unwrap_or_else(|| "the media could not be fetched".into())),
        4 => "the browser cannot play this media",
        _ => "the browser could not decode the media",
    };
    let message = message.map_or_else(|| what.to_string(), |m| format!("{what}: {m}"));
    MediaError::Format(message)
}

impl ElementDecoder {
    fn new() -> Result<ElementDecoder, MediaError> {
        let unsupported = |e: JsValue| MediaError::Unsupported(js_error("a media element", &e));
        let doc = get(&js_sys::global(), "document");
        // The element still in its picture-in-picture window, if the stream before left one.
        let spare = SPARE.with(|s| s.borrow_mut().take());
        let video = match spare {
            Some(sp) => {
                let _ = call(&js_sys::global(), "clearTimeout", &[sp.timer.into()]);
                if let Some(url) = sp.url {
                    let _ = call(&get(&js_sys::global(), "URL"), "revokeObjectURL", &[url.into()]);
                }
                sp.video
            }
            None => call(&doc, "createElement", &["video".into()]).map_err(unsupported)?,
        };
        set(&video, "preload", "auto");
        set(&video, "playsInline", true);
        set(&video, "crossOrigin", "anonymous");
        // A managed media source (Safari) plays only with AirPlay off or offered another way.
        set(&video, "disableRemotePlayback", true);
        let _ = call(&video, "setAttribute", &["playsinline".into(), "".into()]);
        let _ = call(&video, "setAttribute", &["aria-hidden".into(), "true".into()]);
        // In the page and over everything, so the browser composites (and calls back for) its
        // frames, a covered element's being skipped; but unseen and out of the way: the
        // engine draws the pictures.
        set(&get(&video, "style"), "cssText", HIDDEN);
        let body = get(&doc, "body");
        call(&body, "appendChild", std::slice::from_ref(&video)).map_err(unsupported)?;
        let s: Shared = Rc::new(RefCell::new(State {
            video: video.clone(),
            frames: VecDeque::new(),
            waker: None,
            error: None,
            ended: false,
            meta: false,
            playing: false,
            policy_muted: false,
            floor: 0.0,
            closed: false,
        }));
        let on = |name: &'static str, f: fn(&mut State)| {
            let s = s.clone();
            Listener::on(&video, name, move |_| {
                let mut st = s.borrow_mut();
                f(&mut st);
                st.wake();
            })
        };
        let mut listeners = vec![
            on("loadedmetadata", |st| st.meta = true),
            on("ended", |st| st.ended = true),
            on("error", |st| {
                let e = element_error(&st.video);
                st.fail(e);
            }),
        ];
        for name in ["timeupdate", "progress", "waiting", "playing", "canplay", "durationchange"] {
            listeners.push(on(name, |_| {}));
        }
        // Into or out of its picture-in-picture window: the picture again, which wakes the
        // player to say so.
        for name in ["enterpictureinpicture", "leavepictureinpicture"] {
            let s = s.clone();
            listeners.push(Listener::on(&video, name, move |_| {
                let t = num(&s.borrow().video, "currentTime");
                shown(&s, t);
            }));
        }
        // Without frame callbacks a seek while paused shows its frame once it lands (where
        // there are, the callback brings it: drawn at `seeked` it may still be the old one).
        let paused_frame = s.clone();
        let rvfc = get(&video, "requestVideoFrameCallback").is_function();
        listeners.push(Listener::on(&video, "seeked", move |_| {
            if rvfc {
                paused_frame.borrow_mut().wake();
                return;
            }
            let (paused, t) = {
                let st = paused_frame.borrow();
                (get(&st.video, "paused").is_truthy(), num(&st.video, "currentTime"))
            };
            if paused {
                shown(&paused_frame, t);
            }
            paused_frame.borrow_mut().wake();
        }));
        let frames = Frames::start(&s);
        Ok(ElementDecoder {
            s,
            info: MediaInfo::default(),
            hls: None,
            playout: false,
            object_url: None,
            frames,
            _listeners: listeners,
            _live: crate::os::Live::new(),
        })
    }

    fn video(&self) -> JsValue {
        self.s.borrow().video.clone()
    }

    /// `url` played by the element as it is: an MP4, WebM or MP3 file, or HLS where the browser
    /// plays it itself (Safari).
    pub(super) fn plain(url: &str) -> Result<ElementDecoder, MediaError> {
        let dec = ElementDecoder::new()?;
        set(&dec.video(), "src", url);
        Ok(dec)
    }

    /// A file the person picked or dropped, through a blob URL.
    pub(super) fn bytes(bytes: &[u8]) -> Result<ElementDecoder, MediaError> {
        let fail = |e: JsValue| MediaError::Unsupported(js_error("a blob URL", &e));
        let blob =
            construct(get(&js_sys::global(), "Blob"), &Array::of1(&Array::of1(&Uint8Array::from(bytes))))
                .map_err(fail)?;
        let url = call(&get(&js_sys::global(), "URL"), "createObjectURL", &[blob])
            .map_err(fail)?
            .as_string()
            .unwrap_or_default();
        let mut dec = ElementDecoder::plain(&url)?;
        dec.object_url = Some(url);
        Ok(dec)
    }

    /// An HLS playlist: through Media Source Extensions, or the element's own HLS (Safari
    /// without them); `Unsupported` where the browser has neither.
    pub(super) fn hls(url: &str) -> Result<ElementDecoder, MediaError> {
        if media_source_class().is_none() {
            let doc = get(&js_sys::global(), "document");
            let native = call(&doc, "createElement", &["video".into()])
                .and_then(|probe| call(&probe, "canPlayType", &["application/vnd.apple.mpegurl".into()]))
                .ok()
                .and_then(|v| v.as_string())
                .is_some_and(|v| !v.is_empty());
            if native {
                return ElementDecoder::plain(url);
            }
            return Err(MediaError::Unsupported(
                "this browser has neither WebCodecs (secure pages only) nor Media Source Extensions for HLS"
                    .into(),
            ));
        }
        let mut dec = ElementDecoder::new()?;
        dec.hls = Some(HlsFeed {
            reader: Reader::new(url),
            source: None,
            video: None,
            audio: None,
            ahead: Duration::ZERO,
            flushed: false,
            ended: false,
            length: f64::NAN,
            _listeners: Vec::new(),
        });
        dec.pump()?;
        Ok(dec)
    }

    /// Fetches, opens the media source once the streams are known, hands the buffers what the
    /// playhead will want next, lets go of what played, and ends the stream at its end.
    fn pump(&mut self) -> Result<(), MediaError> {
        let video = self.video();
        let s = self.s.clone();
        let Some(h) = self.hls.as_mut() else { return Ok(()) };
        let woken = s.clone();
        fetch_next(
            &h.reader,
            Rc::new(move |delivered| match delivered {
                Ok(()) => woken.borrow_mut().wake(),
                Err(e) => woken.borrow_mut().fail(e),
            }),
        );
        if h.source.is_none() {
            if !h.reader.borrow().hls.started() {
                return Ok(());
            }
            let unsupported = |e: JsValue| MediaError::Unsupported(js_error("Media Source Extensions", &e));
            let class =
                media_source_class().ok_or_else(|| MediaError::Unsupported("no MediaSource".into()))?;
            let source = construct(class, &Array::new()).map_err(unsupported)?;
            let opened = Rc::new(Cell::new(false));
            let (o, w) = (opened.clone(), s.clone());
            h._listeners.push(Listener::on(&source, "sourceopen", move |_| {
                o.set(true);
                w.borrow_mut().wake();
            }));
            let url = call(&get(&js_sys::global(), "URL"), "createObjectURL", std::slice::from_ref(&source))
                .map_err(unsupported)?
                .as_string()
                .unwrap_or_default();
            set(&video, "src", url.as_str());
            self.object_url = Some(url);
            h.source = Some((source, opened));
            return Ok(());
        }
        let Some((source, opened)) = h.source.clone() else { return Ok(()) };
        if !opened.get() {
            return Ok(());
        }
        if h.video.is_none() && h.audio.is_none() {
            let (vc, ac) = {
                let r = h.reader.borrow();
                (r.hls.video().cloned(), r.hls.audio().cloned())
            };
            let class = media_source_class().unwrap_or(JsValue::UNDEFINED);
            let supported = |mime: &str| {
                call(&class, "isTypeSupported", &[mime.into()]).ok().is_some_and(|v| v.is_truthy())
            };
            let add = |mime: &str, video: bool, rate: u32, init: Vec<u8>| -> Result<Buffer, MediaError> {
                let sb = call(&source, "addSourceBuffer", &[mime.into()])
                    .map_err(|e| MediaError::Unsupported(js_error(&format!("playing {mime}"), &e)))?;
                let w = s.clone();
                let wake = Listener::on(&sb, "updateend", move |_| w.borrow_mut().wake());
                let w = s.clone();
                let error = Listener::on(&sb, "error", move |_| {
                    w.borrow_mut().fail(MediaError::Decode("the browser could not take the stream".into()))
                });
                let mut ops = VecDeque::new();
                ops.push_back(Op::Init(Uint8Array::from(init.as_slice())));
                Ok(Buffer {
                    sb,
                    track: fmp4::Track::new(video, rate),
                    ops,
                    busy_init: false,
                    _listeners: vec![wake, error],
                })
            };
            if let Some(v) = &vc {
                let mime = format!("video/mp4; codecs=\"{}\"", v.codec);
                if !supported(&mime) {
                    return Err(MediaError::Unsupported(format!(
                        "this browser's media element cannot play {mime}"
                    )));
                }
                h.video = Some(add(&mime, true, 0, fmp4::video_init(v))?);
            }
            if let Some(a) = &ac {
                let mime = fmp4::audio_codecs(a)
                    .into_iter()
                    .map(|c| format!("audio/mp4; codecs=\"{c}\""))
                    .find(|m| supported(m))
                    .ok_or_else(|| {
                        MediaError::Unsupported(format!(
                            "this browser's media element cannot play {} sound",
                            a.codec_string
                        ))
                    })?;
                h.audio = Some(add(&mime, false, a.sample_rate, fmp4::audio_init(a))?);
            }
            self.info = h.reader.borrow().hls.info();
            // The length at once, before anything is appended (it cannot change while a buffer
            // is busy): a seek before the first segment is in needs it, or the element clamps
            // the seek to nothing buffered yet.
            if let Some(length) = self.info.duration.map(secs).filter(|l| l.is_finite() && *l > 0.0) {
                set(&source, "duration", length);
                h.length = length;
            }
            for b in h.buffers() {
                b.run(0.0)?;
            }
        }
        // What the playhead wants next, up to `mse_ahead` past it.
        let now = num(&video, "currentTime");
        let limits = h.reader.borrow().hls.limits;
        let reach = duration(now) + limits.mse_ahead;
        let (mut vu, mut au) = (Vec::new(), Vec::new());
        while h.ahead < reach {
            let Some(u) = h.reader.borrow_mut().hls.next_unit() else { break };
            match &u {
                Unit::Video { pts, .. } => {
                    h.ahead = h.ahead.max(*pts);
                    vu.push(u);
                }
                Unit::Audio { pts, .. } => {
                    h.ahead = h.ahead.max(*pts);
                    au.push(u);
                }
            }
        }
        let end = !h.flushed && h.reader.borrow().hls.ended();
        h.flushed |= end;
        for (b, units) in [(h.video.as_mut(), vu), (h.audio.as_mut(), au)] {
            if let Some(b) = b
                && let Some(f) = b.track.fragment(units, end)
            {
                b.ops.push_back(Op::Append(Uint8Array::from(f.as_slice())));
            }
        }
        // What played long enough ago goes.
        let behind = now - secs(limits.mse_behind);
        for b in h.buffers() {
            if b.ops.is_empty()
                && !b.updating()
                && behind > 1.0
                && b.buffered().first().is_some_and(|r| secs(r.start) < behind - 1.0)
            {
                b.ops.push_back(Op::Remove(0.0, behind));
            }
        }
        for b in h.buffers() {
            b.run(now)?;
        }
        let idle = h.buffers().all(|b| b.ops.is_empty() && !b.updating());
        let open = get(&source, "readyState").as_string().as_deref() == Some("open");
        // The length, as the playlist says it (growing with a growing one); never less than
        // what is buffered, which the element refuses.
        self.info.duration = h.reader.borrow().hls.info().duration;
        self.info.growing = h.reader.borrow().hls.info().growing;
        let length = self.info.duration.map_or(f64::NAN, secs);
        if idle && open && length.is_finite() && length != h.length {
            let held = h.buffers().flat_map(|b| b.buffered()).map(|r| secs(r.end)).fold(0.0, f64::max);
            set(&source, "duration", length.max(held));
            h.length = length;
        }
        if idle && open && h.flushed && !h.ended {
            h.ended = true;
            let _ = call(&source, "endOfStream", &[]);
        }
        Ok(())
    }

    /// What the element knows of a source it plays itself.
    fn plain_info(&self) -> MediaInfo {
        let video = self.video();
        let length = num(&video, "duration");
        let (w, h) = (num(&video, "videoWidth") as u32, num(&video, "videoHeight") as u32);
        MediaInfo {
            duration: length.is_finite().then(|| duration(length)),
            seekable: length.is_finite(),
            video: (w > 0 && h > 0).then(|| VideoInfo::new(w, h, None)),
            // The element's own sound: its format is the element's to know.
            audio: None,
            ..MediaInfo::default()
        }
    }
}

impl Decoder for ElementDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MediaError>> {
        self.s.borrow_mut().waker = Some(cx.waker().clone());
        if let Err(e) = self.pump() {
            return Poll::Ready(Err(e));
        }
        if let Some(e) = self.s.borrow_mut().error.take() {
            return Poll::Ready(Err(e));
        }
        match &self.hls {
            Some(h) if h.video.is_some() || h.audio.is_some() => Poll::Ready(Ok(())),
            Some(_) => Poll::Pending,
            None if self.s.borrow().meta => {
                self.info = self.plain_info();
                Poll::Ready(Ok(()))
            }
            None => Poll::Pending,
        }
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        match self.poll_read(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(MediaError::Decode(
                "the media element shows its pictures later; this decoder is read with poll_read".into(),
            )),
        }
    }

    fn poll_read(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Sample>, MediaError>> {
        self.s.borrow_mut().waker = Some(cx.waker().clone());
        if let Err(e) = self.pump() {
            return Poll::Ready(Err(e));
        }
        let mut st = self.s.borrow_mut();
        if let Some(e) = st.error.take() {
            return Poll::Ready(Err(e));
        }
        if let Some(f) = st.frames.pop_front() {
            return Poll::Ready(Ok(Some(f)));
        }
        if st.ended {
            return Poll::Ready(Ok(None));
        }
        Poll::Pending
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        {
            let mut st = self.s.borrow_mut();
            st.frames.clear();
            st.ended = false;
            st.floor = secs(to);
        }
        let Some(h) = self.hls.as_mut() else { return Ok(()) };
        h.flushed = false;
        h.ended = false;
        // Within what every buffer holds: reading goes on from its end; elsewhere it starts
        // again at `to` and what the buffers hold goes.
        let mut held_to: Option<Duration> = None;
        let mut within = true;
        for b in h.buffers() {
            b.track.reset();
            b.ops.retain(|op| matches!(op, Op::Init(_)));
            match b.buffered().into_iter().find(|r| r.start <= to && to < r.end) {
                Some(r) => held_to = Some(held_to.map_or(r.end, |e| e.min(r.end))),
                None => within = false,
            }
        }
        let open = h
            .source
            .as_ref()
            .is_some_and(|(s, _)| get(s, "readyState").as_string().as_deref() == Some("open"));
        let from = match held_to.filter(|_| within) {
            Some(end) => end,
            None => {
                for b in h.buffers() {
                    if b.updating() && !b.busy_init && open {
                        let _ = call(&b.sb, "abort", &[]);
                    }
                    b.ops.push_back(Op::Remove(0.0, f64::INFINITY));
                }
                to
            }
        };
        h.reader.borrow_mut().hls.seek(from);
        h.ahead = from;
        self.pump()
    }

    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        // Also keeps the fetches going while nothing is read (paused).
        if let Err(e) = self.pump() {
            self.s.borrow_mut().fail(e);
        }
        Some(ranges(&get(&self.video(), "buffered")))
    }

    fn playout(&mut self) -> Option<Box<dyn Playout>> {
        if std::mem::replace(&mut self.playout, true) {
            return None;
        }
        Some(Box::new(ElementPlayout(self.s.clone())))
    }
}

impl Drop for ElementDecoder {
    fn drop(&mut self) {
        let video = self.video();
        self.s.borrow_mut().closed = true;
        self.frames.stop(&video);
        let url = self.object_url.take();
        if !in_pip(&video) {
            return let_go(&video, url);
        }
        // In its picture-in-picture window: kept for the next stream a moment.
        let _ = call(&video, "pause", &[]);
        let _ = SPARE.with(|s| s.borrow_mut().take()).map(|old| let_go(&old.video, old.url));
        let gone = video.clone();
        let let_go_later = Closure::<dyn FnMut()>::new(move || {
            if let Some(sp) = SPARE.with(|s| s.borrow_mut().take()) {
                let doc = get(&js_sys::global(), "document");
                if in_pip(&gone) {
                    let _ = call(&doc, "exitPictureInPicture", &[]);
                }
                let_go(&sp.video, sp.url);
            }
        });
        let ms = PIP_HANDBACK.as_secs_f64() * 1000.0;
        let timer = call(&js_sys::global(), "setTimeout", &[let_go_later.as_ref().clone(), ms.into()])
            .ok()
            .and_then(|t| t.as_f64())
            .unwrap_or(0.0);
        SPARE.with(|s| *s.borrow_mut() = Some(Spare { video, url, timer, _let_go: let_go_later }));
    }
}

/// The element's playback, as the player drives it.
struct ElementPlayout(Shared);

// SAFETY: as for `ElementFrame`.
unsafe impl Send for ElementPlayout {}

impl ElementPlayout {
    fn video(&self) -> JsValue {
        self.0.borrow().video.clone()
    }
}

/// Plays `video`; a page not yet allowed sound plays it muted instead (as browsers allow), and
/// sound comes on with the next volume the person sets.
fn start(s: &Shared) {
    let video = s.borrow().video.clone();
    let Ok(p) = call(&video, "play", &[]) else { return };
    let s = s.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let Err(e) = JsFuture::from(js_sys::Promise::from(p)).await else { return };
        let still = s.borrow().playing && !s.borrow().closed;
        if still && get(&e, "name").as_string().as_deref() == Some("NotAllowedError") {
            s.borrow_mut().policy_muted = true;
            set(&video, "muted", true);
            if let Ok(p) = call(&video, "play", &[]) {
                let _ = JsFuture::from(js_sys::Promise::from(p)).await;
            }
        }
    });
}

impl Playout for ElementPlayout {
    fn position(&mut self) -> Duration {
        duration(num(&self.video(), "currentTime"))
    }

    fn play(&mut self) {
        self.0.borrow_mut().playing = true;
        start(&self.0);
    }

    fn pause(&mut self) {
        self.0.borrow_mut().playing = false;
        let _ = call(&self.video(), "pause", &[]);
    }

    fn seek(&mut self, to: Duration) {
        set(&self.video(), "currentTime", secs(to));
    }

    fn set_volume(&mut self, gain: f32) {
        let video = self.video();
        set(&video, "volume", f64::from(gain.clamp(0.0, 1.0)));
        if std::mem::take(&mut self.0.borrow_mut().policy_muted) {
            set(&video, "muted", false);
        }
    }

    fn set_speed(&mut self, speed: f64) {
        let video = self.video();
        set(&video, "preservesPitch", true);
        set(&video, "playbackRate", speed);
    }

    fn waiting(&mut self) -> bool {
        let video = self.video();
        let st = self.0.borrow();
        st.playing && !st.ended && (num(&video, "readyState") < 3.0 || get(&video, "seeking").is_truthy())
    }

    fn can_pip(&mut self) -> bool {
        get(&get(&js_sys::global(), "document"), "pictureInPictureEnabled").is_truthy()
            && get(&self.video(), "requestPictureInPicture").is_function()
    }

    fn set_pip(&mut self, on: bool) {
        let doc = get(&js_sys::global(), "document");
        let p = if on {
            call(&self.video(), "requestPictureInPicture", &[])
        } else if in_pip(&self.video()) {
            call(&doc, "exitPictureInPicture", &[])
        } else {
            return;
        };
        // A refusal (no recent click on the page) leaves the picture where it is.
        if let Ok(p) = p {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(js_sys::Promise::from(p)).await;
            });
        }
    }

    fn in_pip(&mut self) -> bool {
        in_pip(&self.video())
    }
}
