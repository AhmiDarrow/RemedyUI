//! Linux: the system's GStreamer, opened at run time (`libgstreamer-1.0.so.0` and friends) so
//! nothing is linked or bundled. Decoding is a `playbin` with app sinks for NV12 pictures,
//! interleaved float sound and subtitle text; it plays whatever the installed plugins can
//! (the distribution's codecs, hardware decoders through VA-API plugins). Sound goes out
//! through an `appsrc ! audioconvert ! audioresample ! autoaudiosink` pipeline, whose
//! position is the sound clock.
//!
//! When a renderer takes pictures on the GPU, the video sink asks for DMA-BUFs first and
//! hands them over unmapped (`gst_dmabuf.rs`).
//!
//! WSL cannot use the DMA-BUF path on the real GPU: Mesa's `dzn` (Vulkan on D3D12, which
//! Ubuntu does not ship) has none of `VK_EXT_external_memory_dma_buf`,
//! `VK_EXT_image_drm_format_modifier` or `VK_EXT_queue_family_foreign` (only opaque fds of
//! D3D12 heaps), WSL has no `/dev/dri` for VA-API, and NVDEC (`nvh264dec`) hands out CUDA or
//! GL memory. Vulkan there is llvmpipe, and with no decoder handing out DMA-BUFs pictures
//! are copied through CPU memory; the log says so once ("the decoder hands over its pictures
//! in CPU memory, not DMA-BUFs").

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use libloading::Library;

use crate::deps::MissingDependency;
use crate::host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture,
    PixelFormat, Plane, Sample, TextTrackInfo, VideoCodec, VideoInfo, VideoStreamDecoder, YuvMatrix,
};

pub(crate) type GBoolean = c_int;
pub(crate) type Ptr = *mut c_void;

pub(crate) const GST_FORMAT_TIME: c_int = 3;
/// Buffering ranges in hundred-thousandths of a percent of the whole (`GST_FORMAT_PERCENT`).
const GST_FORMAT_PERCENT: c_int = 5;
const GST_FORMAT_PERCENT_MAX: f64 = 1_000_000.0;
/// `GST_PLAY_FLAG_DOWNLOAD`: playbin keeps what a progressive stream downloads.
const GST_PLAY_FLAG_DOWNLOAD: c_int = 0x80;
pub(crate) const GST_STATE_NULL: c_int = 1;
pub(crate) const GST_STATE_READY: c_int = 2;
pub(crate) const GST_STATE_PAUSED: c_int = 3;
pub(crate) const GST_STATE_PLAYING: c_int = 4;
pub(crate) const GST_STATE_CHANGE_FAILURE: c_int = 0;
pub(crate) const GST_SEEK_FLAG_FLUSH: c_int = 1;
pub(crate) const GST_SEEK_FLAG_ACCURATE: c_int = 2;
pub(crate) const GST_MESSAGE_ERROR: c_uint = 1 << 1;
pub(crate) const GST_MESSAGE_ANY: c_uint = !0;
pub(crate) const GST_MAP_READ: c_int = 1;
pub(crate) const GST_MAP_WRITE: c_int = 2;
pub(crate) const CLOCK_TIME_NONE: u64 = u64::MAX;
/// GST_RESOURCE_ERROR_NOT_FOUND and _OPEN_READ, the codes for a missing or unreadable source.
pub(crate) const RESOURCE_NOT_FOUND: c_int = 3;
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
pub(crate) struct GstMiniObject {
    pub(crate) type_: usize,
    pub(crate) refcount: c_int,
    pub(crate) lockstate: c_int,
    pub(crate) flags: c_uint,
    pub(crate) copy: Ptr,
    pub(crate) dispose: Ptr,
    pub(crate) free: Ptr,
    pub(crate) priv_uint: c_uint,
    pub(crate) priv_pointer: Ptr,
}

#[repr(C)]
pub(crate) struct GstBuffer {
    pub(crate) mini_object: GstMiniObject,
    pub(crate) pool: Ptr,
    pub(crate) pts: u64,
    pub(crate) dts: u64,
    pub(crate) duration: u64,
    pub(crate) offset: u64,
    pub(crate) offset_end: u64,
}

#[repr(C)]
struct GstMessage {
    mini_object: GstMiniObject,
    type_: c_uint,
}

