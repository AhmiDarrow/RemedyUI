//! Linux: the system's GStreamer, opened at run time (`libgstreamer-1.0.so.0` and friends) so
//! nothing is linked or bundled. Decoding is a `playbin` with app sinks for NV12 pictures,
//! interleaved float sound and subtitle text; it plays whatever the installed plugins can
//! (the distribution's codecs, hardware decoders through VA-API plugins). Sound goes out
//! through an `appsrc ! audioconvert ! audioresample ! autoaudiosink` pipeline, whose
//! position is the sound clock.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use libloading::Library;

use crate::host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture,
    PixelFormat, Plane, Sample, TextTrackInfo, VideoInfo, YuvMatrix,
};

type GBoolean = c_int;
type Ptr = *mut c_void;

const GST_FORMAT_TIME: c_int = 3;
const GST_STATE_NULL: c_int = 1;
const GST_STATE_READY: c_int = 2;
const GST_STATE_PAUSED: c_int = 3;
const GST_STATE_PLAYING: c_int = 4;
const GST_STATE_CHANGE_FAILURE: c_int = 0;
const GST_SEEK_FLAG_FLUSH: c_int = 1;
const GST_SEEK_FLAG_ACCURATE: c_int = 2;
const GST_MESSAGE_ERROR: c_uint = 1 << 1;
const GST_MESSAGE_ANY: c_uint = !0;
const GST_MAP_READ: c_int = 1;
const GST_MAP_WRITE: c_int = 2;
const CLOCK_TIME_NONE: u64 = u64::MAX;
/// GST_RESOURCE_ERROR_NOT_FOUND and _OPEN_READ, the codes for a missing or unreadable source.
const RESOURCE_NOT_FOUND: c_int = 3;
const RESOURCE_OPEN_READ: c_int = 5;

/// How long a source may take to give its first picture or sound before it is reported as
/// unreachable: a slow network stream gets this long, a local file needs a fraction of it.
const PREROLL_WAIT: Duration = Duration::from_secs(30);

/// How long `read` waits on one sink before looking at the others.
const POLL: Duration = Duration::from_millis(5);

/// Decoded pictures an app sink holds before the decoder waits: the player keeps its own
/// look-ahead, so this only smooths hand-over between threads.
const SINK_PICTURES: c_uint = 4;
/// Decoded sound buffers an app sink holds before the decoder waits: about a second and a
/// half of AAC or MP3 (1024 or 1152 samples each at 48 kHz), so sound muxed well ahead of its
/// pictures never stalls them. Counted in buffers: appsink's time limit keeps its count of
/// queued time across a flushing seek and can then block a branch that holds nothing.
const SINK_SOUND: c_uint = 64;
/// Sound queued in the output pipeline's source ahead of its sink's own 200 ms buffer.
const OUTPUT_QUEUE: Duration = Duration::from_millis(200);

#[repr(C)]
struct GstMiniObject {
    type_: usize,
    refcount: c_int,
    lockstate: c_int,
    flags: c_uint,
    copy: Ptr,
    dispose: Ptr,
    free: Ptr,
    priv_uint: c_uint,
    priv_pointer: Ptr,
}

#[repr(C)]
struct GstBuffer {
    mini_object: GstMiniObject,
    pool: Ptr,
    pts: u64,
    dts: u64,
    duration: u64,
    offset: u64,
    offset_end: u64,
}

#[repr(C)]
struct GstMessage {
    mini_object: GstMiniObject,
    type_: c_uint,
}

#[repr(C)]
struct GstMapInfo {
    memory: Ptr,
    flags: c_int,
    data: *mut u8,
    size: usize,
    maxsize: usize,
    user_data: [Ptr; 4],
    reserved: [Ptr; 4],
}

impl Default for GstMapInfo {
    fn default() -> Self {
        GstMapInfo {
            memory: std::ptr::null_mut(),
            flags: 0,
            data: std::ptr::null_mut(),
            size: 0,
            maxsize: 0,
            user_data: [std::ptr::null_mut(); 4],
            reserved: [std::ptr::null_mut(); 4],
        }
    }
}

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

