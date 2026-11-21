//! Linux: encoding a video stream and reading a granted screen, on the system's GStreamer
//! (opened at run time like the rest of this backend, never linked or bundled).
//!
//! The encoder is `appsrc ! videoconvert ! <encoder> ! h264parse ! appsink`: the graphics
//! card's encoder where one is installed (VA-API's `vah264enc`/`vah264lpenc`/`vaapih264enc`,
//! NVIDIA's `nvh264enc`), else the processor's (`x264enc`, `openh264enc`). `h264parse`
//! repeats the parameter sets before every key frame and cuts the stream into access units,
//! so what comes out is the Annex B stream [`VideoCodec::H264`] describes, the same a
//! Windows sharer sends.
//!
//! A granted screen is `pipewiresrc ! videoconvert ! appsink` on the PipeWire connection the
//! desktop's ScreenCast portal opened.

use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::time::Duration;

use libloading::Library;

use super::gstreamer::{
    GError, GST_MAP_READ, GST_MAP_WRITE, GST_STATE_CHANGE_FAILURE, GST_STATE_NULL, GST_STATE_PLAYING, Gst,
    GstBuffer, GstMapInfo, Ptr, gst,
};
use crate::deps;
use crate::host::{
    EncodedPiece, MediaError, ScreenFrame, ScreenSource, ScreenStream, VideoCodec, VideoEncoderOptions,
    VideoStreamEncoder,
};

/// GST_BUFFER_FLAG_DELTA_UNIT: the piece needs earlier ones (it is not a key frame).
const BUFFER_FLAG_DELTA_UNIT: c_uint = 1 << 13;
const CLOCK_TIME_NONE: u64 = u64::MAX;

/// GStreamer functions this file needs beyond the shared table.
struct Extra {
    set_object_arg: unsafe extern "C" fn(Ptr, *const c_char, *const c_char),
    find_property: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    element_send_event: unsafe extern "C" fn(Ptr, Ptr) -> c_int,
    upstream_force_key_unit: unsafe extern "C" fn(u64, c_int, c_uint) -> Ptr,
    _libs: Vec<Library>,
}

// SAFETY: plain C function pointers into libraries kept loaded for the process's life.
unsafe impl Send for Extra {}
// SAFETY: as above.
unsafe impl Sync for Extra {}

fn open_lib(names: &[&str]) -> Result<Library, String> {
    let mut last = String::new();
    for n in names {
        // SAFETY: loading GStreamer's and GLib's own libraries, safe to load into any process.
        match unsafe { Library::new(n) } {
            Ok(l) => return Ok(l),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        // SAFETY: the symbol's type is the documented C signature of that function; the
        // library stays loaded while the table lives.
        *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("{}: {e}", $name))?
    };
}

impl Extra {
    fn load() -> Result<Extra, String> {
        let gst = open_lib(&["libgstreamer-1.0.so.0", "libgstreamer-1.0.so"])?;
        let video = open_lib(&["libgstvideo-1.0.so.0", "libgstvideo-1.0.so"])?;
        let gobject = open_lib(&["libgobject-2.0.so.0", "libgobject-2.0.so"])?;
        Ok(Extra {
            set_object_arg: sym!(gst, "gst_util_set_object_arg"),
            find_property: sym!(gobject, "g_object_class_find_property"),
            element_send_event: sym!(gst, "gst_element_send_event"),
            upstream_force_key_unit: sym!(video, "gst_video_event_new_upstream_force_key_unit"),
            _libs: vec![gst, video, gobject],
        })
    }
}

fn extra() -> Result<&'static Extra, MediaError> {
    static EXTRA: std::sync::OnceLock<Result<Extra, String>> = std::sync::OnceLock::new();
    EXTRA
        .get_or_init(Extra::load)
        .as_ref()
        .map_err(|_| MediaError::Missing(deps::gst_library("libgstvideo-1.0", ENCODING)))
}