#[repr(C)]
pub(crate) struct GstMapInfo {
    pub(crate) memory: Ptr,
    pub(crate) flags: c_int,
    pub(crate) data: *mut u8,
    pub(crate) size: usize,
    pub(crate) maxsize: usize,
    pub(crate) user_data: [Ptr; 4],
    pub(crate) reserved: [Ptr; 4],
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
pub(crate) struct GError {
    pub(crate) domain: u32,
    pub(crate) code: c_int,
    pub(crate) message: *mut c_char,
}

/// The GStreamer and GLib functions used, from the system's libraries (shared with the
/// stream encoder and capture in `gstreamer_stream.rs`).
pub(crate) struct Gst {
    pub(crate) init_check: unsafe extern "C" fn(*mut c_int, *mut c_void, *mut *mut GError) -> GBoolean,
    pub(crate) element_factory_make: unsafe extern "C" fn(*const c_char, *const c_char) -> Ptr,
    pub(crate) parse_launch: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> Ptr,
    pub(crate) bin_get_by_name: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    pub(crate) element_set_state: unsafe extern "C" fn(Ptr, c_int) -> c_int,
    pub(crate) element_get_state: unsafe extern "C" fn(Ptr, *mut c_int, *mut c_int, u64) -> c_int,
    pub(crate) element_query_duration: unsafe extern "C" fn(Ptr, c_int, *mut i64) -> GBoolean,
    pub(crate) element_query_position: unsafe extern "C" fn(Ptr, c_int, *mut i64) -> GBoolean,
    pub(crate) element_seek_simple: unsafe extern "C" fn(Ptr, c_int, c_int, i64) -> GBoolean,
    pub(crate) element_send_event: unsafe extern "C" fn(Ptr, Ptr) -> GBoolean,
    pub(crate) event_new_flush_start: unsafe extern "C" fn() -> Ptr,
    pub(crate) event_new_flush_stop: unsafe extern "C" fn(GBoolean) -> Ptr,
    pub(crate) element_get_bus: unsafe extern "C" fn(Ptr) -> Ptr,
    pub(crate) element_query: unsafe extern "C" fn(Ptr, Ptr) -> GBoolean,
    pub(crate) query_new_seeking: unsafe extern "C" fn(c_int) -> Ptr,
    pub(crate) query_parse_seeking: unsafe extern "C" fn(Ptr, *mut c_int, *mut GBoolean, *mut i64, *mut i64),
    pub(crate) query_new_buffering: unsafe extern "C" fn(c_int) -> Ptr,
    pub(crate) query_parse_buffering_range:
        unsafe extern "C" fn(Ptr, *mut c_int, *mut i64, *mut i64, *mut i64),
    pub(crate) query_get_n_buffering_ranges: unsafe extern "C" fn(Ptr) -> c_uint,
    pub(crate) query_parse_nth_buffering_range:
        unsafe extern "C" fn(Ptr, c_uint, *mut i64, *mut i64) -> GBoolean,
    pub(crate) bus_timed_pop_filtered: unsafe extern "C" fn(Ptr, u64, c_uint) -> Ptr,
    pub(crate) message_parse_error: unsafe extern "C" fn(Ptr, *mut *mut GError, *mut *mut c_char),
    pub(crate) object_ref: unsafe extern "C" fn(Ptr) -> Ptr,
    pub(crate) object_unref: unsafe extern "C" fn(Ptr),
    pub(crate) mini_object_unref: unsafe extern "C" fn(Ptr),
    pub(crate) caps_from_string: unsafe extern "C" fn(*const c_char) -> Ptr,
    pub(crate) caps_get_structure: unsafe extern "C" fn(Ptr, c_uint) -> Ptr,
    pub(crate) structure_get_int: unsafe extern "C" fn(Ptr, *const c_char, *mut c_int) -> GBoolean,
    pub(crate) structure_get_fraction:
        unsafe extern "C" fn(Ptr, *const c_char, *mut c_int, *mut c_int) -> GBoolean,
    pub(crate) structure_get_string: unsafe extern "C" fn(Ptr, *const c_char) -> *const c_char,
    pub(crate) sample_get_buffer: unsafe extern "C" fn(Ptr) -> *mut GstBuffer,
    pub(crate) sample_get_caps: unsafe extern "C" fn(Ptr) -> Ptr,
    pub(crate) buffer_map: unsafe extern "C" fn(*mut GstBuffer, *mut GstMapInfo, c_int) -> GBoolean,
    pub(crate) buffer_unmap: unsafe extern "C" fn(*mut GstBuffer, *mut GstMapInfo),
    pub(crate) buffer_new_allocate: unsafe extern "C" fn(Ptr, usize, Ptr) -> *mut GstBuffer,
    pub(crate) app_sink_try_pull_sample: unsafe extern "C" fn(Ptr, u64) -> Ptr,
    pub(crate) app_sink_is_eos: unsafe extern "C" fn(Ptr) -> GBoolean,
    pub(crate) app_src_push_buffer: unsafe extern "C" fn(Ptr, *mut GstBuffer) -> c_int,
    pub(crate) app_src_end_of_stream: unsafe extern "C" fn(Ptr) -> c_int,
    pub(crate) object_set: unsafe extern "C" fn(Ptr, *const c_char, ...),
    pub(crate) object_get: unsafe extern "C" fn(Ptr, *const c_char, ...),
    pub(crate) error_free: unsafe extern "C" fn(*mut GError),
    pub(crate) free: unsafe extern "C" fn(Ptr),
    pub(crate) filename_to_uri: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_char,
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
            element_send_event: sym!(gst, "gst_element_send_event"),
            event_new_flush_start: sym!(gst, "gst_event_new_flush_start"),
            event_new_flush_stop: sym!(gst, "gst_event_new_flush_stop"),
            element_get_bus: sym!(gst, "gst_element_get_bus"),
            element_query: sym!(gst, "gst_element_query"),
            query_new_seeking: sym!(gst, "gst_query_new_seeking"),
            query_parse_seeking: sym!(gst, "gst_query_parse_seeking"),
            query_new_buffering: sym!(gst, "gst_query_new_buffering"),
            query_parse_buffering_range: sym!(gst, "gst_query_parse_buffering_range"),
            query_get_n_buffering_ranges: sym!(gst, "gst_query_get_n_buffering_ranges"),
            query_parse_nth_buffering_range: sym!(gst, "gst_query_parse_nth_buffering_range"),
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
    pub(crate) fn take_error(&self, err: *mut GError) -> String {
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
    pub(crate) fn bus_error(&self, element: Ptr) -> Option<(c_int, String)> {
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

    pub(crate) fn make(&self, factory: &str) -> Result<Ptr, MediaError> {
        let f = CString::new(factory).map_err(|e| MediaError::Unsupported(e.to_string()))?;
        // SAFETY: creating an element by factory name.
        let e = unsafe { (self.element_factory_make)(f.as_ptr(), std::ptr::null()) };
        if e.is_null() {
            return Err(MediaError::Missing(crate::deps::element(
                factory,
                "playing and recording sound and video",
            )));
        }
        Ok(e)
    }

    pub(crate) fn caps(&self, s: &str) -> Result<Ptr, MediaError> {
        let c = CString::new(s).map_err(|e| MediaError::Unsupported(e.to_string()))?;
        // SAFETY: parsing a caps string.
        let caps = unsafe { (self.caps_from_string)(c.as_ptr()) };
        if caps.is_null() {
            return Err(MediaError::Unsupported(format!("GStreamer could not read the caps `{s}`")));
        }
        Ok(caps)
    }

    /// An app sink taking `caps`, as fast as it is pulled, holding at most what is given.
    pub(crate) fn app_sink(&self, caps: &str, max_buffers: c_uint) -> Result<Ptr, MediaError> {
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

    pub(crate) fn get_int(&self, object: Ptr, prop: &CStr) -> c_int {
        let mut v: c_int = 0;
        // SAFETY: reading an int property into a local.
        unsafe { (self.object_get)(object, prop.as_ptr(), &mut v as *mut c_int, std::ptr::null::<c_char>()) };
        v
    }
}

/// The system's GStreamer, loaded and started once for the process.
pub(crate) fn gst() -> Result<&'static Gst, MediaError> {
    static GST: OnceLock<Result<Gst, String>> = OnceLock::new();
    GST.get_or_init(Gst::load).as_ref().map_err(|e| {
        // Loaded but not started is installed and unusable; a library not found is absent.
        let d = crate::deps::gstreamer();
        MediaError::Missing(if e.starts_with("GStreamer did not start") { d.unusable(e.clone()) } else { d })
    })
}

struct GstHost {
    /// The last HLS stream's decoders, for the next.
    spare: crate::hls::Spare,
}

pub(super) fn host() -> Result<Arc<dyn MediaHost>, MediaError> {
    gst()?;
    Ok(Arc::new(GstHost { spare: crate::hls::Spare::default() }))
}

impl MediaHost for GstHost {
    fn name(&self) -> &str {
        "GStreamer"
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        if crate::hls::is_hls(source) {
            return crate::hls::open(
                &source.to_url(),
                Box::new(super::gstreamer_hls::GstFetch::new()?),
                &self.spare,
                || Ok(Box::new(super::gstreamer_hls::GstElementary::new()?)),
            );
        }
        Ok(Box::new(GstDecoder::open(gst()?, source)?))
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(GstInput::open(gst()?, format)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(GstOutput::open(gst()?, format, crate::host::Latency::Playback)?))
    }

    fn audio_output_with(
        &self,
        format: AudioFormat,
        latency: crate::host::Latency,
    ) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(GstOutput::open(gst()?, format, latency)?))
    }