/// The GStreamer and GLib functions used, from the system's libraries.
struct Gst {
    init_check: unsafe extern "C" fn(*mut c_int, *mut c_void, *mut *mut GError) -> GBoolean,
    element_factory_make: unsafe extern "C" fn(*const c_char, *const c_char) -> Ptr,
    parse_launch: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> Ptr,
    bin_get_by_name: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    element_set_state: unsafe extern "C" fn(Ptr, c_int) -> c_int,
    element_get_state: unsafe extern "C" fn(Ptr, *mut c_int, *mut c_int, u64) -> c_int,
    element_query_duration: unsafe extern "C" fn(Ptr, c_int, *mut i64) -> GBoolean,
    element_query_position: unsafe extern "C" fn(Ptr, c_int, *mut i64) -> GBoolean,
    element_seek_simple: unsafe extern "C" fn(Ptr, c_int, c_int, i64) -> GBoolean,
    element_get_bus: unsafe extern "C" fn(Ptr) -> Ptr,
    element_query: unsafe extern "C" fn(Ptr, Ptr) -> GBoolean,
    query_new_seeking: unsafe extern "C" fn(c_int) -> Ptr,
    query_parse_seeking: unsafe extern "C" fn(Ptr, *mut c_int, *mut GBoolean, *mut i64, *mut i64),
    bus_timed_pop_filtered: unsafe extern "C" fn(Ptr, u64, c_uint) -> Ptr,
    message_parse_error: unsafe extern "C" fn(Ptr, *mut *mut GError, *mut *mut c_char),
    object_ref: unsafe extern "C" fn(Ptr) -> Ptr,
    object_unref: unsafe extern "C" fn(Ptr),
    mini_object_unref: unsafe extern "C" fn(Ptr),
    caps_from_string: unsafe extern "C" fn(*const c_char) -> Ptr,
    caps_get_structure: unsafe extern "C" fn(Ptr, c_uint) -> Ptr,
    structure_get_int: unsafe extern "C" fn(Ptr, *const c_char, *mut c_int) -> GBoolean,
    structure_get_fraction: unsafe extern "C" fn(Ptr, *const c_char, *mut c_int, *mut c_int) -> GBoolean,
    structure_get_string: unsafe extern "C" fn(Ptr, *const c_char) -> *const c_char,
    sample_get_buffer: unsafe extern "C" fn(Ptr) -> *mut GstBuffer,
    sample_get_caps: unsafe extern "C" fn(Ptr) -> Ptr,
    buffer_map: unsafe extern "C" fn(*mut GstBuffer, *mut GstMapInfo, c_int) -> GBoolean,
    buffer_unmap: unsafe extern "C" fn(*mut GstBuffer, *mut GstMapInfo),
    buffer_new_allocate: unsafe extern "C" fn(Ptr, usize, Ptr) -> *mut GstBuffer,
    app_sink_try_pull_sample: unsafe extern "C" fn(Ptr, u64) -> Ptr,
    app_sink_is_eos: unsafe extern "C" fn(Ptr) -> GBoolean,
    app_src_push_buffer: unsafe extern "C" fn(Ptr, *mut GstBuffer) -> c_int,
    app_src_end_of_stream: unsafe extern "C" fn(Ptr) -> c_int,
    object_set: unsafe extern "C" fn(Ptr, *const c_char, ...),
    object_get: unsafe extern "C" fn(Ptr, *const c_char, ...),
    error_free: unsafe extern "C" fn(*mut GError),
    free: unsafe extern "C" fn(Ptr),
    filename_to_uri: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_char,
    _libs: Vec<Library>,
}

// SAFETY: the table holds plain C function pointers into libraries kept loaded for the
// process's life; GStreamer's API is thread-safe for the calls made here.
unsafe impl Send for Gst {}
// SAFETY: as above.
unsafe impl Sync for Gst {}

fn open_lib(names: &[&str]) -> Result<Library, String> {
    let mut last = String::new();
    for n in names {
        // SAFETY: loading a system library runs its initializers; these are GStreamer's and
        // GLib's own, which are safe to load into any process.
        match unsafe { Library::new(n) } {
            Ok(l) => return Ok(l),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        // SAFETY: the symbol's type is the documented C signature of that GStreamer or GLib
        // function; the library stays loaded while the table lives.
        *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("{}: {e}", $name))?
    };
}

impl Gst {
    fn load() -> Result<Gst, String> {
        let gst = open_lib(&["libgstreamer-1.0.so.0", "libgstreamer-1.0.so"])?;
        let app = open_lib(&["libgstapp-1.0.so.0", "libgstapp-1.0.so"])?;
        let gobject = open_lib(&["libgobject-2.0.so.0", "libgobject-2.0.so"])?;
        let glib = open_lib(&["libglib-2.0.so.0", "libglib-2.0.so"])?;
        let t = Gst {
            init_check: sym!(gst, "gst_init_check"),
            element_factory_make: sym!(gst, "gst_element_factory_make"),
            parse_launch: sym!(gst, "gst_parse_launch"),
            bin_get_by_name: sym!(gst, "gst_bin_get_by_name"),
            element_set_state: sym!(gst, "gst_element_set_state"),
            element_get_state: sym!(gst, "gst_element_get_state"),
            element_query_duration: sym!(gst, "gst_element_query_duration"),
            element_query_position: sym!(gst, "gst_element_query_position"),
            element_seek_simple: sym!(gst, "gst_element_seek_simple"),
            element_get_bus: sym!(gst, "gst_element_get_bus"),
            element_query: sym!(gst, "gst_element_query"),
            query_new_seeking: sym!(gst, "gst_query_new_seeking"),
            query_parse_seeking: sym!(gst, "gst_query_parse_seeking"),
            bus_timed_pop_filtered: sym!(gst, "gst_bus_timed_pop_filtered"),
            message_parse_error: sym!(gst, "gst_message_parse_error"),
            object_ref: sym!(gst, "gst_object_ref"),
            object_unref: sym!(gst, "gst_object_unref"),
            mini_object_unref: sym!(gst, "gst_mini_object_unref"),
            caps_from_string: sym!(gst, "gst_caps_from_string"),
            caps_get_structure: sym!(gst, "gst_caps_get_structure"),
            structure_get_int: sym!(gst, "gst_structure_get_int"),
            structure_get_fraction: sym!(gst, "gst_structure_get_fraction"),
            structure_get_string: sym!(gst, "gst_structure_get_string"),
            sample_get_buffer: sym!(gst, "gst_sample_get_buffer"),
            sample_get_caps: sym!(gst, "gst_sample_get_caps"),
            buffer_map: sym!(gst, "gst_buffer_map"),
            buffer_unmap: sym!(gst, "gst_buffer_unmap"),
            buffer_new_allocate: sym!(gst, "gst_buffer_new_allocate"),
            app_sink_try_pull_sample: sym!(app, "gst_app_sink_try_pull_sample"),
            app_sink_is_eos: sym!(app, "gst_app_sink_is_eos"),
            app_src_push_buffer: sym!(app, "gst_app_src_push_buffer"),
            app_src_end_of_stream: sym!(app, "gst_app_src_end_of_stream"),
            object_set: sym!(gobject, "g_object_set"),
            object_get: sym!(gobject, "g_object_get"),
            error_free: sym!(glib, "g_error_free"),
            free: sym!(glib, "g_free"),
            filename_to_uri: sym!(gst, "gst_filename_to_uri"),
            _libs: vec![gst, app, gobject, glib],
        };
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: gst_init_check with no arguments, reporting into `err`.
        let ok = unsafe { (t.init_check)(std::ptr::null_mut(), std::ptr::null_mut(), &mut err) };
        if ok == 0 {
            return Err(format!("GStreamer did not start: {}", t.take_error(err)));
        }
        Ok(t)
    }