/// True when `object` has a property called `name`.
fn has_property(x: &Extra, object: Ptr, name: &CStr) -> bool {
    // SAFETY: a GObject's first field is its GTypeInstance, whose first field is its class;
    // looking a property up in the class only reads it.
    unsafe {
        let class = *object.cast::<Ptr>();
        !class.is_null() && !(x.find_property)(class, name.as_ptr()).is_null()
    }
}

/// Sets `name` on `object` from text (GStreamer converts it to the property's type), when
/// the object has that property. True when it was set.
fn set(x: &Extra, object: Ptr, name: &str, value: &str) -> bool {
    let (Ok(n), Ok(v)) = (CString::new(name), CString::new(value)) else { return false };
    if !has_property(x, object, &n) {
        return false;
    }
    // SAFETY: setting an existing property from a string on a live object.
    unsafe { (x.set_object_arg)(object, n.as_ptr(), v.as_ptr()) };
    true
}

/// How an encoder element is told its rate and key frame spacing, and whether it can change
/// the rate while it runs.
struct Kind {
    element: &'static str,
    hardware: bool,
    /// The bitrate property counts bits a second (else kilobits).
    bits: bool,
    /// The rate can change while encoding.
    live_rate: bool,
    /// Properties set once: (name, value), where `{gop}` is the key frame spacing.
    settings: &'static [(&'static str, &'static str)],
    gop: &'static str,
}

const KINDS: &[Kind] = &[
    Kind {
        element: "nvh264enc",
        hardware: true,
        bits: false,
        live_rate: true,
        settings: &[
            ("bframes", "0"),
            ("rc-mode", "cbr"),
            ("zerolatency", "true"),
            ("preset", "low-latency-hq"),
        ],
        gop: "gop-size",
    },
    Kind {
        element: "vah264enc",
        hardware: true,
        bits: false,
        live_rate: true,
        settings: &[("b-frames", "0"), ("rate-control", "cbr")],
        gop: "key-int-max",
    },
    Kind {
        element: "vah264lpenc",
        hardware: true,
        bits: false,
        live_rate: true,
        settings: &[("b-frames", "0"), ("rate-control", "cbr")],
        gop: "key-int-max",
    },
    Kind {
        element: "vaapih264enc",
        hardware: true,
        bits: false,
        live_rate: false,
        settings: &[("max-bframes", "0"), ("rate-control", "cbr")],
        gop: "keyframe-period",
    },
    Kind {
        element: "x264enc",
        hardware: false,
        bits: false,
        live_rate: true,
        settings: &[
            ("tune", "zerolatency"),
            ("speed-preset", "veryfast"),
            ("bframes", "0"),
            ("byte-stream", "true"),
        ],
        gop: "key-int-max",
    },
    Kind {
        element: "openh264enc",
        hardware: false,
        bits: true,
        live_rate: false,
        settings: &[("rate-control", "bitrate"), ("complexity", "low"), ("usage-type", "screen")],
        gop: "gop-size",
    },
];

/// The encoders to try, in order: NVIDIA's first when that card is preferred, else VA-API's
/// first; the processor's last (or only, without `hardware`).
fn order(hardware: bool, vendor: u32) -> Vec<&'static Kind> {
    let mut hw: Vec<&Kind> = KINDS.iter().filter(|k| k.hardware).collect();
    if vendor != 0x10de {
        hw.sort_by_key(|k| k.element == "nvh264enc");
    }
    let sw = KINDS.iter().filter(|k| !k.hardware);
    if hardware { hw.into_iter().chain(sw).collect() } else { sw.collect() }
}

/// True when GStreamer has an element called `name` installed.
fn installed(g: &Gst, name: &str) -> bool {
    match g.make(name) {
        Ok(e) => {
            // SAFETY: the probe was floating and is ours to drop.
            unsafe {
                (g.object_ref)(e);
                (g.object_unref)(e);
                (g.object_unref)(e);
            }
            true
        }
        Err(_) => false,
    }
}