    fn video_stream_decoder_choice(&self, codec: VideoCodec, hardware: bool) -> Result<String, MediaError> {
        GstStreamDecoder::choice(gst()?, codec, hardware).map(str::to_owned)
    }

    fn video_stream_decoder(
        &self,
        codec: VideoCodec,
        hardware: bool,
    ) -> Result<Box<dyn VideoStreamDecoder>, MediaError> {
        Ok(Box::new(GstStreamDecoder::open(gst()?, codec, hardware)?))
    }

    fn video_stream_encoder(
        &self,
        options: &crate::host::VideoEncoderOptions,
    ) -> Result<Box<dyn crate::host::VideoStreamEncoder>, MediaError> {
        Ok(Box::new(super::gstreamer_stream::GstStreamEncoder::open(options)?))
    }

    fn video_stream_encoder_choice(&self, codec: VideoCodec, hardware: bool) -> Result<String, MediaError> {
        let VideoCodec::H264 = codec;
        super::gstreamer_stream::GstStreamEncoder::choice(hardware)
    }

    fn screen_stream(
        &self,
        source: crate::host::ScreenSource,
    ) -> Result<Box<dyn crate::host::ScreenStream>, MediaError> {
        Ok(Box::new(super::gstreamer_stream::GstScreenStream::open(source)?))
    }

    fn output_capture(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(super::pulse_capture::PulseCapture::open(format)?))
    }
}

/// One stream's app sink and the sample it has ready.
struct Branch {
    sink: Ptr,
    head: Option<Ptr>,
    done: bool,
}

pub(crate) struct GstDecoder {
    g: &'static Gst,
    /// Counted while it lives (debug builds), as every pipeline here is.
    _live: super::Live,
    playbin: Ptr,
    video: Option<Branch>,
    audio: Option<Branch>,
    text: Option<Branch>,
    channels: usize,
    info: MediaInfo,
    matrix: YuvMatrix,
    full_range: bool,
    /// Set when the video sink asked for DMA-BUFs (a renderer imports them).
    dma: Option<&'static super::gst_dmabuf::Fns>,
}

// SAFETY: GStreamer elements are thread-safe objects; the decoder is used from one thread at
// a time.
unsafe impl Send for GstDecoder {}

/// Says once, in the log, why pictures go through CPU memory although a renderer takes them on
/// the GPU: a decoder, driver or format that cannot share. Playback is the same, only slower.
fn copy_path(why: &str) {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!("remedy-ui-media-playback: video pictures are copied through CPU memory: {why}");
    }
}

/// A pipeline description's error as a missing element when that is what it says (GStreamer
/// writes `no element "autoaudiosink"`), else `other` with the text.
pub(crate) fn launch_error(
    text: String,
    purpose: &str,
    other: impl FnOnce(String) -> MediaError,
) -> MediaError {
    match missing_element(&text) {
        Some(factory) => MediaError::Missing(crate::deps::element(&factory, purpose)),
        None => other(text),
    }
}

/// The factory a "no element" message names.
fn missing_element(text: &str) -> Option<String> {
    let rest = text.split("no element \"").nth(1)?;
    rest.split('"').next().filter(|f| !f.is_empty()).map(str::to_owned)
}

/// A file or stream whose format needs a plugin that is not installed: GStreamer's decoders
/// for the common formats are in its good, bad, ugly and FFmpeg sets.
fn missing_codec(text: &str) -> MediaError {
    let mut d = crate::deps::MissingDependency::not_installed(
        "a GStreamer plugin for this media's format",
        "playing this media",
        &[
            (
                "Debian/Ubuntu",
                "gstreamer1.0-libav gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly",
            ),
            (
                "Fedora",
                "gstreamer1-plugin-libav gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugins-ugly",
            ),
            ("Arch", "gst-libav gst-plugins-good gst-plugins-bad gst-plugins-ugly"),
        ],
    );
    d.name = format!("{} ({})", d.name, text.trim_end_matches('.'));
    MediaError::Missing(d)
}

pub(crate) fn map_error(code: c_int, text: String) -> MediaError {
    if text.contains("missing a plug-in")
        || text.contains("missing a plugin")
        || text.contains("No decoder available")
    {
        return missing_codec(&text);
    }
    if code == RESOURCE_NOT_FOUND || code == RESOURCE_OPEN_READ {
        MediaError::NotFound(text)
    } else {
        MediaError::Format(text)
    }
}

impl GstDecoder {
    fn open(g: &'static Gst, source: &MediaSource) -> Result<GstDecoder, MediaError> {
        GstDecoder::open_with(g, source, None)
    }