    /// The message of a GError, which is freed.
    fn take_error(&self, err: *mut GError) -> String {
        if err.is_null() {
            return "no reason given".into();
        }
        // SAFETY: a GError GLib handed over; its message is a C string; freed once.
        unsafe {
            let msg = if (*err).message.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*err).message).to_string_lossy().into_owned()
            };
            (self.error_free)(err);
            msg
        }
    }

    /// The first error posted on `element`'s bus since the last look, if any. Every other
    /// message waiting there (state changes, tags, stream starts) is let go, so the bus never
    /// grows while media plays.
    fn bus_error(&self, element: Ptr) -> Option<(c_int, String)> {
        let mut found = None;
        // SAFETY: bus calls on a live element; each message and error is freed once.
        unsafe {
            let bus = (self.element_get_bus)(element);
            if bus.is_null() {
                return None;
            }
            loop {
                let msg = (self.bus_timed_pop_filtered)(bus, 0, GST_MESSAGE_ANY);
                if msg.is_null() {
                    break;
                }
                if found.is_none() && (*msg.cast::<GstMessage>()).type_ & GST_MESSAGE_ERROR != 0 {
                    let mut err: *mut GError = std::ptr::null_mut();
                    let mut debug: *mut c_char = std::ptr::null_mut();
                    (self.message_parse_error)(msg, &mut err, &mut debug);
                    if !debug.is_null() {
                        (self.free)(debug.cast());
                    }
                    let code = if err.is_null() { 0 } else { (*err).code };
                    found = Some((code, self.take_error(err)));
                }
                (self.mini_object_unref)(msg);
            }
            (self.object_unref)(bus);
        }
        found
    }

    fn make(&self, factory: &str) -> Result<Ptr, MediaError> {
        let f = CString::new(factory).map_err(|e| MediaError::Unsupported(e.to_string()))?;
        // SAFETY: creating an element by factory name.
        let e = unsafe { (self.element_factory_make)(f.as_ptr(), std::ptr::null()) };
        if e.is_null() {
            return Err(MediaError::Unsupported(format!(
                "the GStreamer element `{factory}` is not installed (gst-plugins-base)"
            )));
        }
        Ok(e)
    }

    fn caps(&self, s: &str) -> Result<Ptr, MediaError> {
        let c = CString::new(s).map_err(|e| MediaError::Unsupported(e.to_string()))?;
        // SAFETY: parsing a caps string.
        let caps = unsafe { (self.caps_from_string)(c.as_ptr()) };
        if caps.is_null() {
            return Err(MediaError::Unsupported(format!("GStreamer could not read the caps `{s}`")));
        }
        Ok(caps)
    }

    /// An app sink taking `caps`, as fast as it is pulled, holding at most what is given.
    fn app_sink(&self, caps: &str, max_buffers: c_uint) -> Result<Ptr, MediaError> {
        let sink = self.make("appsink")?;
        let caps = self.caps(caps)?;
        // SAFETY: setting documented appsink properties with their C types; the caps are
        // copied by the property and then released.
        unsafe {
            (self.object_set)(sink, c"caps".as_ptr(), caps, std::ptr::null::<c_char>());
            (self.mini_object_unref)(caps);
            (self.object_set)(sink, c"sync".as_ptr(), 0 as GBoolean, std::ptr::null::<c_char>());
            (self.object_set)(sink, c"drop".as_ptr(), 0 as GBoolean, std::ptr::null::<c_char>());
            (self.object_set)(sink, c"max-buffers".as_ptr(), max_buffers, std::ptr::null::<c_char>());
            // Keep our own reference: playbin takes the floating one.
            (self.object_ref)(sink);
        }
        Ok(sink)
    }

    fn get_int(&self, object: Ptr, prop: &CStr) -> c_int {
        let mut v: c_int = 0;
        // SAFETY: reading an int property into a local.
        unsafe { (self.object_get)(object, prop.as_ptr(), &mut v as *mut c_int, std::ptr::null::<c_char>()) };
        v
    }
}

fn gst() -> Result<&'static Gst, MediaError> {
    static GST: OnceLock<Result<Gst, String>> = OnceLock::new();
    GST.get_or_init(Gst::load).as_ref().map_err(|e| {
        MediaError::Unsupported(format!(
            "GStreamer 1.x is not installed or could not start ({e}); install gstreamer1.0-plugins-base \
             and the codecs you need (gstreamer1.0-libav)"
        ))
    })
}

struct GstHost;

pub(super) fn host() -> Result<Arc<dyn MediaHost>, MediaError> {
    gst()?;
    Ok(Arc::new(GstHost))
}

impl MediaHost for GstHost {
    fn name(&self) -> &str {
        "GStreamer"
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Ok(Box::new(GstDecoder::open(gst()?, source)?))
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(GstInput::open(gst()?, format)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(GstOutput::open(gst()?, format)?))
    }
}

/// One stream's app sink and the sample it has ready.
struct Branch {
    sink: Ptr,
    head: Option<Ptr>,
    done: bool,
}

struct GstDecoder {
    g: &'static Gst,
    playbin: Ptr,
    video: Option<Branch>,
    audio: Option<Branch>,
    text: Option<Branch>,
    channels: usize,
    info: MediaInfo,
    matrix: YuvMatrix,
    full_range: bool,
}

// SAFETY: GStreamer elements are thread-safe objects; the decoder is used from one thread at
// a time.
unsafe impl Send for GstDecoder {}