/// A pipeline with a named source and sink, started.
pub(crate) struct Pipe {
    g: &'static Gst,
    pub(crate) pipeline: Ptr,
    named: Vec<Ptr>,
}

impl Pipe {
    /// Builds `desc` and looks up `names`; nothing is started.
    pub(crate) fn build(g: &'static Gst, desc: &str, names: &[&CStr]) -> Result<Pipe, MediaError> {
        let desc = CString::new(desc).map_err(|e| MediaError::Unsupported(e.to_string()))?;
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: building a pipeline from a description; errors come back in `err`.
        let pipeline = unsafe { (g.parse_launch)(desc.as_ptr(), &mut err) };
        if pipeline.is_null() {
            return Err(MediaError::Unsupported(g.take_error(err)));
        }
        let mut p = Pipe { g, pipeline, named: Vec::new() };
        if !err.is_null() {
            return Err(MediaError::Unsupported(g.take_error(err)));
        }
        for n in names {
            // SAFETY: looking up a named element (a new reference).
            let e = unsafe { (g.bin_get_by_name)(pipeline, n.as_ptr()) };
            if e.is_null() {
                return Err(MediaError::Unsupported(format!("the pipeline has no {}", n.to_string_lossy())));
            }
            p.named.push(e);
        }
        Ok(p)
    }

    pub(crate) fn get(&self, i: usize) -> Ptr {
        self.named.get(i).copied().unwrap_or(std::ptr::null_mut())
    }

    pub(crate) fn start(&self, what: &str) -> Result<(), MediaError> {
        // SAFETY: a state change on a live pipeline.
        if unsafe { (self.g.element_set_state)(self.pipeline, GST_STATE_PLAYING) } == GST_STATE_CHANGE_FAILURE
        {
            let reason =
                self.g.bus_error(self.pipeline).map_or_else(|| format!("{what} would not start"), |e| e.1);
            return Err(MediaError::Unsupported(reason));
        }
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: stopping the pipeline and releasing our references, once.
        unsafe {
            (self.g.element_set_state)(self.pipeline, GST_STATE_NULL);
            for e in self.named.drain(..) {
                (self.g.object_unref)(e);
            }
            (self.g.object_unref)(self.pipeline);
        }
    }
}

/// Pulls one sample from `sink`, waiting up to `wait`, and copies its bytes out with the
/// buffer's flags.
pub(crate) fn pull(g: &Gst, sink: Ptr, wait: Duration) -> Option<(Vec<u8>, c_uint, Ptr)> {
    let ns = u64::try_from(wait.as_nanos()).unwrap_or(u64::MAX);
    // SAFETY: pulling from a live app sink; the sample is ours until unref'd (by the caller,
    // through the returned pointer), its buffer mapped for reading while copied.
    unsafe {
        let s = (g.app_sink_try_pull_sample)(sink, ns);
        if s.is_null() {
            return None;
        }
        let buf = (g.sample_get_buffer)(s);
        if buf.is_null() {
            (g.mini_object_unref)(s);
            return None;
        }
        let flags = (*buf).mini_object.flags;
        let mut map = GstMapInfo::default();
        if (g.buffer_map)(buf, &mut map, GST_MAP_READ) == 0 {
            (g.mini_object_unref)(s);
            return None;
        }
        let bytes = if map.data.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(map.data, map.size).to_vec()
        };
        (g.buffer_unmap)(buf, &mut map);
        Some((bytes, flags, s))
    }
}

/// `appsrc ! videoconvert ! <encoder> ! h264parse ! appsink`, one picture at a time.
pub(crate) struct GstStreamEncoder {
    g: &'static Gst,
    x: &'static Extra,
    pipe: Pipe,
    kind: &'static Kind,
    frame: usize,
    interval: Duration,
}