    /// As `open`, with `filter` (a floating bin) run on the decoded pictures before the video
    /// sink: tests put a DMA-BUF producer there where no hardware decoder is.
    pub(crate) fn open_with(
        g: &'static Gst,
        source: &MediaSource,
        filter: Option<Ptr>,
    ) -> Result<GstDecoder, MediaError> {
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
            MediaSource::Bytes(_) => {
                return Err(MediaError::Unsupported(
                    "bytes in memory are opened through `open_source`".into(),
                ));
            }
        };
        let uri = CString::new(uri).map_err(|e| MediaError::NotFound(e.to_string()))?;
        let playbin = g.make("playbin")?;
        let mut d = GstDecoder {
            g,
            _live: super::Live::new(),
            playbin,
            video: None,
            audio: None,
            text: None,
            channels: 0,
            info: MediaInfo::default(),
            matrix: YuvMatrix::Bt709,
            full_range: false,
            dma: None,
        };
        // DMA-BUFs in the layouts the renderer imports, when one has said it takes them.
        let modifiers = match remedy_ui_paint::gpu_video_adapter() {
            Some(remedy_ui_paint::GpuAdapter::Uuid(_)) => remedy_ui_paint::gpu_video_dma_buf_modifiers(),
            _ => Vec::new(),
        };
        d.dma = (!modifiers.is_empty()).then(super::gst_dmabuf::fns).flatten();
        let vcaps = match d.dma {
            Some(_) => super::gst_dmabuf::caps(&modifiers),
            None => "video/x-raw,format=NV12".to_owned(),
        };
        let vsink = g.app_sink(&vcaps, SINK_PICTURES)?;
        if let Some(f) = d.dma {
            super::gst_dmabuf::accept_video_meta(f, vsink);
            // Without playsink's software colour balance, which works only in CPU memory and
            // would pull DMA-BUF pictures back there (video, audio, text, soft volume,
            // deinterlace: playbin's defaults otherwise).
            // SAFETY: a documented playbin property with its C type (GstPlayFlags).
            unsafe {
                (g.object_set)(playbin, c"flags".as_ptr(), 0x217 as c_uint, std::ptr::null::<c_char>())
            };
        }
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
            if matches!(source, MediaSource::Url(_)) {
                // Progressive download: a stream's bytes are kept (in a temporary file) as
                // they arrive, so the buffering query says which parts of the whole are in
                // hand, not just how full a queue is.
                let flags = g.get_int(playbin, c"flags") | GST_PLAY_FLAG_DOWNLOAD;
                (g.object_set)(playbin, c"flags".as_ptr(), flags, std::ptr::null::<c_char>());
            }
        }
        if let Some(bin) = filter {
            // SAFETY: a documented playbin property; playbin takes the floating bin.
            unsafe { (g.object_set)(playbin, c"video-filter".as_ptr(), bin, std::ptr::null::<c_char>()) };
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
        if let (Some(f), Some(s)) = (d.dma, d.video.as_ref().and_then(|b| b.head))
            && !super::gst_dmabuf::is_dmabuf(g, f, s)
        {
            copy_path("the decoder hands over its pictures in CPU memory, not DMA-BUFs");
        }
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
            // A picture in DMA-BUFs goes over as it is, never mapped. One that cannot (its
            // planes are not all in DMA-BUFs, say) is copied, which a linear one allows.
            if which == 0
                && let Some(f) = self.dma
                && super::gst_dmabuf::is_dmabuf(g, f, s)
            {
                match self.shared_picture(f, s) {
                    Ok(picture) => {
                        let duration = (duration != CLOCK_TIME_NONE).then(|| Duration::from_nanos(duration));
                        (g.mini_object_unref)(s);
                        return Ok(Some(Sample::Video { at, duration, picture }));
                    }
                    Err((why, linear)) => {
                        copy_path(&format!("a picture in DMA-BUFs cannot be shared: {why}"));
                        if !linear {
                            (g.mini_object_unref)(s);
                            return Err(MediaError::Decode(format!(
                                "a tiled DMA-BUF picture cannot be read: {why}"
                            )));
                        }
                    }
                }
            }
            if which == 0 {
                super::gst_dmabuf::MAPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
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

    /// A picture in DMA-BUFs, held as it is; `Err` says why it cannot be, and whether its
    /// layout is linear (so it can be mapped and copied instead).
    fn shared_picture(&self, f: &super::gst_dmabuf::Fns, s: Ptr) -> Result<Picture, (String, bool)> {
        let g = self.g;
        let (mut w, mut h) = (0, 0);
        // SAFETY: reading the sample's caps fields into locals.
        unsafe {
            let st = (g.caps_get_structure)((g.sample_get_caps)(s), 0);
            (g.structure_get_int)(st, c"width".as_ptr(), &mut w);
            (g.structure_get_int)(st, c"height".as_ptr(), &mut h);
        }
        let (w, h) = (u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0));
        if w == 0 || h == 0 {
            return Err(("it has no size".into(), false));
        }
        let p = super::gst_dmabuf::DmaPicture::new(g, f, s, w, h)?;
        Ok(Picture::on_gpu(w, h, self.matrix, self.full_range, Arc::new(p)))
    }

    /// An NV12 picture as its `GstVideoMeta` lays it out (a sink that asked for DMA-BUFs also
    /// takes padded rows in CPU memory), else in GStreamer's default layout: rows padded to 4
    /// bytes, the chroma plane after the luma's (even) rows.
    fn picture(&self, s: Ptr, bytes: &[u8]) -> Result<Picture, MediaError> {
        let g = self.g;
        let (mut w, mut h) = (0, 0);
        // SAFETY: reading the sample's caps fields and buffer into locals.
        let meta = unsafe {
            let st = (g.caps_get_structure)((g.sample_get_caps)(s), 0);
            (g.structure_get_int)(st, c"width".as_ptr(), &mut w);
            (g.structure_get_int)(st, c"height".as_ptr(), &mut h);
            let buf = (g.sample_get_buffer)(s);
            self.dma.and_then(|f| super::gst_dmabuf::video_layout(f, buf))
        };
        let (w, h) = (usize::try_from(w).unwrap_or(0), usize::try_from(h).unwrap_or(0));
        let [(y, ys), (uv, uvs)] = meta.unwrap_or_else(|| {
            let stride = w.div_ceil(4) * 4;
            [(0, stride), (stride * h.div_ceil(2) * 2, stride)]
        });
        let need = (y + ys * h).max(uv + uvs * h.div_ceil(2));
        if w == 0 || h == 0 || ys < w || uvs < w.div_ceil(2) * 2 || bytes.len() < need {
            return Err(MediaError::Decode(format!(
                "an NV12 picture of {w}x{h} came in {} bytes",
                bytes.len()
            )));
        }
        Ok(Picture {
            width: w as u32,
            height: h as u32,
            format: PixelFormat::Nv12,
            planes: [Plane { offset: y, stride: ys }, Plane { offset: uv, stride: uvs }, Plane::default()],
            matrix: self.matrix,
            full_range: self.full_range,
            data: Arc::from(bytes.get(..need).unwrap_or_default()),
            gpu: None,
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

    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        // Asked in time first (a pipeline that knows says so directly); a download buffer
        // (queue2) answers only in percent of the whole, which the length turns into time.
        let time = self.buffering(GST_FORMAT_TIME);
        if time.as_ref().is_some_and(|r| !r.is_empty()) {
            return time;
        }
        match self.buffering(GST_FORMAT_PERCENT) {
            Some(r) if !r.is_empty() => Some(r),
            _ => time,
        }
    }
}

impl GstDecoder {
    /// The pipeline's answer to a buffering query asked in `format`, as positions in the
    /// media: `None` when it does not answer or answers in a format with no time here.
    fn buffering(&self, format: c_int) -> Option<Vec<std::ops::Range<Duration>>> {
        let g = self.g;
        let total = self.info.duration;
        // SAFETY: a buffering query made, asked of the live pipeline, read through its
        // documented accessors with out-pointers to locals, and released.
        unsafe {
            let q = (g.query_new_buffering)(format);
            if q.is_null() {
                return None;
            }
            let mut out = None;
            if (g.element_query)(self.playbin, q) != 0 {
                let (mut format, mut start, mut stop) = (0 as c_int, -1i64, -1i64);
                (g.query_parse_buffering_range)(q, &mut format, &mut start, &mut stop, std::ptr::null_mut());
                let at = |x: i64| -> Option<Duration> {
                    match (format, total) {
                        _ if x < 0 => None,
                        (GST_FORMAT_TIME, _) => Some(Duration::from_nanos(x as u64)),
                        (GST_FORMAT_PERCENT, Some(t)) => {
                            Some(t.mul_f64((x as f64 / GST_FORMAT_PERCENT_MAX).clamp(0.0, 1.0)))
                        }
                        _ => None,
                    }
                };
                let known = format == GST_FORMAT_TIME || (format == GST_FORMAT_PERCENT && total.is_some());
                if known {
                    // A pipeline reports a handful; a bound keeps a strange one from costing.
                    let n = (g.query_get_n_buffering_ranges)(q).min(256);
                    let mut ranges = Vec::new();
                    for i in 0..n {
                        let (mut a, mut b) = (0i64, 0i64);
                        if (g.query_parse_nth_buffering_range)(q, i, &mut a, &mut b) != 0
                            && let (Some(a), Some(b)) = (at(a), at(b))
                        {
                            ranges.push(a..b);
                        }
                    }
                    // No list, but the stretch being downloaded: that one.
                    if ranges.is_empty()
                        && let (Some(a), Some(b)) = (at(start), at(stop))
                    {
                        ranges.push(a..b);
                    }
                    out = Some(ranges);
                }
            }
            (g.mini_object_unref)(q);
            out
        }
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
    _live: super::Live,
    pipeline: Ptr,
    src: Ptr,
    format: AudioFormat,
    /// How much sound waits in the pipeline's queue when it is full.
    depth: Duration,
    queue_bytes: u64,
    written: u64,
    playing: bool,
}

// SAFETY: as for the decoder.
unsafe impl Send for GstOutput {}

impl GstOutput {
    fn open(
        g: &'static Gst,
        format: AudioFormat,
        latency: crate::host::Latency,
    ) -> Result<GstOutput, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        // Interactive: little waiting in the pipeline's own queue (the sound server's
        // buffer after it is the server's to size).
        let depth = match latency {
            crate::host::Latency::Interactive => Duration::from_millis(20),
            _ => OUTPUT_QUEUE,
        };
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
            return Err(launch_error(g.take_error(err), "playing sound", MediaError::Device));
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
        let queue_bytes = format.frames_in(depth) * u64::from(format.channels) * 4;
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
        Ok(GstOutput {
            g,
            _live: super::Live::new(),
            pipeline,
            src,
            format,
            depth,
            queue_bytes,
            written: 0,
            playing: false,
        })
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
        self.depth
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
    _live: super::Live,
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
            return Err(launch_error(g.take_error(err), "recording from the microphone", MediaError::Device));
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
        Ok(GstInput { g, _live: super::Live::new(), pipeline, sink, format })
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

/// Stream decoders tried for H.264, graphics card first: VA-API (Intel, AMD), NVIDIA's,
/// V4L2 stateless and stateful (ARM boards), then the processor's (FFmpeg's, Cisco's).
const H264_HARDWARE: &[&str] = &["vah264dec", "vaapih264dec", "nvh264dec", "v4l2slh264dec", "v4l2h264dec"];
const H264_SOFTWARE: &[&str] = &["avdec_h264", "openh264dec"];

/// Pictures the stream decoder's sink holds: the newest matter, older ones are dropped.
const STREAM_PICTURES: c_uint = 2;

/// What is known in this process about each stream decoder: `Ok` once it decoded a picture
/// here, `Err` with why once it could not (not installed, its device would not open, it failed
/// on data). A decoder is chosen only after it has decoded a picture on this machine, so one
/// that is installed but cannot run here (NVIDIA's with no usable NVIDIA card, as in WSL or on
/// a laptop with the card off) is never chosen.
static VERDICTS: std::sync::Mutex<Vec<(String, Result<(), MissingDependency>)>> =
    std::sync::Mutex::new(Vec::new());

/// What shared-screen and stream decoding is for, in the missing-dependency messages.
const DECODING: &str = "showing H.264 video and shared screens";

/// How long a decoder may take to give its first picture of the test stream.
const PROBE_WAIT: Duration = Duration::from_secs(3);

/// The first of `candidates` known (or now found) to run here, asking `probe` about each one
/// not judged yet and remembering its answer in `verdicts`; the reasons of those that do not
/// run when none does.
fn choose<'a>(
    candidates: &[&'a str],
    verdicts: &mut Vec<(String, Result<(), MissingDependency>)>,
    mut probe: impl FnMut(&str) -> Result<(), MissingDependency>,
) -> Result<&'a str, Vec<MissingDependency>> {
    let mut why = Vec::new();
    for name in candidates {
        let known = verdicts.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
        let verdict = match known {
            Some(v) => v,
            None => {
                let v = probe(name);
                verdicts.push(((*name).to_owned(), v.clone()));
                v
            }
        };
        match verdict {
            Ok(()) => return Ok(name),
            Err(e) => why.push(e),
        }
    }
    Err(why)
}

/// Marks `name` as not running here (it failed on real data after its test passed).
fn condemn(name: &str, why: &str) {
    let verdict = Err(unusable(name, why));
    if let Ok(mut v) = VERDICTS.lock() {
        match v.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = verdict,
            None => v.push((name.to_owned(), verdict)),
        }
    }
}

/// Decoder `name`, installed, cannot run here because of `why`: for a graphics card's decoder
/// it is the card's driver that is missing or unusable.
fn unusable(name: &str, why: &str) -> MissingDependency {
    let why = format!("{name} is installed but {}", why.trim_end_matches('.'));
    if name.starts_with("nv") {
        crate::deps::nvidia_driver(&why).for_purpose(DECODING)
    } else if name.starts_with("va") {
        crate::deps::va_driver(&why).for_purpose(DECODING)
    } else {
        crate::deps::element(name, DECODING).unusable(why)
    }
}

/// A tiny H.264 stream every decoder can read: one access unit (SPS, PPS and an IDR slice) of
/// a 64 by 64 grey picture, Constrained Baseline, each macroblock stored as raw samples
/// (I_PCM), so it needs no encoder to make. Decoding it proves a decoder runs here.
pub(crate) fn test_access_unit() -> Vec<u8> {
    const MBS_WIDE: u32 = 4;
    const MBS_HIGH: u32 = 4;
    let mut sps = Bits::default();
    sps.u(8, 66); // profile_idc: Baseline
    sps.u(8, 0b1100_0000); // constraint_set0 and set1: Constrained Baseline
    sps.u(8, 30); // level_idc 3.0
    sps.ue(0); // seq_parameter_set_id
    sps.ue(0); // log2_max_frame_num_minus4
    sps.ue(2); // pic_order_cnt_type
    sps.ue(1); // max_num_ref_frames
    sps.u(1, 0); // gaps_in_frame_num_value_allowed_flag
    sps.ue(MBS_WIDE - 1); // pic_width_in_mbs_minus1
    sps.ue(MBS_HIGH - 1); // pic_height_in_map_units_minus1
    sps.u(1, 1); // frame_mbs_only_flag
    sps.u(1, 1); // direct_8x8_inference_flag
    sps.u(1, 0); // frame_cropping_flag
    sps.u(1, 0); // vui_parameters_present_flag
    sps.trailing();
    let mut pps = Bits::default();
    pps.ue(0); // pic_parameter_set_id
    pps.ue(0); // seq_parameter_set_id
    pps.u(1, 0); // entropy_coding_mode_flag: CAVLC
    pps.u(1, 0); // bottom_field_pic_order_in_frame_present_flag
    pps.ue(0); // num_slice_groups_minus1
    pps.ue(0); // num_ref_idx_l0_default_active_minus1
    pps.ue(0); // num_ref_idx_l1_default_active_minus1
    pps.u(1, 0); // weighted_pred_flag
    pps.u(2, 0); // weighted_bipred_idc
    pps.se(0); // pic_init_qp_minus26
    pps.se(0); // pic_init_qs_minus26
    pps.se(0); // chroma_qp_index_offset
    pps.u(1, 1); // deblocking_filter_control_present_flag
    pps.u(1, 0); // constrained_intra_pred_flag
    pps.u(1, 0); // redundant_pic_cnt_present_flag
    pps.trailing();
    let mut slice = Bits::default();
    slice.ue(0); // first_mb_in_slice
    slice.ue(7); // slice_type: I (all slices of the picture)
    slice.ue(0); // pic_parameter_set_id
    slice.u(4, 0); // frame_num
    slice.ue(0); // idr_pic_id
    slice.u(1, 0); // no_output_of_prior_pics_flag
    slice.u(1, 0); // long_term_reference_flag
    slice.se(0); // slice_qp_delta
    slice.ue(1); // disable_deblocking_filter_idc: off
    for _ in 0..MBS_WIDE * MBS_HIGH {
        slice.ue(25); // mb_type: I_PCM
        slice.align_zero(); // pcm_alignment_zero_bit
        for _ in 0..256 + 2 * 64 {
            slice.u(8, 0x80); // grey luma and neutral chroma samples
        }
    }
    slice.trailing();
    let mut out = Vec::new();
    for (header, rbsp) in [(0x67u8, sps.bytes()), (0x68, pps.bytes()), (0x65, slice.bytes())] {
        out.extend_from_slice(&[0, 0, 0, 1, header]);
        // Emulation prevention: no 00 00 followed by 00 to 03 inside a NAL unit.
        let mut zeros = 0;
        for b in rbsp {
            if zeros >= 2 && b <= 3 {
                out.push(3);
                zeros = 0;
            }
            out.push(b);
            zeros = if b == 0 { zeros + 1 } else { 0 };
        }
    }
    out
}

/// Bits written most significant first, as H.264 syntax is.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    used: u8,
}

impl Bits {
    fn bit(&mut self, b: u32) {
        if self.used == 0 {
            self.out.push(0);
        }
        if b != 0
            && let Some(last) = self.out.last_mut()
        {
            *last |= 0x80 >> self.used;
        }
        self.used = (self.used + 1) % 8;
    }

    fn u(&mut self, n: u32, v: u32) {
        for i in (0..n).rev() {
            self.bit((v >> i) & 1);
        }
    }

    /// Exp-Golomb, unsigned.
    fn ue(&mut self, v: u32) {
        let code = v + 1;
        let len = 32 - code.leading_zeros();
        self.u(len - 1, 0);
        self.u(len, code);
    }

    /// Exp-Golomb, signed.
    fn se(&mut self, v: i32) {
        let k = if v > 0 { (2 * v - 1) as u32 } else { (-2 * v) as u32 };
        self.ue(k);
    }

    fn align_zero(&mut self) {
        while self.used != 0 {
            self.bit(0);
        }
    }

    /// rbsp_trailing_bits: a one, then zeros to the byte.
    fn trailing(&mut self) {
        self.bit(1);
        self.align_zero();
    }

    fn bytes(self) -> Vec<u8> {
        self.out
    }
}

/// A video stream decoded as it arrives:
/// `appsrc ! h264parse ! <decoder> ! videoconvert ! video/x-raw,format=NV12 ! appsink`.
pub(crate) struct GstStreamDecoder {
    g: &'static Gst,
    _live: super::Live,
    pipeline: Ptr,
    src: Ptr,
    sink: Ptr,
    name: String,
    hardware: bool,
    pushed: u64,
    /// Being tested by [`GstStreamDecoder::probe`]: its failures are the test's answer, not a
    /// verdict to record here (the test holds the verdicts).
    probing: bool,
}

// SAFETY: as for the decoder: GStreamer elements are thread-safe, used from one thread at a time.
unsafe impl Send for GstStreamDecoder {}

impl GstStreamDecoder {
    /// The decoders to consider, best first.
    fn candidates(hardware: bool) -> Vec<&'static str> {
        if hardware {
            H264_HARDWARE.iter().chain(H264_SOFTWARE).copied().collect()
        } else {
            H264_SOFTWARE.to_vec()
        }
    }

    /// The decoder to use: the best one that has decoded the test stream on this machine.
    pub(crate) fn choice(
        g: &'static Gst,
        codec: VideoCodec,
        hardware: bool,
    ) -> Result<&'static str, MediaError> {
        let VideoCodec::H264 = codec;
        let mut verdicts = VERDICTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        choose(&Self::candidates(hardware), &mut verdicts, |name| Self::probe(g, name))
            .map_err(|why| MediaError::Missing(crate::deps::any_of("an H.264 decoder", DECODING, why)))
    }

    fn open(g: &'static Gst, codec: VideoCodec, hardware: bool) -> Result<GstStreamDecoder, MediaError> {
        // A decoder that passed its test but will not start now is judged again, and the next
        // one is taken.
        loop {
            let name = Self::choice(g, codec, hardware)?;
            match Self::with(g, name, H264_HARDWARE.contains(&name)) {
                Ok(d) => return Ok(d),
                Err(e) => condemn(name, &e.to_string()),
            }
        }
    }

    /// Whether decoder `name` runs on this machine: it is installed, its pipeline starts (its
    /// device or GPU context opens), and it turns the test stream into a picture in time.
    fn probe(g: &'static Gst, name: &str) -> Result<(), MissingDependency> {
        // Not installed (or a part it needs is not) as such; installed but failing as unusable.
        let mut d = Self::with(g, name, H264_HARDWARE.contains(&name)).map_err(|e| match e {
            MediaError::Missing(m) => m.for_purpose(DECODING),
            other => unusable(name, &other.to_string()),
        })?;
        d.probing = true;
        d.push(&test_access_unit()).map_err(|e| unusable(name, &e.to_string()))?;
        // The end of the stream makes a decoder that holds pictures back give them out.
        // SAFETY: signalling the end on a live app source.
        unsafe { (g.app_src_end_of_stream)(d.src) };
        let until = std::time::Instant::now() + PROBE_WAIT;
        while std::time::Instant::now() < until {
            match d.next() {
                Ok(Some(p)) if p.width == 64 && p.height == 64 => return Ok(()),
                Ok(Some(p)) => {
                    return Err(unusable(
                        name,
                        &format!("its test picture came out {}x{}", p.width, p.height),
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(unusable(name, &e.to_string())),
            }
        }
        Err(unusable(name, "gave no picture of a test stream"))
    }

    /// The pipeline with decoder `name`, started.
    fn with(g: &'static Gst, name: &str, hardware: bool) -> Result<GstStreamDecoder, MediaError> {
        // An element that is not installed is not tried in a pipeline (whose error would only
        // say "no element").
        let probe = g.make(name)?;
        // SAFETY: the probe was floating and is ours to drop.
        unsafe {
            (g.object_ref)(probe);
            (g.object_unref)(probe);
            (g.object_unref)(probe);
        }
        let desc = format!(
            "appsrc name=src is-live=true format=time do-timestamp=true block=false \
             caps=video/x-h264,stream-format=byte-stream,alignment=au \
             ! h264parse ! {name} ! videoconvert ! video/x-raw,format=NV12 \
             ! appsink name=sink sync=false drop=true max-buffers={STREAM_PICTURES}"
        );
        let desc = CString::new(desc).map_err(|e| MediaError::Unsupported(e.to_string()))?;
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: building a pipeline from a description; errors come back in `err`.
        let pipeline = unsafe { (g.parse_launch)(desc.as_ptr(), &mut err) };
        if pipeline.is_null() {
            return Err(MediaError::Unsupported(g.take_error(err)));
        }
        if !err.is_null() {
            let msg = g.take_error(err);
            // SAFETY: releasing the half-made pipeline.
            unsafe { (g.object_unref)(pipeline) };
            return Err(MediaError::Unsupported(msg));
        }
        // SAFETY: looking up the named elements (new references).
        let (src, sink) = unsafe {
            ((g.bin_get_by_name)(pipeline, c"src".as_ptr()), (g.bin_get_by_name)(pipeline, c"sink".as_ptr()))
        };
        let release = |src: Ptr, sink: Ptr| {
            // SAFETY: dropping what was taken, once.
            unsafe {
                (g.element_set_state)(pipeline, GST_STATE_NULL);
                if !src.is_null() {
                    (g.object_unref)(src);
                }
                if !sink.is_null() {
                    (g.object_unref)(sink);
                }
                (g.object_unref)(pipeline);
            }
        };
        if src.is_null() || sink.is_null() {
            release(src, sink);
            return Err(MediaError::Unsupported("the stream pipeline has no source or sink".into()));
        }
        // SAFETY: a state change on a live pipeline.
        if unsafe { (g.element_set_state)(pipeline, GST_STATE_PLAYING) } == GST_STATE_CHANGE_FAILURE {
            let reason = g.bus_error(pipeline).map_or_else(|| format!("{name} would not start"), |e| e.1);
            release(src, sink);
            return Err(MediaError::Unsupported(reason));
        }
        Ok(GstStreamDecoder {
            g,
            _live: super::Live::new(),
            pipeline,
            src,
            sink,
            name: name.to_owned(),
            hardware,
            pushed: 0,
            probing: false,
        })
    }
}

impl GstStreamDecoder {
    /// An error posted while the stream flowed: this decoder is not chosen again in this
    /// process, and the error says which decoder failed.
    fn failed(&self, text: String) -> MediaError {
        if !self.probing {
            condemn(&self.name, &text);
        }
        MediaError::Decode(format!("{}: {text}", self.name))
    }
}

impl VideoStreamDecoder for GstStreamDecoder {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_hardware(&self) -> bool {
        self.hardware
    }

    fn push(&mut self, data: &[u8]) -> Result<(), MediaError> {
        if let Some((_, text)) = self.g.bus_error(self.pipeline) {
            return Err(self.failed(text));
        }
        if data.is_empty() {
            return Ok(());
        }
        let g = self.g;
        // SAFETY: a new buffer of the piece's size, mapped for writing while filled, then handed
        // to the source (which takes ownership).
        unsafe {
            let buf = (g.buffer_new_allocate)(std::ptr::null_mut(), data.len(), std::ptr::null_mut());
            if buf.is_null() {
                return Err(MediaError::Decode("no memory for a video buffer".into()));
            }
            let mut map = GstMapInfo::default();
            if (g.buffer_map)(buf, &mut map, GST_MAP_WRITE) == 0
                || map.data.is_null()
                || map.size < data.len()
            {
                (g.mini_object_unref)(buf.cast());
                return Err(MediaError::Decode("a video buffer could not be written".into()));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), map.data, data.len());
            (g.buffer_unmap)(buf, &mut map);
            if (g.app_src_push_buffer)(self.src, buf) != 0 {
                let why = g
                    .bus_error(self.pipeline)
                    .map_or_else(|| "it stopped taking the stream".to_owned(), |e| e.1);
                return Err(self.failed(why));
            }
        }
        self.pushed += 1;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Picture>, MediaError> {
        let g = self.g;
        if let Some((_, text)) = g.bus_error(self.pipeline) {
            return Err(self.failed(text));
        }
        // The newest picture ready: older ones still in the sink are let go.
        let mut newest: Option<Ptr> = None;
        loop {
            // SAFETY: pulling from a live app sink without waiting.
            let s = unsafe { (g.app_sink_try_pull_sample)(self.sink, 0) };
            if s.is_null() {
                break;
            }
            if let Some(old) = newest.replace(s) {
                // SAFETY: a sample we pulled and no longer need.
                unsafe { (g.mini_object_unref)(old) };
            }
        }
        let Some(s) = newest else { return Ok(None) };
        // SAFETY: the sample is ours until unref'd; its buffer is mapped for reading while
        // copied and unmapped after.
        unsafe {
            let buf = (g.sample_get_buffer)(s);
            if buf.is_null() {
                (g.mini_object_unref)(s);
                return Ok(None);
            }
            let mut map = GstMapInfo::default();
            if (g.buffer_map)(buf, &mut map, GST_MAP_READ) == 0 {
                (g.mini_object_unref)(s);
                return Err(MediaError::Decode("a decoded picture could not be read".into()));
            }
            let bytes: &[u8] =
                if map.data.is_null() { &[] } else { std::slice::from_raw_parts(map.data, map.size) };
            let out = nv12_picture(g, s, bytes);
            (g.buffer_unmap)(buf, &mut map);
            (g.mini_object_unref)(s);
            out.map(Some)
        }
    }
}

impl Drop for GstStreamDecoder {
    fn drop(&mut self) {
        // SAFETY: stopping the pipeline and releasing our references, once.
        unsafe {
            (self.g.element_set_state)(self.pipeline, GST_STATE_NULL);
            (self.g.object_unref)(self.src);
            (self.g.object_unref)(self.sink);
            (self.g.object_unref)(self.pipeline);
        }
    }
}

/// An NV12 picture from a sample in GStreamer's default layout (rows padded to 4 bytes, the
/// chroma plane after the luma's even rows), with its colours as its caps say.
pub(crate) fn nv12_picture(g: &Gst, s: Ptr, bytes: &[u8]) -> Result<Picture, MediaError> {
    let (mut w, mut h) = (0, 0);
    // SAFETY: reading the sample's caps fields into locals.
    let colorimetry = unsafe {
        let st = (g.caps_get_structure)((g.sample_get_caps)(s), 0);
        (g.structure_get_int)(st, c"width".as_ptr(), &mut w);
        (g.structure_get_int)(st, c"height".as_ptr(), &mut h);
        let c = (g.structure_get_string)(st, c"colorimetry".as_ptr());
        if c.is_null() { String::new() } else { CStr::from_ptr(c).to_string_lossy().into_owned() }
    };
    let (w, h) = (usize::try_from(w).unwrap_or(0), usize::try_from(h).unwrap_or(0));
    let (matrix, full_range) = colours(&colorimetry, h);
    let stride = w.div_ceil(4) * 4;
    let rows = h.div_ceil(2) * 2;
    let need = stride * rows + stride * rows / 2;
    if w == 0 || h == 0 || bytes.len() < need {
        return Err(MediaError::Decode(format!("an NV12 picture of {w}x{h} came in {} bytes", bytes.len())));
    }
    Ok(Picture {
        width: w as u32,
        height: h as u32,
        format: PixelFormat::Nv12,
        planes: [Plane { offset: 0, stride }, Plane { offset: stride * rows, stride }, Plane::default()],
        matrix,
        full_range,
        data: Arc::from(bytes.get(..need).unwrap_or_default()),
        gpu: None,
    })
}

/// The YUV matrix and range a caps `colorimetry` string names ("bt709", "bt601", or
/// "range:matrix:transfer:primaries" with range 1 = full and matrix 3 = BT.709, 4 = BT.601);
/// without one, standard definition (under 720 rows) is BT.601.
fn colours(c: &str, height: usize) -> (YuvMatrix, bool) {
    let sd = c.starts_with("bt601") || c.split(':').nth(1) == Some("4") || (c.is_empty() && height < 720);
    (if sd { YuvMatrix::Bt601 } else { YuvMatrix::Bt709 }, c.split(':').next() == Some("1") || c == "jpeg")
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn colorimetry_names_the_matrix_and_range() {
        assert_eq!(colours("bt709", 1080), (YuvMatrix::Bt709, false));
        assert_eq!(colours("bt601", 1080), (YuvMatrix::Bt601, false));
        assert_eq!(colours("1:4:5:4", 1080), (YuvMatrix::Bt601, true));
        assert_eq!(colours("", 480), (YuvMatrix::Bt601, false));
        assert_eq!(colours("", 1080), (YuvMatrix::Bt709, false));
    }

    #[test]
    fn a_decoder_is_chosen_only_once_it_has_run_and_each_is_tried_once() {
        let mut verdicts = Vec::new();
        let mut asked = Vec::new();
        // The card's decoder is installed but cannot run here; the processor's can.
        let mut fake = |name: &str| -> Result<(), MissingDependency> {
            asked.push(name.to_owned());
            match name {
                "nvh264dec" => Err(unusable(name, "no usable NVIDIA GPU was found")),
                "vah264dec" => Err(crate::deps::element(name, DECODING)),
                _ => Ok(()),
            }
        };
        let got = choose(&["vah264dec", "nvh264dec", "avdec_h264", "openh264dec"], &mut verdicts, &mut fake);
        assert_eq!(got, Ok("avdec_h264"));
        let again = choose(&["vah264dec", "nvh264dec", "avdec_h264"], &mut verdicts, &mut fake);
        assert_eq!(again, Ok("avdec_h264"), "the same answer");
        assert_eq!(
            asked,
            ["vah264dec", "nvh264dec", "avdec_h264"],
            "each decoder is tested once per process"
        );
        let none = choose(&["vah264dec", "nvh264dec"], &mut verdicts, |_| Ok(()));
        let Err(why) = none else { unreachable!("neither decoder runs") };
        assert_eq!(why.len(), 2, "a reason for each that does not run");
        assert_eq!(
            why.first().map(|w| w.state.clone()),
            Some(crate::deps::DependencyState::NotInstalled),
            "vah264dec is not installed"
        );
        let s = why.get(1).map(ToString::to_string).unwrap_or_default();
        assert!(
            s.contains("NVIDIA") && s.contains("nvh264dec is installed but no usable NVIDIA GPU was found"),
            "{s}"
        );
    }

    #[test]
    fn the_test_stream_is_one_annex_b_access_unit_without_start_code_lookalikes() {
        let au = test_access_unit();
        let starts: Vec<usize> =
            au.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 0, 1]).map(|(i, _)| i).collect();
        assert_eq!(starts.len(), 3, "SPS, PPS and the IDR slice");
        let types: Vec<u8> = starts.iter().filter_map(|i| au.get(i + 4)).map(|b| b & 0x1f).collect();
        assert_eq!(types, [7, 8, 5]);
        // Inside each unit no 00 00 00/01/02 appears (emulation prevention was applied).
        for (k, s) in starts.iter().enumerate() {
            let end = starts.get(k + 1).copied().unwrap_or(au.len());
            let body = au.get(s + 4..end).unwrap_or_default();
            assert!(body.windows(3).all(|w| !matches!(w, [0, 0, 0..=2])));
        }
        // 16 macroblocks of 384 raw samples each, plus headers.
        assert!(au.len() > 16 * 384);
    }

    #[test]
    fn exp_golomb_codes_are_written_as_the_standard_says() {
        let mut b = Bits::default();
        b.ue(0); // 1
        b.ue(1); // 010
        b.ue(25); // 000011010
        b.se(-1); // ue(2) = 011
        b.trailing();
        // 1 010 000011010 011 1 then zeros: 10100000 11010011 10000000
        assert_eq!(b.bytes(), [0b1010_0000, 0b1101_0011, 0b1000_0000]);
    }

    #[test]
    fn a_pipeline_without_an_element_names_what_to_install() {
        let e = launch_error("no element \"autoaudiosink\"".into(), "playing sound", MediaError::Device);
        let s = e.to_string();
        assert!(s.starts_with("Playing sound needs GStreamer's good plugins, for the autoaudiosink element, which is not installed."), "{s}");
        assert!(matches!(
            launch_error("something else".into(), "x", MediaError::Device),
            MediaError::Device(_)
        ));
        assert!(matches!(
            map_error(0, "Your GStreamer installation is missing a plug-in.".into()),
            MediaError::Missing(_)
        ));
    }

    #[test]
    fn hardware_decoders_come_before_the_processors() {
        assert!(H264_SOFTWARE.contains(&"avdec_h264"));
        assert!(H264_HARDWARE.iter().all(|h| !H264_SOFTWARE.contains(h)));
    }
}