fn map_error(code: c_int, text: String) -> MediaError {
    if code == RESOURCE_NOT_FOUND || code == RESOURCE_OPEN_READ {
        MediaError::NotFound(text)
    } else {
        MediaError::Format(text)
    }
}

impl GstDecoder {
    fn open(g: &'static Gst, source: &MediaSource) -> Result<GstDecoder, MediaError> {
        let uri = match source {
            MediaSource::File(p) => {
                if !p.exists() {
                    return Err(MediaError::NotFound(p.display().to_string()));
                }
                let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
                let c = CString::new(abs.to_string_lossy().as_bytes())
                    .map_err(|e| MediaError::NotFound(e.to_string()))?;
                let mut err: *mut GError = std::ptr::null_mut();
                // SAFETY: converting a path; the result is a g_malloc'd string, freed below.
                let u = unsafe { (g.filename_to_uri)(c.as_ptr(), &mut err) };
                if u.is_null() {
                    return Err(MediaError::NotFound(g.take_error(err)));
                }
                // SAFETY: as above.
                let s = unsafe { CStr::from_ptr(u) }.to_string_lossy().into_owned();
                // SAFETY: freeing what GLib allocated.
                unsafe { (g.free)(u.cast()) };
                s
            }
            MediaSource::Url(u) => u.clone(),
        };
        let uri = CString::new(uri).map_err(|e| MediaError::NotFound(e.to_string()))?;
        let playbin = g.make("playbin")?;
        let mut d = GstDecoder {
            g,
            playbin,
            video: None,
            audio: None,
            text: None,
            channels: 0,
            info: MediaInfo::default(),
            matrix: YuvMatrix::Bt709,
            full_range: false,
        };
        let vsink = g.app_sink("video/x-raw,format=NV12", SINK_PICTURES)?;
        d.video = Some(Branch { sink: vsink, head: None, done: false });
        let asink = g.app_sink("audio/x-raw,format=F32LE,layout=interleaved", SINK_SOUND)?;
        d.audio = Some(Branch { sink: asink, head: None, done: false });
        let tsink = g.app_sink("text/x-raw", 0)?;
        d.text = Some(Branch { sink: tsink, head: None, done: false });
        // SAFETY: documented playbin properties with their C types.
        unsafe {
            (g.object_set)(playbin, c"uri".as_ptr(), uri.as_ptr(), std::ptr::null::<c_char>());
            (g.object_set)(playbin, c"video-sink".as_ptr(), vsink, std::ptr::null::<c_char>());
            (g.object_set)(playbin, c"audio-sink".as_ptr(), asink, std::ptr::null::<c_char>());
            (g.object_set)(playbin, c"text-sink".as_ptr(), tsink, std::ptr::null::<c_char>());
        }
        // SAFETY: state changes on a live element.
        let changed = unsafe { (g.element_set_state)(playbin, GST_STATE_PAUSED) };
        if changed == GST_STATE_CHANGE_FAILURE {
            let (code, text) = g.bus_error(playbin).unwrap_or((0, "the pipeline would not start".into()));
            return Err(map_error(code, text));
        }
        let ns = u64::try_from(PREROLL_WAIT.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: waiting for the state change on a live element.
        let state = unsafe { (g.element_get_state)(playbin, std::ptr::null_mut(), std::ptr::null_mut(), ns) };
        if state == GST_STATE_CHANGE_FAILURE {
            let (code, text) = g.bus_error(playbin).unwrap_or((0, "the source could not be read".into()));
            return Err(map_error(code, text));
        }
        if let Some((code, text)) = g.bus_error(playbin) {
            return Err(map_error(code, text));
        }
        let (nv, na, nt) =
            (g.get_int(playbin, c"n-video"), g.get_int(playbin, c"n-audio"), g.get_int(playbin, c"n-text"));
        if nv == 0 {
            d.video = d.video.take().and_then(|b| d.drop_branch(b));
        }
        if na == 0 {
            d.audio = d.audio.take().and_then(|b| d.drop_branch(b));
        }
        if nt == 0 {
            d.text = d.text.take().and_then(|b| d.drop_branch(b));
        } else {
            // playbin feeds the text sink from the one subtitle stream it selects.
            d.info.text_tracks.push(TextTrackInfo { label: "Embedded subtitles".into(), language: None });
        }
        if d.video.is_none() && d.audio.is_none() {
            return Err(MediaError::Format(
                "no sound or video stream this system can decode (a GStreamer plugin may be missing)".into(),
            ));
        }
        // Decoding runs as fast as the sinks are pulled (they do not sync to a clock); app
        // sinks hand out samples only while playing.
        // SAFETY: a state change on a live element.
        unsafe { (g.element_set_state)(playbin, GST_STATE_PLAYING) };
        // What the first samples say about the streams.
        d.describe()?;
        let mut dur: i64 = -1;
        // SAFETY: queries into locals on a live element.
        unsafe {
            if (g.element_query_duration)(playbin, GST_FORMAT_TIME, &mut dur) != 0 && dur > 0 {
                d.info.duration = Some(Duration::from_nanos(dur as u64));
            }
            let q = (g.query_new_seeking)(GST_FORMAT_TIME);
            if !q.is_null() {
                if (g.element_query)(playbin, q) != 0 {
                    let mut seekable: GBoolean = 0;
                    (g.query_parse_seeking)(
                        q,
                        std::ptr::null_mut(),
                        &mut seekable,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    d.info.seekable = seekable != 0;
                }
                (g.mini_object_unref)(q);
            }
        }
        Ok(d)
    }

    fn drop_branch(&self, b: Branch) -> Option<Branch> {
        // SAFETY: releasing the reference taken in `app_sink`.
        unsafe { (self.g.object_unref)(b.sink) };
        None
    }

    /// Reads the formats from the first sample of each stream (kept as that stream's head).
    fn describe(&mut self) -> Result<(), MediaError> {
        let g = self.g;
        let wait = u64::try_from(PREROLL_WAIT.as_nanos()).unwrap_or(u64::MAX);
        if let Some(b) = self.video.as_mut() {
            // SAFETY: pulling the preroll sample and reading its caps' fields into locals.
            unsafe {
                let s = (g.app_sink_try_pull_sample)(b.sink, wait);
                if s.is_null() {
                    return Err(MediaError::Format("the video gave no first picture".into()));
                }
                b.head = Some(s);
                let st = (g.caps_get_structure)((g.sample_get_caps)(s), 0);
                let (mut w, mut h, mut n, mut dd) = (0, 0, 0, 0);
                (g.structure_get_int)(st, c"width".as_ptr(), &mut w);
                (g.structure_get_int)(st, c"height".as_ptr(), &mut h);
                let fps = ((g.structure_get_fraction)(st, c"framerate".as_ptr(), &mut n, &mut dd) != 0
                    && n > 0
                    && dd > 0)
                    .then(|| f64::from(n) / f64::from(dd));
                let colorimetry = (g.structure_get_string)(st, c"colorimetry".as_ptr());
                let c = if colorimetry.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(colorimetry).to_string_lossy().into_owned()
                };
                // "bt709", "bt601", "bt2020", "sRGB", or "range:matrix:transfer:primaries"
                // with range 1 = full and matrix 3 = BT.709, 4 = BT.601.
                let sd =
                    c.starts_with("bt601") || c.split(':').nth(1) == Some("4") || (c.is_empty() && h < 720);
                self.matrix = if sd { YuvMatrix::Bt601 } else { YuvMatrix::Bt709 };
                self.full_range = c.split(':').next() == Some("1") || c == "jpeg";
                if w <= 0 || h <= 0 {
                    return Err(MediaError::Format("the video has no size".into()));
                }
                self.info.video = Some(VideoInfo { width: w as u32, height: h as u32, frame_rate: fps });
            }
        }
        if let Some(b) = self.audio.as_mut() {
            // SAFETY: as above.
            unsafe {
                let s = (g.app_sink_try_pull_sample)(b.sink, wait);
                if s.is_null() {
                    return Err(MediaError::Format("the sound gave no first samples".into()));
                }
                b.head = Some(s);
                let st = (g.caps_get_structure)((g.sample_get_caps)(s), 0);
                let (mut rate, mut ch) = (0, 0);
                (g.structure_get_int)(st, c"rate".as_ptr(), &mut rate);
                (g.structure_get_int)(st, c"channels".as_ptr(), &mut ch);
                if rate <= 0 || ch <= 0 {
                    return Err(MediaError::Format("the sound has no rate or channels".into()));
                }
                self.channels = ch as usize;
                self.info.audio = Some(AudioFormat {
                    sample_rate: rate as u32,
                    channels: u16::try_from(ch).unwrap_or(u16::MAX),
                });
            }
        }
        Ok(())
    }

    /// Turns a pulled sample into one of ours, releasing it.
    fn convert(&self, which: usize, s: Ptr) -> Result<Option<Sample>, MediaError> {
        let g = self.g;
        // SAFETY: the sample is ours until unref'd at the end; its buffer is mapped for reading
        // while copied and unmapped after.
        unsafe {
            let buf = (g.sample_get_buffer)(s);
            if buf.is_null() {
                (g.mini_object_unref)(s);
                return Ok(None);
            }
            let pts = (*buf).pts;
            let duration = (*buf).duration;
            let at = if pts == CLOCK_TIME_NONE { Duration::ZERO } else { Duration::from_nanos(pts) };
            let mut map = GstMapInfo::default();
            if (g.buffer_map)(buf, &mut map, GST_MAP_READ) == 0 {
                (g.mini_object_unref)(s);
                return Err(MediaError::Decode("a decoded buffer could not be read".into()));
            }
            let bytes: &[u8] =
                if map.data.is_null() { &[] } else { std::slice::from_raw_parts(map.data, map.size) };
            let out = match which {
                0 => self.picture(s, bytes).map(|picture| {
                    let duration = (duration != CLOCK_TIME_NONE).then(|| Duration::from_nanos(duration));
                    Some(Sample::Video { at, duration, picture })
                }),
                1 => {
                    let samples: Vec<f32> =
                        bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
                    let whole = samples.len() - samples.len() % self.channels.max(1);
                    Ok((whole > 0).then(|| Sample::Audio {
                        at,
                        samples: samples.get(..whole).unwrap_or_default().to_vec(),
                    }))
                }
                _ => {
                    let text = strip_markup(&String::from_utf8_lossy(bytes));
                    let until =
                        if duration == CLOCK_TIME_NONE { at } else { at + Duration::from_nanos(duration) };
                    Ok((!text.is_empty() && until > at).then_some(Sample::Text { track: 0, at, until, text }))
                }
            };
            (g.buffer_unmap)(buf, &mut map);
            (g.mini_object_unref)(s);
            out
        }
    }

    /// An NV12 picture in GStreamer's default layout: rows padded to 4 bytes, the chroma
    /// plane after the luma's (even) rows.
    fn picture(&self, s: Ptr, bytes: &[u8]) -> Result<Picture, MediaError> {
        let g = self.g;
        let (mut w, mut h) = (0, 0);
        // SAFETY: reading the sample's caps fields into locals.
        unsafe {
            let st = (g.caps_get_structure)((g.sample_get_caps)(s), 0);
            (g.structure_get_int)(st, c"width".as_ptr(), &mut w);
            (g.structure_get_int)(st, c"height".as_ptr(), &mut h);
        }
        let (w, h) = (usize::try_from(w).unwrap_or(0), usize::try_from(h).unwrap_or(0));
        let stride = w.div_ceil(4) * 4;
        let rows = h.div_ceil(2) * 2;
        let need = stride * rows + stride * rows / 2;
        if w == 0 || h == 0 || bytes.len() < need {
            return Err(MediaError::Decode(format!(
                "an NV12 picture of {w}x{h} came in {} bytes",
                bytes.len()
            )));
        }
        Ok(Picture {
            width: w as u32,
            height: h as u32,
            format: PixelFormat::Nv12,
            planes: [Plane { offset: 0, stride }, Plane { offset: stride * rows, stride }, Plane::default()],
            matrix: self.matrix,
            full_range: self.full_range,
            data: Arc::from(bytes.get(..need).unwrap_or_default()),
        })
    }

    /// Fills each branch's head with what its sink has ready. With nothing ready anywhere, it
    /// waits up to `wait` on the pictures (or the sound, without pictures) before returning.
    fn fill(&mut self, wait: Duration) {
        let g = self.g;
        let mut any = false;
        for b in [self.video.as_mut(), self.audio.as_mut(), self.text.as_mut()].into_iter().flatten() {
            if b.head.is_none() && !b.done {
                // SAFETY: pulling from a live app sink without waiting.
                let s = unsafe { (g.app_sink_try_pull_sample)(b.sink, 0) };
                if s.is_null() {
                    // SAFETY: as above.
                    b.done = unsafe { (g.app_sink_is_eos)(b.sink) } != 0;
                } else {
                    b.head = Some(s);
                }
            }
            any |= b.head.is_some();
        }
        if any {
            return;
        }
        let ns = u64::try_from(wait.as_nanos()).unwrap_or(0);
        if let Some(b) = [self.video.as_mut(), self.audio.as_mut()].into_iter().flatten().find(|b| !b.done) {
            // SAFETY: pulling from a live app sink, waiting at most `wait`.
            let s = unsafe { (g.app_sink_try_pull_sample)(b.sink, ns) };
            if !s.is_null() {
                b.head = Some(s);
            }
        }
    }

    fn heads_at(&self) -> [Option<u64>; 3] {
        let g = self.g;
        let at = |b: &Option<Branch>| {
            b.as_ref().and_then(|b| b.head).map(|s| {
                // SAFETY: reading the timestamp of a held sample's buffer.
                unsafe {
                    let buf = (g.sample_get_buffer)(s);
                    if buf.is_null() || (*buf).pts == CLOCK_TIME_NONE { 0 } else { (*buf).pts }
                }
            })
        };
        [at(&self.video), at(&self.audio), at(&self.text)]
    }

    fn release_heads(&mut self) {
        let g = self.g;
        for b in [self.video.as_mut(), self.audio.as_mut(), self.text.as_mut()].into_iter().flatten() {
            if let Some(s) = b.head.take() {
                // SAFETY: releasing a held sample.
                unsafe { (g.mini_object_unref)(s) };
            }
            b.done = false;
        }
    }
}

/// Pango markup and HTML-like tags removed, entities decoded.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

impl Decoder for GstDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        loop {
            if let Some((code, text)) = self.g.bus_error(self.playbin) {
                return Err(match map_error(code, text) {
                    MediaError::Format(t) => MediaError::Decode(t),
                    e => e,
                });
            }
            self.fill(POLL);
            let heads = self.heads_at();
            // The earliest sample ready goes first.
            let pick = heads
                .iter()
                .enumerate()
                .filter_map(|(i, t)| t.map(|t| (i, t)))
                .min_by_key(|&(_, t)| t)
                .map(|p| p.0);
            let Some(i) = pick else {
                let all_done =
                    [&self.video, &self.audio, &self.text].iter().all(|b| b.as_ref().is_none_or(|b| b.done));
                if all_done {
                    return Ok(None);
                }
                continue;
            };
            let branch = match i {
                0 => self.video.as_mut(),
                1 => self.audio.as_mut(),
                _ => self.text.as_mut(),
            };
            let Some(s) = branch.and_then(|b| b.head.take()) else { continue };
            if let Some(sample) = self.convert(i, s)? {
                return Ok(Some(sample));
            }
        }
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.release_heads();
        let ns = i64::try_from(to.as_nanos()).unwrap_or(i64::MAX);
        // SAFETY: a flushing seek on a live pipeline.
        let ok = unsafe {
            (self.g.element_seek_simple)(
                self.playbin,
                GST_FORMAT_TIME,
                GST_SEEK_FLAG_FLUSH | GST_SEEK_FLAG_ACCURATE,
                ns,
            )
        };
        if ok == 0 {
            return Err(MediaError::Decode("GStreamer refused the seek".into()));
        }
        Ok(())
    }
}