// SAFETY: GStreamer elements are thread-safe; the encoder is used from one thread at a time.
unsafe impl Send for GstStreamEncoder {}

impl GstStreamEncoder {
    pub(crate) fn open(o: &VideoEncoderOptions) -> Result<GstStreamEncoder, MediaError> {
        let VideoCodec::H264 = o.codec;
        if o.width == 0 || o.height == 0 || !o.width.is_multiple_of(2) || !o.height.is_multiple_of(2) {
            return Err(MediaError::Format(format!(
                "H.264 needs an even picture size, not {}x{}",
                o.width, o.height
            )));
        }
        let (g, x) = (gst()?, extra()?);
        if !installed(g, "h264parse") {
            return Err(MediaError::Missing(deps::element("h264parse", ENCODING)));
        }
        let all = order(o.hardware, o.vendor);
        let mut failed: Vec<(&'static str, String)> = Vec::new();
        let mut candidates = all.clone();
        // The first that runs here, then the rest if this picture size still fails it.
        loop {
            match choose(&candidates, |n| installed(g, n), |k| usable(g, x, k)) {
                Ok(kind) => match Self::with(g, x, kind, o) {
                    Ok(e) => return Ok(e),
                    // It runs, but not at this size: the next one.
                    Err(e) => {
                        failed.push((kind.element, e.to_string()));
                        candidates.retain(|k| k.element != kind.element);
                    }
                },
                Err(rest) => {
                    failed.extend(rest);
                    return Err(none_runs(&all, &failed));
                }
            }
        }
    }

    /// The encoder [`GstStreamEncoder::open`] would choose with `hardware` (and no preferred
    /// card), after making sure it runs here; for a settings line that names it.
    pub(crate) fn choice(hardware: bool) -> Result<String, MediaError> {
        let (g, x) = (gst()?, extra()?);
        let all = order(hardware, 0);
        choose(&all, |n| installed(g, n), |k| usable(g, x, k))
            .map(|k| k.element.to_owned())
            .map_err(|failed| none_runs(&all, &failed))
    }

    fn with(
        g: &'static Gst,
        x: &'static Extra,
        kind: &'static Kind,
        o: &VideoEncoderOptions,
    ) -> Result<Self, MediaError> {
        let fps = o.fps.clamp(1, 240);
        let desc = format!(
            "appsrc name=src is-live=true format=time do-timestamp=true block=false \
             caps=video/x-raw,format=BGRx,width={w},height={h},framerate={fps}/1 \
             ! videoconvert ! video/x-raw,format=(string){{NV12,I420}} \
             ! {enc} name=enc ! h264parse config-interval=-1 \
             ! video/x-h264,stream-format=byte-stream,alignment=au \
             ! appsink name=sink sync=false max-buffers=0",
            w = o.width,
            h = o.height,
            enc = kind.element,
        );
        let pipe = Pipe::build(g, &desc, &[c"src", c"enc", c"sink"])?;
        let enc = pipe.get(1);
        let gop = if o.keyframe_interval == 0 { fps * 3 } else { o.keyframe_interval };
        for (name, value) in kind.settings {
            set(x, enc, name, value);
        }
        set(x, enc, kind.gop, &gop.to_string());
        set(x, enc, "bitrate", &rate(kind, o.kbps).to_string());
        pipe.start(kind.element)?;
        Ok(GstStreamEncoder {
            g,
            x,
            pipe,
            kind,
            frame: o.width as usize * o.height as usize * 4,
            interval: Duration::from_micros(1_000_000 / u64::from(fps)),
        })
    }
}

/// What sharing the screen uses an encoder for, in the words a missing component is named with.
const ENCODING: &str = "sharing the screen (encoding it as H.264)";

/// No encoder could be used: every candidate, in order, as not installed or as installed but
/// unable to run here (with what it said), the processor's last so its packages are the hint.
fn none_runs(candidates: &[&'static Kind], failed: &[(&'static str, String)]) -> MediaError {
    let alternatives = candidates
        .iter()
        .map(|k| {
            let d = deps::element(k.element, ENCODING);
            match failed.iter().find(|(n, _)| *n == k.element) {
                Some((_, why)) => d.unusable(why.clone()),
                None => d,
            }
        })
        .collect();
    MediaError::Missing(deps::any_of("an H.264 encoder", ENCODING, alternatives))
}