impl Drop for GstDecoder {
    fn drop(&mut self) {
        self.release_heads();
        let g = self.g;
        // SAFETY: stopping and releasing the pipeline and the sink references taken here.
        unsafe {
            (g.element_set_state)(self.playbin, GST_STATE_NULL);
            for b in [self.video.take(), self.audio.take(), self.text.take()].into_iter().flatten() {
                (g.object_unref)(b.sink);
            }
            (g.object_unref)(self.playbin);
        }
    }
}

/// Sound out through `appsrc ! audioconvert ! audioresample ! autoaudiosink`.
struct GstOutput {
    g: &'static Gst,
    pipeline: Ptr,
    src: Ptr,
    format: AudioFormat,
    queue_bytes: u64,
    written: u64,
    playing: bool,
}

// SAFETY: as for the decoder.
unsafe impl Send for GstOutput {}

impl GstOutput {
    fn open(g: &'static Gst, format: AudioFormat) -> Result<GstOutput, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        let desc = format!(
            "appsrc name=src format=time is-live=false block=false \
             caps=audio/x-raw,format=F32LE,layout=interleaved,rate={},channels={} \
             ! audioconvert ! audioresample ! autoaudiosink",
            format.sample_rate, format.channels
        );
        let desc = CString::new(desc).map_err(|e| MediaError::Device(e.to_string()))?;
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: building a pipeline from a description; errors come back in `err`.
        let pipeline = unsafe { (g.parse_launch)(desc.as_ptr(), &mut err) };
        if pipeline.is_null() {
            return Err(MediaError::Device(g.take_error(err)));
        }
        if !err.is_null() {
            // A recoverable warning (an element missing a property): report it.
            let msg = g.take_error(err);
            // SAFETY: releasing the half-made pipeline.
            unsafe { (g.object_unref)(pipeline) };
            return Err(MediaError::Device(msg));
        }
        // SAFETY: looking up the named source in the bin (a new reference).
        let src = unsafe { (g.bin_get_by_name)(pipeline, c"src".as_ptr()) };
        if src.is_null() {
            // SAFETY: as above.
            unsafe { (g.object_unref)(pipeline) };
            return Err(MediaError::Device("the sound pipeline has no source".into()));
        }
        let queue_bytes = format.frames_in(OUTPUT_QUEUE) * u64::from(format.channels) * 4;
        // SAFETY: a documented appsrc property and a state change on a live pipeline.
        unsafe {
            (g.object_set)(src, c"max-bytes".as_ptr(), queue_bytes, std::ptr::null::<c_char>());
            if (g.element_set_state)(pipeline, GST_STATE_PAUSED) == GST_STATE_CHANGE_FAILURE {
                let reason = g.bus_error(pipeline).map_or("no sound output could be opened".into(), |e| e.1);
                (g.element_set_state)(pipeline, GST_STATE_NULL);
                (g.object_unref)(src);
                (g.object_unref)(pipeline);
                return Err(MediaError::Device(reason));
            }
        }
        Ok(GstOutput { g, pipeline, src, format, queue_bytes, written: 0, playing: false })
    }

    fn check(&self) -> Result<(), MediaError> {
        match self.g.bus_error(self.pipeline) {
            Some((_, text)) => Err(MediaError::Device(text)),
            None => Ok(()),
        }
    }
}

impl AudioOutput for GstOutput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn buffer(&self) -> Duration {
        OUTPUT_QUEUE
    }

    fn room(&mut self) -> Result<usize, MediaError> {
        self.check()?;
        let mut level: u64 = 0;
        // SAFETY: reading a guint64 property into a local.
        unsafe {
            (self.g.object_get)(
                self.src,
                c"current-level-bytes".as_ptr(),
                &mut level as *mut u64,
                std::ptr::null::<c_char>(),
            )
        };
        let frame = u64::from(self.format.channels) * 4;
        Ok(usize::try_from(self.queue_bytes.saturating_sub(level) / frame).unwrap_or(0))
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let ch = usize::from(self.format.channels);
        let frames = (samples.len() / ch) as u64;
        if frames == 0 {
            return Ok(());
        }
        let bytes = frames as usize * ch * 4;
        let g = self.g;
        // SAFETY: a new buffer of `bytes`, mapped for writing while filled, then handed to
        // the source (which takes ownership).
        unsafe {
            let buf = (g.buffer_new_allocate)(std::ptr::null_mut(), bytes, std::ptr::null_mut());
            if buf.is_null() {
                return Err(MediaError::Device("no memory for a sound buffer".into()));
            }
            let mut map = GstMapInfo::default();
            if (g.buffer_map)(buf, &mut map, GST_MAP_WRITE) == 0 || map.data.is_null() || map.size < bytes {
                (g.mini_object_unref)(buf.cast());
                return Err(MediaError::Device("a sound buffer could not be written".into()));
            }
            std::ptr::copy_nonoverlapping(samples.as_ptr().cast::<u8>(), map.data, bytes);
            (g.buffer_unmap)(buf, &mut map);
            let start = self.format.duration_of(self.written);
            let end = self.format.duration_of(self.written + frames);
            (*buf).pts = u64::try_from(start.as_nanos()).unwrap_or(u64::MAX);
            (*buf).duration = u64::try_from((end - start).as_nanos()).unwrap_or(u64::MAX);
            if (g.app_src_push_buffer)(self.src, buf) != 0 {
                return Err(MediaError::Device("the sound output stopped taking sound".into()));
            }
        }
        self.written += frames;
        Ok(())
    }

    fn played(&mut self) -> Result<u64, MediaError> {
        self.check()?;
        let mut pos: i64 = 0;
        // SAFETY: a position query into a local.
        let ok = unsafe { (self.g.element_query_position)(self.pipeline, GST_FORMAT_TIME, &mut pos) };
        if ok == 0 || pos < 0 {
            return Ok(0);
        }
        // Rounded to the nearest frame: the sink reports times, and the last frame's time
        // rounded down would never reach the end of what was written.
        let frames =
            (u128::from(pos as u64) * u128::from(self.format.sample_rate) + 500_000_000) / 1_000_000_000;
        Ok(u64::try_from(frames).unwrap_or(u64::MAX).min(self.written))
    }

    fn pause(&mut self) -> Result<(), MediaError> {
        if self.playing {
            // SAFETY: a state change on a live pipeline.
            unsafe { (self.g.element_set_state)(self.pipeline, GST_STATE_PAUSED) };
            self.playing = false;
        }
        self.check()
    }

    fn resume(&mut self) -> Result<(), MediaError> {
        if !self.playing {
            // SAFETY: as above.
            let r = unsafe { (self.g.element_set_state)(self.pipeline, GST_STATE_PLAYING) };
            if r == GST_STATE_CHANGE_FAILURE {
                return Err(self
                    .check()
                    .err()
                    .unwrap_or(MediaError::Device("the sound output would not play".into())));
            }
            self.playing = true;
        }
        self.check()
    }

    fn finish(&mut self) -> Result<(), MediaError> {
        // The end of the stream makes the sink play out its last partial segment.
        // SAFETY: signalling the end on a live app source.
        if unsafe { (self.g.app_src_end_of_stream)(self.src) } != 0 {
            return Err(MediaError::Device("the sound output would not take the end of the stream".into()));
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        // Back to READY drops everything queued and starts a new stream at time zero.
        // SAFETY: state changes on a live pipeline.
        unsafe {
            (self.g.element_set_state)(self.pipeline, GST_STATE_READY);
            (self.g.element_set_state)(self.pipeline, GST_STATE_PAUSED);
        }
        self.playing = false;
        self.written = 0;
        self.check()
    }
}