/// The first of `candidates` (in order) that is `installed` and passes `probe`, or each
/// installed one with what it said. Elements that are not installed are passed over.
fn choose(
    candidates: &[&'static Kind],
    installed: impl Fn(&str) -> bool,
    mut probe: impl FnMut(&'static Kind) -> Result<(), String>,
) -> Result<&'static Kind, Vec<(&'static str, String)>> {
    let mut failed = Vec::new();
    for &kind in candidates {
        if !installed(kind.element) {
            continue;
        }
        match probe(kind) {
            Ok(()) => return Ok(kind),
            Err(e) => failed.push((kind.element, e)),
        }
    }
    Err(failed)
}

/// How long a trial picture may take to come out of an encoder before it counts as broken.
const TRIAL_WAIT: Duration = Duration::from_secs(4);

/// Whether `kind` really encodes on this machine, asked once per process and remembered. The
/// trial builds its real pipeline (a state change it refuses fails it) and encodes small
/// pictures until one comes out: an installed element can still fail only then, when it
/// opens its device (`nvh264enc` without a usable NVIDIA card or CUDA, as in WSL or with the
/// card off; a VA-API driver without an H.264 encoder).
fn usable(g: &'static Gst, x: &'static Extra, kind: &'static Kind) -> Result<(), String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashMap<&'static str, Result<(), String>>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Mutex::default);
    if let Some(r) = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(kind.element) {
        return r.clone();
    }
    let trial = || -> Result<(), String> {
        let (w, h) = (256, 144);
        let mut e = GstStreamEncoder::with(g, x, kind, &VideoEncoderOptions::new(w, h, 30, 500))
            .map_err(|e| e.to_string())?;
        let picture = vec![0u8; w as usize * h as usize * 4];
        let until = std::time::Instant::now() + TRIAL_WAIT;
        while std::time::Instant::now() < until {
            if !e.encode(&picture).map_err(|e| e.to_string())?.is_empty() {
                return Ok(());
            }
        }
        Err("no picture came out of it".into())
    };
    let r = trial();
    seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(kind.element, r.clone());
    r
}

/// The bitrate property's value for `kbps` on this kind of encoder.
fn rate(kind: &Kind, kbps: u32) -> u32 {
    let kbps = kbps.clamp(100, 200_000);
    if kind.bits { kbps.saturating_mul(1000) } else { kbps }
}

impl VideoStreamEncoder for GstStreamEncoder {
    fn name(&self) -> &str {
        self.kind.element
    }

    fn is_hardware(&self) -> bool {
        self.kind.hardware
    }

    fn request_keyframe(&mut self) -> bool {
        // SAFETY: a new upstream event, handed (with its reference) to the sink, which sends
        // it up the pipeline to the encoder.
        unsafe {
            let ev = (self.x.upstream_force_key_unit)(CLOCK_TIME_NONE, 1, 0);
            !ev.is_null() && (self.x.element_send_event)(self.pipe.get(2), ev) != 0
        }
    }

    fn set_kbps(&mut self, kbps: u32) -> bool {
        self.kind.live_rate && set(self.x, self.pipe.get(1), "bitrate", &rate(self.kind, kbps).to_string())
    }

    fn encode(&mut self, bgrx: &[u8]) -> Result<Vec<EncodedPiece>, MediaError> {
        let g = self.g;
        if let Some((_, text)) = g.bus_error(self.pipe.pipeline) {
            return Err(MediaError::Decode(format!("{}: {text}", self.kind.element)));
        }
        if bgrx.len() < self.frame {
            return Err(MediaError::Format(format!("a picture of {} bytes, not {}", bgrx.len(), self.frame)));
        }
        // SAFETY: a new buffer of the picture's size, mapped for writing while filled, then
        // handed to the source (which takes ownership).
        unsafe {
            let buf: *mut GstBuffer =
                (g.buffer_new_allocate)(std::ptr::null_mut(), self.frame, std::ptr::null_mut());
            if buf.is_null() {
                return Err(MediaError::Decode("no memory for a picture".into()));
            }
            let mut map = GstMapInfo::default();
            if (g.buffer_map)(buf, &mut map, GST_MAP_WRITE) == 0
                || map.data.is_null()
                || map.size < self.frame
            {
                (g.mini_object_unref)(buf.cast());
                return Err(MediaError::Decode("a picture buffer could not be written".into()));
            }
            std::ptr::copy_nonoverlapping(bgrx.as_ptr(), map.data, self.frame);
            (g.buffer_unmap)(buf, &mut map);
            if (g.app_src_push_buffer)(self.pipe.get(0), buf) != 0 {
                return Err(MediaError::Decode("the encoder stopped taking pictures".into()));
            }
        }
        // This picture's piece usually follows within a fraction of a picture's time; pieces
        // an encoder holds longer come with a later call.
        let mut out = Vec::new();
        let mut wait = self.interval / 2;
        while let Some((data, flags, sample)) = pull(g, self.pipe.get(2), wait) {
            // SAFETY: the sample `pull` handed over, released once.
            unsafe { (g.mini_object_unref)(sample) };
            if !data.is_empty() {
                out.push(EncodedPiece { keyframe: flags & BUFFER_FLAG_DELTA_UNIT == 0, data });
            }
            wait = Duration::ZERO;
        }
        Ok(out)
    }
}

/// `pipewiresrc ! videoconvert ! appsink` on the portal's PipeWire connection.
pub(crate) struct GstScreenStream {
    g: &'static Gst,
    pipe: Pipe,
    /// The portal's connection, kept open while the stream reads it.
    _fd: std::os::fd::OwnedFd,
}

// SAFETY: as for the encoder.
unsafe impl Send for GstScreenStream {}

impl GstScreenStream {
    pub(crate) fn open(source: ScreenSource) -> Result<GstScreenStream, MediaError> {
        let ScreenSource::PipeWire { fd, node } = source;
        let g = gst()?;
        if !installed(g, "pipewiresrc") {
            return Err(MediaError::Missing(deps::element("pipewiresrc", "sharing the screen on Wayland")));
        }
        use std::os::fd::AsRawFd;
        // pipewiresrc takes its own copy of the connection; ours stays open with the stream.
        let desc = format!(
            "pipewiresrc fd={fd} path={node} do-timestamp=true always-copy=true \
             ! videoconvert ! video/x-raw,format=BGRx \
             ! appsink name=sink sync=false drop=true max-buffers=2",
            fd = fd.as_raw_fd(),
        );
        let pipe = Pipe::build(g, &desc, &[c"sink"])?;
        pipe.start("pipewiresrc")?;
        Ok(GstScreenStream { g, pipe, _fd: fd })
    }
}