impl Drop for GstOutput {
    fn drop(&mut self) {
        // SAFETY: stopping and releasing what `open` made.
        unsafe {
            (self.g.element_set_state)(self.pipeline, GST_STATE_NULL);
            (self.g.object_unref)(self.src);
            (self.g.object_unref)(self.pipeline);
        }
    }
}

/// The default microphone through `autoaudiosrc ! audioconvert ! audioresample ! appsink`:
/// whatever the system's sound server is (PipeWire, PulseAudio, ALSA), converted to the format
/// asked for.
struct GstInput {
    g: &'static Gst,
    pipeline: Ptr,
    sink: Ptr,
    format: AudioFormat,
}

// SAFETY: as for the decoder: the pipeline is used from one thread at a time.
unsafe impl Send for GstInput {}

/// How often the source hands over sound: 10 ms buffers, a device period's worth.
const INPUT_PERIOD: Duration = Duration::from_millis(10);

impl GstInput {
    fn open(g: &'static Gst, format: AudioFormat) -> Result<GstInput, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        // Half a second of sound waits in the sink at most, for a reader that is late; a
        // reader that keeps up never lets it fill.
        let max_buffers = 50;
        let desc = format!(
            "autoaudiosrc ! audioconvert ! audioresample \
             ! appsink name=sink sync=false max-buffers={max_buffers} drop=false \
             caps=audio/x-raw,format=F32LE,layout=interleaved,rate={rate},channels={channels}",
            rate = format.sample_rate,
            channels = format.channels,
        );
        let desc = CString::new(desc).map_err(|e| MediaError::Device(e.to_string()))?;
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: building a pipeline from a description; errors come back in `err`.
        let pipeline = unsafe { (g.parse_launch)(desc.as_ptr(), &mut err) };
        if pipeline.is_null() {
            return Err(MediaError::Device(g.take_error(err)));
        }
        if !err.is_null() {
            let msg = g.take_error(err);
            // SAFETY: releasing the half-made pipeline.
            unsafe { (g.object_unref)(pipeline) };
            return Err(MediaError::Device(msg));
        }
        // SAFETY: looking up the named sink in the bin (a new reference).
        let sink = unsafe { (g.bin_get_by_name)(pipeline, c"sink".as_ptr()) };
        if sink.is_null() {
            // SAFETY: as above.
            unsafe { (g.object_unref)(pipeline) };
            return Err(MediaError::Device("the microphone pipeline has no sink".into()));
        }
        // SAFETY: a state change on a live pipeline; on failure everything made is released.
        unsafe {
            if (g.element_set_state)(pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE {
                let reason = g.bus_error(pipeline).map_or("no microphone could be opened".into(), |e| e.1);
                (g.element_set_state)(pipeline, GST_STATE_NULL);
                (g.object_unref)(sink);
                (g.object_unref)(pipeline);
                return Err(MediaError::NotFound(reason));
            }
        }
        Ok(GstInput { g, pipeline, sink, format })
    }
}

impl AudioInput for GstInput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn period(&self) -> Duration {
        INPUT_PERIOD
    }

    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError> {
        let g = self.g;
        if let Some((_, text)) = g.bus_error(self.pipeline) {
            return Err(MediaError::Device(text));
        }
        let ch = usize::from(self.format.channels);
        let mut frames = 0;
        loop {
            // SAFETY: a non-blocking pull from a live sink; the sample is ours until unref'd,
            // and its buffer is mapped for reading while copied.
            unsafe {
                let s = (g.app_sink_try_pull_sample)(self.sink, 0);
                if s.is_null() {
                    if (g.app_sink_is_eos)(self.sink) != 0 {
                        return Err(MediaError::Device("the microphone stopped".into()));
                    }
                    return Ok(frames);
                }
                let buf = (g.sample_get_buffer)(s);
                if !buf.is_null() {
                    let mut map = GstMapInfo::default();
                    if (g.buffer_map)(buf, &mut map, GST_MAP_READ) != 0 {
                        let bytes: &[u8] = if map.data.is_null() {
                            &[]
                        } else {
                            std::slice::from_raw_parts(map.data, map.size)
                        };
                        let samples = bytes.as_chunks::<4>().0;
                        let whole = samples.len() - samples.len() % ch.max(1);
                        into.extend(samples.iter().take(whole).map(|b| f32::from_le_bytes(*b)));
                        frames += whole / ch.max(1);
                        (g.buffer_unmap)(buf, &mut map);
                    }
                }
                (g.mini_object_unref)(s);
            }
        }
    }
}

impl Drop for GstInput {
    fn drop(&mut self) {
        // SAFETY: stopping and releasing what `open` made.
        unsafe {
            (self.g.element_set_state)(self.pipeline, GST_STATE_NULL);
            (self.g.object_unref)(self.sink);
            (self.g.object_unref)(self.pipeline);
        }
    }
}