impl ScreenStream for GstScreenStream {
    fn next(&mut self, timeout: Duration) -> Result<Option<ScreenFrame>, MediaError> {
        let g = self.g;
        let ended = |why: String| MediaError::NotFound(format!("the shared screen ended: {why}"));
        if let Some((_, text)) = g.bus_error(self.pipe.pipeline) {
            return Err(ended(text));
        }
        let sink = self.pipe.get(0);
        // The newest picture: the first waits up to `timeout`, any queued behind it are newer.
        let Some(mut got) = pull(g, sink, timeout) else {
            // SAFETY: asking a live app sink whether its stream is over.
            if unsafe { (g.app_sink_is_eos)(sink) } != 0 {
                return Err(ended("the desktop stopped it".into()));
            }
            return Ok(None);
        };
        while let Some(newer) = pull(g, sink, Duration::ZERO) {
            // SAFETY: the older sample is released once.
            unsafe { (g.mini_object_unref)(got.2) };
            got = newer;
        }
        let (bytes, _, sample) = got;
        let (mut w, mut h) = (0, 0);
        // SAFETY: reading the sample's caps (borrowed), then releasing the sample once.
        unsafe {
            let caps = (g.sample_get_caps)(sample);
            if !caps.is_null() {
                let s = (g.caps_get_structure)(caps, 0);
                if !s.is_null() {
                    (g.structure_get_int)(s, c"width".as_ptr(), &mut w);
                    (g.structure_get_int)(s, c"height".as_ptr(), &mut h);
                }
            }
            (g.mini_object_unref)(sample);
        }
        let (w, h) = (usize::try_from(w).unwrap_or(0), usize::try_from(h).unwrap_or(0));
        if w == 0 || h == 0 || bytes.len() < w * h * 4 {
            return Ok(None);
        }
        // Rows may be padded; the stride is what each row takes.
        let stride = bytes.len() / h;
        let bgrx = if stride == w * 4 {
            bytes
        } else {
            let mut tight = Vec::with_capacity(w * h * 4);
            for row in bytes.chunks(stride).take(h) {
                tight.extend_from_slice(row.get(..w * 4).unwrap_or(row));
            }
            tight
        };
        Ok(Some(ScreenFrame { width: w as u32, height: h as u32, bgrx }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graphics_card_goes_first_and_nvidia_only_when_preferred() {
        let names = |v: Vec<&Kind>| v.iter().map(|k| k.element).collect::<Vec<_>>();
        let any = names(order(true, 0));
        assert_eq!(any.first(), Some(&"vah264enc"));
        assert_eq!(any.iter().position(|n| *n == "nvh264enc"), Some(3), "{any:?}");
        assert_eq!(any.get(4..), Some(&["x264enc", "openh264enc"][..]));
        assert_eq!(names(order(true, 0x10de)).first(), Some(&"nvh264enc"));
        assert_eq!(names(order(false, 0x10de)), ["x264enc", "openh264enc"]);
    }

    /// A fake element factory: which elements are installed and which run.
    fn pick(
        installed: &[&str],
        runs: &[&str],
        hardware: bool,
        vendor: u32,
    ) -> (Option<&'static str>, Vec<(&'static str, String)>, Vec<&'static str>) {
        let mut probed = Vec::new();
        let r = choose(
            &order(hardware, vendor),
            |n| installed.contains(&n),
            |k| {
                probed.push(k.element);
                if runs.contains(&k.element) { Ok(()) } else { Err("no device".into()) }
            },
        );
        match r {
            Ok(k) => (Some(k.element), Vec::new(), probed),
            Err(t) => (None, t, probed),
        }
    }

    #[test]
    fn an_installed_encoder_that_does_not_run_is_never_chosen() {
        // WSL: NVIDIA's encoder is installed, but no usable card: the processor's is chosen.
        let (got, _, probed) =
            pick(&["nvh264enc", "x264enc", "openh264enc"], &["x264enc", "openh264enc"], true, 0x10de);
        assert_eq!(got, Some("x264enc"));
        assert_eq!(probed, ["nvh264enc", "x264enc"], "tried in order, stopping at the first that runs");
        // A graphics card that runs is chosen before the processor.
        let (got, _, _) = pick(&["vah264enc", "x264enc"], &["vah264enc", "x264enc"], true, 0);
        assert_eq!(got, Some("vah264enc"));
        // Not installed: never tried.
        let (got, _, probed) = pick(&["openh264enc"], &["openh264enc"], true, 0);
        assert_eq!((got, probed), (Some("openh264enc"), vec!["openh264enc"]));
        // Processor only: the graphics card is not even tried.
        let (got, _, probed) = pick(&["vah264enc", "x264enc"], &["vah264enc", "x264enc"], false, 0);
        assert_eq!((got, probed), (Some("x264enc"), vec!["x264enc"]));
    }

    #[test]
    fn with_none_running_each_says_why() {
        let (got, tried, _) = pick(&["nvh264enc", "x264enc"], &[], true, 0);
        assert_eq!(got, None);
        assert_eq!(tried, [("nvh264enc", "no device".to_owned()), ("x264enc", "no device".to_owned())]);
        let (got, tried, _) = pick(&[], &[], true, 0);
        assert_eq!((got, tried.len()), (None, 0), "nothing installed: nothing tried");
        // What people are told: every encoder, which cannot run and why, which is missing, and
        // the processor's encoder's packages to install.
        let all = order(true, 0);
        let MediaError::Missing(d) = none_runs(&all, &[("nvh264enc", "no NVIDIA card".into())]) else {
            unreachable!("a missing dependency")
        };
        assert_eq!(d.alternatives.len(), all.len());
        let text = d.to_string();
        assert!(text.contains("no NVIDIA card"), "{text}");
        assert!(text.contains("is not installed"), "{text}");
        assert!(!d.install.is_empty(), "says what to install: {text}");
    }

    #[test]
    fn rates_are_in_the_units_each_encoder_counts() {
        let x264 = KINDS.iter().find(|k| k.element == "x264enc").map(|k| rate(k, 4000));
        let open = KINDS.iter().find(|k| k.element == "openh264enc").map(|k| rate(k, 4000));
        assert_eq!((x264, open), (Some(4000), Some(4_000_000)));
        assert_eq!(KINDS.first().map(|k| rate(k, 5)), Some(100), "never below a usable rate");
    }

    /// Encodes drawn pictures where this machine's GStreamer has an H.264 encoder, and checks
    /// the stream starts with a key frame in Annex B with its parameter sets.
    #[test]
    fn pictures_become_an_annex_b_stream() {
        let mut enc =
            match GstStreamEncoder::open(&VideoEncoderOptions::new(320, 240, 30, 1500).hardware(false)) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("no H.264 encoder here, skipping: {e}");
                    return;
                }
            };
        let mut pieces = Vec::new();
        for n in 0..30u8 {
            let picture: Vec<u8> =
                (0..320 * 240).flat_map(|i| [n.wrapping_mul(7), (i % 256) as u8, 90, 255]).collect();
            match enc.encode(&picture) {
                Ok(p) => pieces.extend(p),
                Err(e) => unreachable!("encoding: {e}"),
            }
        }
        let Some(first) = pieces.first() else { unreachable!("no pieces came out") };
        assert!(first.keyframe, "the stream starts with a key frame");
        assert!(first.data.starts_with(&[0, 0, 0, 1]) || first.data.starts_with(&[0, 0, 1]), "Annex B");
        // NAL type 7 (SPS) appears before the first picture.
        assert!(
            first.data.windows(4).any(|w| matches!(w, [0, 0, 1, n] if n & 0x1f == 7)),
            "parameter sets in band"
        );
        // 8-bit 4:2:0 in Baseline, Main or High, which every H.264 decoder takes (Windows'
        // Media Foundation refuses High 4:4:4, which an encoder fed BGRx would choose).
        let profile = first.data.windows(5).find_map(|w| match w {
            [0, 0, 1, n, p] if n & 0x1f == 7 => Some(*p),
            _ => None,
        });
        assert!(matches!(profile, Some(66 | 77 | 100)), "profile_idc {profile:?}");
        assert!(enc.request_keyframe() || !enc.is_hardware());
    }
}
