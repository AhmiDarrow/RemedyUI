//! Android: the NDK's `AMediaExtractor` and `AMediaCodec` (the device's own decoders,
//! hardware ones included) decode, and AAudio plays the sound. Both libraries
//! (`libmediandk.so`, API 21, and `libaaudio.so`, API 26) are opened at run time, so an app
//! still starts on a device without them and gets `Unsupported` with the reason.

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use libloading::Library;

use crate::host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture,
    PixelFormat, Plane, Sample, VideoInfo, YuvMatrix,
};

type Ptr = *mut c_void;
type Status = i32;

const AMEDIA_OK: Status = 0;
const BUFFER_FLAG_END_OF_STREAM: u32 = 4;
const INFO_TRY_AGAIN_LATER: isize = -1;
const INFO_OUTPUT_FORMAT_CHANGED: isize = -2;
const SEEK_PREVIOUS_SYNC: i32 = 0;
/// `MediaCodecInfo.CodecCapabilities` color formats a byte-buffer decoder may give.
const COLOR_YUV420_PLANAR: i32 = 19;
const COLOR_YUV420_SEMIPLANAR: i32 = 21;
/// `AudioFormat.ENCODING_PCM_FLOAT` and `ENCODING_PCM_16BIT`.
const PCM_FLOAT: i32 = 4;
const PCM_16: i32 = 2;
const AAUDIO_OK: i32 = 0;
const AAUDIO_DIRECTION_OUTPUT: i32 = 0;
const AAUDIO_DIRECTION_INPUT: i32 = 1;
const AAUDIO_PERFORMANCE_MODE_LOW_LATENCY: i32 = 12;
/// Sound as the microphone hears it, without the voice-call processing that would bend a
/// sung or played note (Android 9, API 28; earlier versions keep their default).
const AAUDIO_INPUT_PRESET_UNPROCESSED: i32 = 9;
const AAUDIO_FORMAT_PCM_FLOAT: i32 = 2;
const CLOCK_MONOTONIC: i32 = 1;

/// How long one wait for a codec buffer lasts before the decoder looks at its other work.
const CODEC_WAIT_US: i64 = 10_000;

#[repr(C)]
#[derive(Default)]
struct BufferInfo {
    offset: i32,
    size: i32,
    presentation_time_us: i64,
    flags: u32,
}

struct Ndk {
    extractor_new: unsafe extern "C" fn() -> Ptr,
    extractor_delete: unsafe extern "C" fn(Ptr) -> Status,
    extractor_set_data_source: unsafe extern "C" fn(Ptr, *const c_char) -> Status,
    extractor_get_track_count: unsafe extern "C" fn(Ptr) -> usize,
    extractor_get_track_format: unsafe extern "C" fn(Ptr, usize) -> Ptr,
    extractor_select_track: unsafe extern "C" fn(Ptr, usize) -> Status,
    extractor_read_sample_data: unsafe extern "C" fn(Ptr, *mut u8, usize) -> isize,
    extractor_get_sample_track_index: unsafe extern "C" fn(Ptr) -> i32,
    extractor_get_sample_time: unsafe extern "C" fn(Ptr) -> i64,
    extractor_advance: unsafe extern "C" fn(Ptr) -> bool,
    extractor_seek_to: unsafe extern "C" fn(Ptr, i64, i32) -> Status,
    format_delete: unsafe extern "C" fn(Ptr) -> Status,
    format_get_int32: unsafe extern "C" fn(Ptr, *const c_char, *mut i32) -> bool,
    format_get_int64: unsafe extern "C" fn(Ptr, *const c_char, *mut i64) -> bool,
    format_get_string: unsafe extern "C" fn(Ptr, *const c_char, *mut *const c_char) -> bool,
    format_set_int32: unsafe extern "C" fn(Ptr, *const c_char, i32),
    codec_create_decoder_by_type: unsafe extern "C" fn(*const c_char) -> Ptr,
    codec_configure: unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr, u32) -> Status,
    codec_start: unsafe extern "C" fn(Ptr) -> Status,
    codec_stop: unsafe extern "C" fn(Ptr) -> Status,
    codec_flush: unsafe extern "C" fn(Ptr) -> Status,
    codec_delete: unsafe extern "C" fn(Ptr) -> Status,
    codec_dequeue_input_buffer: unsafe extern "C" fn(Ptr, i64) -> isize,
    codec_get_input_buffer: unsafe extern "C" fn(Ptr, usize, *mut usize) -> *mut u8,
    codec_queue_input_buffer: unsafe extern "C" fn(Ptr, usize, i64, usize, u64, u32) -> Status,
    codec_dequeue_output_buffer: unsafe extern "C" fn(Ptr, *mut BufferInfo, i64) -> isize,
    codec_get_output_buffer: unsafe extern "C" fn(Ptr, usize, *mut usize) -> *mut u8,
    codec_get_output_format: unsafe extern "C" fn(Ptr) -> Ptr,
    codec_release_output_buffer: unsafe extern "C" fn(Ptr, usize, bool) -> Status,
    _lib: Library,
}

struct AAudio {
    create_stream_builder: unsafe extern "C" fn(*mut Ptr) -> i32,
    builder_set_direction: unsafe extern "C" fn(Ptr, i32),
    builder_set_format: unsafe extern "C" fn(Ptr, i32),
    builder_set_channel_count: unsafe extern "C" fn(Ptr, i32),
    builder_set_sample_rate: unsafe extern "C" fn(Ptr, i32),
    builder_open_stream: unsafe extern "C" fn(Ptr, *mut Ptr) -> i32,
    builder_delete: unsafe extern "C" fn(Ptr) -> i32,
    stream_request_start: unsafe extern "C" fn(Ptr) -> i32,
    stream_request_pause: unsafe extern "C" fn(Ptr) -> i32,
    stream_request_flush: unsafe extern "C" fn(Ptr) -> i32,
    stream_close: unsafe extern "C" fn(Ptr) -> i32,
    stream_write: unsafe extern "C" fn(Ptr, *const c_void, i32, i64) -> i32,
    stream_read: unsafe extern "C" fn(Ptr, *mut c_void, i32, i64) -> i32,
    stream_get_frames_per_burst: unsafe extern "C" fn(Ptr) -> i32,
    stream_get_sample_rate: unsafe extern "C" fn(Ptr) -> i32,
    stream_get_channel_count: unsafe extern "C" fn(Ptr) -> i32,
    builder_set_performance_mode: unsafe extern "C" fn(Ptr, i32),
    /// Android 9 and later.
    builder_set_input_preset: Option<unsafe extern "C" fn(Ptr, i32)>,
    stream_get_buffer_size_in_frames: unsafe extern "C" fn(Ptr) -> i32,
    stream_get_frames_written: unsafe extern "C" fn(Ptr) -> i64,
    stream_get_frames_read: unsafe extern "C" fn(Ptr) -> i64,
    stream_get_timestamp: unsafe extern "C" fn(Ptr, i32, *mut i64, *mut i64) -> i32,
    convert_result_to_text: unsafe extern "C" fn(i32) -> *const c_char,
    _lib: Library,
}

// SAFETY: plain C function pointers into libraries kept loaded for the process's life; the
// NDK media and AAudio calls made here may come from any thread.
unsafe impl Send for Ndk {}
// SAFETY: as above.
unsafe impl Sync for Ndk {}
// SAFETY: as above.
unsafe impl Send for AAudio {}
// SAFETY: as above.
unsafe impl Sync for AAudio {}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        // SAFETY: the symbol's type is the NDK's documented C signature; the library stays
        // loaded while the table lives.
        *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("{}: {e}", $name))?
    };
}

fn load(name: &str) -> Result<Library, String> {
    // SAFETY: loading a system library of the platform.
    unsafe { Library::new(name) }.map_err(|e| e.to_string())
}

impl Ndk {
    fn load() -> Result<Ndk, String> {
        let l = load("libmediandk.so")?;
        Ok(Ndk {
            extractor_new: sym!(l, "AMediaExtractor_new"),
            extractor_delete: sym!(l, "AMediaExtractor_delete"),
            extractor_set_data_source: sym!(l, "AMediaExtractor_setDataSource"),
            extractor_get_track_count: sym!(l, "AMediaExtractor_getTrackCount"),
            extractor_get_track_format: sym!(l, "AMediaExtractor_getTrackFormat"),
            extractor_select_track: sym!(l, "AMediaExtractor_selectTrack"),
            extractor_read_sample_data: sym!(l, "AMediaExtractor_readSampleData"),
            extractor_get_sample_track_index: sym!(l, "AMediaExtractor_getSampleTrackIndex"),
            extractor_get_sample_time: sym!(l, "AMediaExtractor_getSampleTime"),
            extractor_advance: sym!(l, "AMediaExtractor_advance"),
            extractor_seek_to: sym!(l, "AMediaExtractor_seekTo"),
            format_delete: sym!(l, "AMediaFormat_delete"),
            format_get_int32: sym!(l, "AMediaFormat_getInt32"),
            format_get_int64: sym!(l, "AMediaFormat_getInt64"),
            format_get_string: sym!(l, "AMediaFormat_getString"),
            format_set_int32: sym!(l, "AMediaFormat_setInt32"),
            codec_create_decoder_by_type: sym!(l, "AMediaCodec_createDecoderByType"),
            codec_configure: sym!(l, "AMediaCodec_configure"),
            codec_start: sym!(l, "AMediaCodec_start"),
            codec_stop: sym!(l, "AMediaCodec_stop"),
            codec_flush: sym!(l, "AMediaCodec_flush"),
            codec_delete: sym!(l, "AMediaCodec_delete"),
            codec_dequeue_input_buffer: sym!(l, "AMediaCodec_dequeueInputBuffer"),
            codec_get_input_buffer: sym!(l, "AMediaCodec_getInputBuffer"),
            codec_queue_input_buffer: sym!(l, "AMediaCodec_queueInputBuffer"),
            codec_dequeue_output_buffer: sym!(l, "AMediaCodec_dequeueOutputBuffer"),
            codec_get_output_buffer: sym!(l, "AMediaCodec_getOutputBuffer"),
            codec_get_output_format: sym!(l, "AMediaCodec_getOutputFormat"),
            codec_release_output_buffer: sym!(l, "AMediaCodec_releaseOutputBuffer"),
            _lib: l,
        })
    }

    fn int(&self, format: Ptr, key: &CStr) -> Option<i32> {
        let mut v = 0;
        // SAFETY: reading a key of a live format into a local.
        unsafe { (self.format_get_int32)(format, key.as_ptr(), &mut v) }.then_some(v)
    }

    fn long(&self, format: Ptr, key: &CStr) -> Option<i64> {
        let mut v = 0;
        // SAFETY: as above.
        unsafe { (self.format_get_int64)(format, key.as_ptr(), &mut v) }.then_some(v)
    }

    fn string(&self, format: Ptr, key: &CStr) -> Option<String> {
        let mut p: *const c_char = std::ptr::null();
        // SAFETY: the string belongs to the format and is copied before the format goes.
        unsafe {
            if (self.format_get_string)(format, key.as_ptr(), &mut p) && !p.is_null() {
                Some(CStr::from_ptr(p).to_string_lossy().into_owned())
            } else {
                None
            }
        }
    }
}

impl AAudio {
    fn load() -> Result<AAudio, String> {
        let l = load("libaaudio.so")?;
        Ok(AAudio {
            create_stream_builder: sym!(l, "AAudio_createStreamBuilder"),
            builder_set_direction: sym!(l, "AAudioStreamBuilder_setDirection"),
            builder_set_format: sym!(l, "AAudioStreamBuilder_setFormat"),
            builder_set_channel_count: sym!(l, "AAudioStreamBuilder_setChannelCount"),
            builder_set_sample_rate: sym!(l, "AAudioStreamBuilder_setSampleRate"),
            builder_open_stream: sym!(l, "AAudioStreamBuilder_openStream"),
            builder_delete: sym!(l, "AAudioStreamBuilder_delete"),
            stream_request_start: sym!(l, "AAudioStream_requestStart"),
            stream_request_pause: sym!(l, "AAudioStream_requestPause"),
            stream_request_flush: sym!(l, "AAudioStream_requestFlush"),
            stream_close: sym!(l, "AAudioStream_close"),
            stream_write: sym!(l, "AAudioStream_write"),
            stream_read: sym!(l, "AAudioStream_read"),
            stream_get_frames_per_burst: sym!(l, "AAudioStream_getFramesPerBurst"),
            stream_get_sample_rate: sym!(l, "AAudioStream_getSampleRate"),
            stream_get_channel_count: sym!(l, "AAudioStream_getChannelCount"),
            builder_set_performance_mode: sym!(l, "AAudioStreamBuilder_setPerformanceMode"),
            // SAFETY: as for `sym!`; absent before Android 9.
            builder_set_input_preset: unsafe { l.get(b"AAudioStreamBuilder_setInputPreset\0") }
                .ok()
                .map(|f| *f),
            stream_get_buffer_size_in_frames: sym!(l, "AAudioStream_getBufferSizeInFrames"),
            stream_get_frames_written: sym!(l, "AAudioStream_getFramesWritten"),
            stream_get_frames_read: sym!(l, "AAudioStream_getFramesRead"),
            stream_get_timestamp: sym!(l, "AAudioStream_getTimestamp"),
            convert_result_to_text: sym!(l, "AAudio_convertResultToText"),
            _lib: l,
        })
    }

    fn error(&self, what: &str, r: i32) -> MediaError {
        // SAFETY: AAudio returns a static string for any result code.
        let text = unsafe {
            let p = (self.convert_result_to_text)(r);
            if p.is_null() { r.to_string() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        MediaError::Device(format!("{what}: {text}"))
    }
}

fn ndk() -> Result<&'static Ndk, MediaError> {
    static NDK: OnceLock<Result<Ndk, String>> = OnceLock::new();
    NDK.get_or_init(Ndk::load)
        .as_ref()
        .map_err(|e| MediaError::Unsupported(format!("the NDK media library is not available: {e}")))
}

fn aaudio() -> Result<&'static AAudio, MediaError> {
    static AA: OnceLock<Result<AAudio, String>> = OnceLock::new();
    AA.get_or_init(AAudio::load)
        .as_ref()
        .map_err(|e| MediaError::Unsupported(format!("AAudio is not available (Android 8 or later): {e}")))
}

struct AndroidHost;

pub(super) fn host() -> Result<Arc<dyn MediaHost>, MediaError> {
    ndk()?;
    Ok(Arc::new(AndroidHost))
}

impl MediaHost for AndroidHost {
    fn name(&self) -> &str {
        "MediaCodec"
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Ok(Box::new(NdkDecoder::open(ndk()?, source)?))
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(AAudioIn::open(aaudio()?, format)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(AAudioOut::open(aaudio()?, format)?))
    }
}

/// One track's decoder.
struct Track {
    index: usize,
    codec: Ptr,
    /// The end of the stream was queued as input.
    input_done: bool,
    /// The decoder gave its last output.
    output_done: bool,
}

/// How a decoded picture sits in a codec's output buffer.
#[derive(Clone, Copy)]
struct Layout {
    format: i32,
    stride: usize,
    slice_height: usize,
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

struct NdkDecoder {
    ndk: &'static Ndk,
    extractor: Ptr,
    video: Option<Track>,
    audio: Option<Track>,
    layout: Option<Layout>,
    pcm_float: bool,
    channels: usize,
    extractor_done: bool,
    pending: VecDeque<Sample>,
    info: MediaInfo,
}

// SAFETY: the extractor and codecs are used from one thread at a time.
unsafe impl Send for NdkDecoder {}

fn picture_layout(ndk: &Ndk, f: Ptr) -> Result<Layout, MediaError> {
    let width = ndk.int(f, c"width").unwrap_or(0).max(0) as usize;
    let height = ndk.int(f, c"height").unwrap_or(0).max(0) as usize;
    let format = ndk.int(f, c"color-format").unwrap_or(COLOR_YUV420_SEMIPLANAR);
    if format != COLOR_YUV420_PLANAR && format != COLOR_YUV420_SEMIPLANAR {
        return Err(MediaError::Unsupported(format!(
            "this device's decoder gives pictures in color format {format:#x}, which only its Image API describes"
        )));
    }
    let stride = ndk.int(f, c"stride").filter(|s| *s > 0).map_or(width, |s| s as usize);
    let slice_height = ndk.int(f, c"slice-height").filter(|s| *s > 0).map_or(height, |s| s as usize);
    let (left, top) = (
        ndk.int(f, c"crop-left").unwrap_or(0).max(0) as usize,
        ndk.int(f, c"crop-top").unwrap_or(0).max(0) as usize,
    );
    let right = ndk.int(f, c"crop-right").map(|r| r.max(0) as usize + 1).unwrap_or(width);
    let bottom = ndk.int(f, c"crop-bottom").map(|b| b.max(0) as usize + 1).unwrap_or(height);
    let (w, h) = (right.saturating_sub(left), bottom.saturating_sub(top));
    if w == 0 || h == 0 {
        return Err(MediaError::Format("the decoder gave pictures with no size".into()));
    }
    Ok(Layout { format, stride, slice_height, left: left & !1, top: top & !1, width: w, height: h })
}

impl NdkDecoder {
    fn open(ndk: &'static Ndk, source: &MediaSource) -> Result<NdkDecoder, MediaError> {
        let location = match source {
            MediaSource::File(p) => {
                if !p.exists() {
                    return Err(MediaError::NotFound(p.display().to_string()));
                }
                p.to_string_lossy().into_owned()
            }
            MediaSource::Url(u) => u.clone(),
        };
        let location = CString::new(location).map_err(|e| MediaError::NotFound(e.to_string()))?;
        // SAFETY: NDK calls on objects made here, released in `Drop` (or below on failure).
        unsafe {
            let extractor = (ndk.extractor_new)();
            if extractor.is_null() {
                return Err(MediaError::Unsupported("no media extractor".into()));
            }
            let mut d = NdkDecoder {
                ndk,
                extractor,
                video: None,
                audio: None,
                layout: None,
                pcm_float: false,
                channels: 0,
                extractor_done: false,
                pending: VecDeque::new(),
                info: MediaInfo::default(),
            };
            if (ndk.extractor_set_data_source)(extractor, location.as_ptr()) != AMEDIA_OK {
                return Err(MediaError::NotFound(format!(
                    "{} could not be read",
                    location.to_string_lossy()
                )));
            }
            let mut duration = None;
            for i in 0..(ndk.extractor_get_track_count)(extractor) {
                let f = (ndk.extractor_get_track_format)(extractor, i);
                if f.is_null() {
                    continue;
                }
                let mime = ndk.string(f, c"mime").unwrap_or_default();
                if let Some(us) = ndk.long(f, c"durationUs").filter(|d| *d > 0) {
                    duration = Some(duration.map_or(us, |d: i64| d.max(us)));
                }
                if mime.starts_with("video/") && d.video.is_none() {
                    if let Some(codec) = d.decoder(&mime, f) {
                        let fps = ndk.int(f, c"frame-rate").filter(|r| *r > 0).map(f64::from);
                        let (w, h) = (ndk.int(f, c"width").unwrap_or(0), ndk.int(f, c"height").unwrap_or(0));
                        d.info.video = Some(VideoInfo {
                            width: w.max(0) as u32,
                            height: h.max(0) as u32,
                            frame_rate: fps,
                        });
                        d.video = Some(Track { index: i, codec, input_done: false, output_done: false });
                        (ndk.extractor_select_track)(extractor, i);
                    }
                } else if mime.starts_with("audio/") && d.audio.is_none() {
                    // Ask for float samples (honored from Android 9; 16-bit ones are converted).
                    (ndk.format_set_int32)(f, c"pcm-encoding".as_ptr(), PCM_FLOAT);
                    if let Some(codec) = d.decoder(&mime, f) {
                        let rate = ndk.int(f, c"sample-rate").unwrap_or(0);
                        let ch = ndk.int(f, c"channel-count").unwrap_or(0);
                        if rate > 0 && ch > 0 {
                            d.channels = ch as usize;
                            d.info.audio =
                                Some(AudioFormat { sample_rate: rate as u32, channels: ch as u16 });
                            d.audio = Some(Track { index: i, codec, input_done: false, output_done: false });
                            (ndk.extractor_select_track)(extractor, i);
                        } else {
                            (ndk.codec_delete)(codec);
                        }
                    }
                }
                (ndk.format_delete)(f);
            }
            if d.video.is_none() && d.audio.is_none() {
                return Err(MediaError::Format(
                    "no sound or video stream this device can decode (a codec may be missing)".into(),
                ));
            }
            d.info.duration = duration.map(|us| Duration::from_micros(us as u64));
            d.info.seekable = d.info.duration.is_some();
            Ok(d)
        }
    }

    /// A started decoder for `mime` configured from the track's format, or `None`.
    fn decoder(&self, mime: &str, format: Ptr) -> Option<Ptr> {
        let ndk = self.ndk;
        let m = CString::new(mime).ok()?;
        // SAFETY: creating, configuring and starting a decoder; it is deleted on failure.
        unsafe {
            let codec = (ndk.codec_create_decoder_by_type)(m.as_ptr());
            if codec.is_null() {
                return None;
            }
            let null = std::ptr::null_mut();
            if (ndk.codec_configure)(codec, format, null, null, 0) != AMEDIA_OK
                || (ndk.codec_start)(codec) != AMEDIA_OK
            {
                (ndk.codec_delete)(codec);
                return None;
            }
            Some(codec)
        }
    }

    /// Feeds the extractor's next sample to its track's decoder, if one has room.
    fn feed(&mut self) -> Result<(), MediaError> {
        let ndk = self.ndk;
        if self.extractor_done {
            // Every decoder hears the end once.
            for t in [self.video.as_mut(), self.audio.as_mut()].into_iter().flatten() {
                if t.input_done {
                    continue;
                }
                // SAFETY: queueing an empty end-of-stream input on a live decoder.
                unsafe {
                    let i = (ndk.codec_dequeue_input_buffer)(t.codec, CODEC_WAIT_US);
                    if i >= 0 {
                        (ndk.codec_queue_input_buffer)(
                            t.codec,
                            i as usize,
                            0,
                            0,
                            0,
                            BUFFER_FLAG_END_OF_STREAM,
                        );
                        t.input_done = true;
                    }
                }
            }
            return Ok(());
        }
        // SAFETY: extractor and decoder calls on live objects; the sample is read straight
        // into the decoder's input buffer, no larger than it.
        unsafe {
            let track = (ndk.extractor_get_sample_track_index)(self.extractor);
            if track < 0 {
                self.extractor_done = true;
                return Ok(());
            }
            let t = [self.video.as_mut(), self.audio.as_mut()]
                .into_iter()
                .flatten()
                .find(|t| t.index == track as usize);
            let Some(t) = t else {
                self.extractor_done = !(ndk.extractor_advance)(self.extractor);
                return Ok(());
            };
            let i = (ndk.codec_dequeue_input_buffer)(t.codec, CODEC_WAIT_US);
            if i < 0 {
                return Ok(());
            }
            let mut cap = 0usize;
            let buf = (ndk.codec_get_input_buffer)(t.codec, i as usize, &mut cap);
            if buf.is_null() {
                return Err(MediaError::Decode("the decoder gave no input buffer".into()));
            }
            let n = (ndk.extractor_read_sample_data)(self.extractor, buf, cap);
            let time = (ndk.extractor_get_sample_time)(self.extractor).max(0) as u64;
            let size = usize::try_from(n).unwrap_or(0);
            (ndk.codec_queue_input_buffer)(t.codec, i as usize, 0, size, time, 0);
            self.extractor_done = !(ndk.extractor_advance)(self.extractor);
        }
        Ok(())
    }

    /// Takes what the decoders have finished into `pending`.
    fn drain(&mut self, wait_us: i64) -> Result<(), MediaError> {
        let ndk = self.ndk;
        for video in [true, false] {
            let Some(codec) = (if video { self.video.as_ref() } else { self.audio.as_ref() })
                .filter(|t| !t.output_done)
                .map(|t| t.codec)
            else {
                continue;
            };
            let mut info = BufferInfo::default();
            // SAFETY: dequeuing, reading and releasing an output buffer of a live decoder.
            unsafe {
                let i = (ndk.codec_dequeue_output_buffer)(codec, &mut info, wait_us);
                if i == INFO_TRY_AGAIN_LATER {
                    continue;
                }
                if i == INFO_OUTPUT_FORMAT_CHANGED {
                    let f = (ndk.codec_get_output_format)(codec);
                    if !f.is_null() {
                        if video {
                            self.layout = Some(picture_layout(ndk, f)?);
                        } else {
                            self.pcm_float = ndk.int(f, c"pcm-encoding").unwrap_or(PCM_16) == PCM_FLOAT;
                            if let Some(ch) = ndk.int(f, c"channel-count").filter(|c| *c > 0) {
                                self.channels = ch as usize;
                            }
                        }
                        (ndk.format_delete)(f);
                    }
                    continue;
                }
                if i < 0 {
                    continue;
                }
                let idx = i as usize;
                let mut size = 0usize;
                let base = (ndk.codec_get_output_buffer)(codec, idx, &mut size);
                let start = usize::try_from(info.offset).unwrap_or(0);
                let len = usize::try_from(info.size).unwrap_or(0);
                let at = Duration::from_micros(info.presentation_time_us.max(0) as u64);
                if !base.is_null() && len > 0 && start + len <= size {
                    let bytes = std::slice::from_raw_parts(base.add(start), len);
                    if video {
                        let layout = match self.layout {
                            Some(l) => l,
                            None => {
                                let f = (ndk.codec_get_output_format)(codec);
                                let l = picture_layout(ndk, f);
                                (ndk.format_delete)(f);
                                l?
                            }
                        };
                        self.layout = Some(layout);
                        let picture = picture(bytes, layout)?;
                        self.pending.push_back(Sample::Video { at, duration: None, picture });
                    } else {
                        let samples: Vec<f32> = if self.pcm_float {
                            bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect()
                        } else {
                            bytes
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|b| f32::from(i16::from_ne_bytes(*b)) / 32768.0)
                                .collect()
                        };
                        let whole = samples.len() - samples.len() % self.channels.max(1);
                        if whole > 0 {
                            self.pending.push_back(Sample::Audio {
                                at,
                                samples: samples.get(..whole).unwrap_or_default().to_vec(),
                            });
                        }
                    }
                }
                (ndk.codec_release_output_buffer)(codec, idx, false);
                if info.flags & BUFFER_FLAG_END_OF_STREAM != 0 {
                    let t = if video { self.video.as_mut() } else { self.audio.as_mut() };
                    if let Some(t) = t {
                        t.output_done = true;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Copies a decoded picture out of a codec buffer, keeping its rows and planes.
fn picture(bytes: &[u8], l: Layout) -> Result<Picture, MediaError> {
    let luma = l.stride * l.slice_height;
    let (format, planes, need) = if l.format == COLOR_YUV420_SEMIPLANAR {
        let chroma = luma;
        (
            PixelFormat::Nv12,
            [
                Plane { offset: l.top * l.stride + l.left, stride: l.stride },
                Plane { offset: chroma + (l.top / 2) * l.stride + l.left, stride: l.stride },
                Plane::default(),
            ],
            chroma + l.stride * l.slice_height.div_ceil(2),
        )
    } else {
        let half = l.stride.div_ceil(2);
        let u = luma;
        let v = u + half * l.slice_height.div_ceil(2);
        (
            PixelFormat::I420,
            [
                Plane { offset: l.top * l.stride + l.left, stride: l.stride },
                Plane { offset: u + (l.top / 2) * half + l.left / 2, stride: half },
                Plane { offset: v + (l.top / 2) * half + l.left / 2, stride: half },
            ],
            v + half * l.slice_height.div_ceil(2),
        )
    };
    let data = bytes.get(..need.min(bytes.len())).unwrap_or_default();
    let p = Picture {
        width: l.width as u32,
        height: l.height as u32,
        format,
        planes,
        matrix: if l.height < 720 { YuvMatrix::Bt601 } else { YuvMatrix::Bt709 },
        full_range: false,
        data: Arc::from(data),
    };
    let check = remedy_ui_paint::VideoFrame {
        stream: 0,
        id: 0,
        width: p.width,
        height: p.height,
        format: p.format,
        matrix: p.matrix,
        full_range: p.full_range,
        planes: p.planes,
        data: p.data.clone(),
    };
    if !check.is_valid() {
        return Err(MediaError::Decode("a decoded picture is smaller than its layout says".into()));
    }
    Ok(p)
}

impl Decoder for NdkDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        loop {
            if let Some(s) = self.pending.pop_front() {
                return Ok(Some(s));
            }
            let all_done =
                [&self.video, &self.audio].iter().all(|t| t.as_ref().is_none_or(|t| t.output_done));
            if all_done {
                return Ok(None);
            }
            self.drain(0)?;
            if !self.pending.is_empty() {
                continue;
            }
            self.feed()?;
            if self.extractor_done {
                // Only the decoders' tails are left: wait for them.
                self.drain(CODEC_WAIT_US)?;
            }
        }
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        let ndk = self.ndk;
        self.pending.clear();
        let us = i64::try_from(to.as_micros()).unwrap_or(i64::MAX);
        // SAFETY: seeking the extractor and flushing live decoders.
        unsafe {
            if (ndk.extractor_seek_to)(self.extractor, us, SEEK_PREVIOUS_SYNC) != AMEDIA_OK {
                return Err(MediaError::Decode("the extractor refused the seek".into()));
            }
            for t in [self.video.as_mut(), self.audio.as_mut()].into_iter().flatten() {
                (ndk.codec_flush)(t.codec);
                t.input_done = false;
                t.output_done = false;
            }
        }
        self.extractor_done = false;
        Ok(())
    }
}

impl Drop for NdkDecoder {
    fn drop(&mut self) {
        let ndk = self.ndk;
        // SAFETY: stopping and deleting what `open` made.
        unsafe {
            for t in [self.video.take(), self.audio.take()].into_iter().flatten() {
                (ndk.codec_stop)(t.codec);
                (ndk.codec_delete)(t.codec);
            }
            (ndk.extractor_delete)(self.extractor);
        }
    }
}

/// An AAudio output stream in float samples.
struct AAudioOut {
    aa: &'static AAudio,
    stream: Ptr,
    format: AudioFormat,
    /// Frames presented when the clock was last set back to zero (a flush).
    base: i64,
}

// SAFETY: the stream is used from one thread at a time.
unsafe impl Send for AAudioOut {}

impl AAudioOut {
    fn open(aa: &'static AAudio, format: AudioFormat) -> Result<AAudioOut, MediaError> {
        // SAFETY: building and opening a stream; the builder is deleted after.
        unsafe {
            let mut builder: Ptr = std::ptr::null_mut();
            let r = (aa.create_stream_builder)(&mut builder);
            if r != AAUDIO_OK {
                return Err(aa.error("no stream builder", r));
            }
            (aa.builder_set_direction)(builder, AAUDIO_DIRECTION_OUTPUT);
            (aa.builder_set_format)(builder, AAUDIO_FORMAT_PCM_FLOAT);
            (aa.builder_set_channel_count)(builder, i32::from(format.channels));
            (aa.builder_set_sample_rate)(builder, i32::try_from(format.sample_rate).unwrap_or(i32::MAX));
            let mut stream: Ptr = std::ptr::null_mut();
            let r = (aa.builder_open_stream)(builder, &mut stream);
            (aa.builder_delete)(builder);
            if r != AAUDIO_OK || stream.is_null() {
                return Err(aa.error("the sound output would not open", r));
            }
            Ok(AAudioOut { aa, stream, format, base: 0 })
        }
    }

    fn presented(&self) -> i64 {
        let (mut frames, mut nanos) = (0i64, 0i64);
        // SAFETY: queries on a live stream into locals.
        unsafe {
            if (self.aa.stream_get_timestamp)(self.stream, CLOCK_MONOTONIC, &mut frames, &mut nanos)
                == AAUDIO_OK
            {
                frames
            } else {
                // Not playing yet (or paused): what the device has taken is what was heard.
                (self.aa.stream_get_frames_read)(self.stream)
            }
        }
    }
}

impl AudioOutput for AAudioOut {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn buffer(&self) -> Duration {
        // SAFETY: a query on a live stream.
        let frames = unsafe { (self.aa.stream_get_buffer_size_in_frames)(self.stream) };
        self.format.duration_of(u64::try_from(frames).unwrap_or(0))
    }

    fn room(&mut self) -> Result<usize, MediaError> {
        // SAFETY: queries on a live stream.
        let (size, written, read) = unsafe {
            (
                i64::from((self.aa.stream_get_buffer_size_in_frames)(self.stream)),
                (self.aa.stream_get_frames_written)(self.stream),
                (self.aa.stream_get_frames_read)(self.stream),
            )
        };
        Ok(usize::try_from(size - (written - read)).unwrap_or(0))
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let frames = samples.len() / usize::from(self.format.channels.max(1));
        let n = i32::try_from(frames).map_err(|_| MediaError::Device("too many frames at once".into()))?;
        // SAFETY: writing `n` whole frames from a live slice, without waiting.
        let r = unsafe { (self.aa.stream_write)(self.stream, samples.as_ptr().cast(), n, 0) };
        if r < 0 {
            return Err(self.aa.error("writing sound failed", r));
        }
        Ok(())
    }

    fn played(&mut self) -> Result<u64, MediaError> {
        Ok(u64::try_from(self.presented() - self.base).unwrap_or(0))
    }

    fn pause(&mut self) -> Result<(), MediaError> {
        // SAFETY: a state request on a live stream.
        let r = unsafe { (self.aa.stream_request_pause)(self.stream) };
        if r < 0 { Err(self.aa.error("pausing sound failed", r)) } else { Ok(()) }
    }

    fn resume(&mut self) -> Result<(), MediaError> {
        // SAFETY: as above.
        let r = unsafe { (self.aa.stream_request_start)(self.stream) };
        if r < 0 { Err(self.aa.error("starting sound failed", r)) } else { Ok(()) }
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        self.pause()?;
        // SAFETY: flushing a paused stream.
        let r = unsafe { (self.aa.stream_request_flush)(self.stream) };
        if r < 0 {
            return Err(self.aa.error("dropping queued sound failed", r));
        }
        // After a flush everything written counts as read: the clock starts again there.
        // SAFETY: a query on a live stream.
        self.base = unsafe { (self.aa.stream_get_frames_written)(self.stream) };
        Ok(())
    }
}

impl Drop for AAudioOut {
    fn drop(&mut self) {
        // SAFETY: closing the stream `open` made.
        unsafe { (self.aa.stream_close)(self.stream) };
    }
}

/// An AAudio input stream (the default microphone) in float samples, read without waiting.
struct AAudioIn {
    aa: &'static AAudio,
    stream: Ptr,
    format: AudioFormat,
    period: Duration,
}

// SAFETY: the stream is used from one thread at a time (the microphone's worker).
unsafe impl Send for AAudioIn {}

impl AAudioIn {
    fn open(aa: &'static AAudio, format: AudioFormat) -> Result<AAudioIn, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        // SAFETY: building, opening and starting a stream; the builder is deleted after, and
        // a stream that fails to start is closed.
        unsafe {
            let mut builder: Ptr = std::ptr::null_mut();
            let r = (aa.create_stream_builder)(&mut builder);
            if r != AAUDIO_OK {
                return Err(aa.error("no stream builder", r));
            }
            (aa.builder_set_direction)(builder, AAUDIO_DIRECTION_INPUT);
            (aa.builder_set_format)(builder, AAUDIO_FORMAT_PCM_FLOAT);
            (aa.builder_set_channel_count)(builder, i32::from(format.channels));
            (aa.builder_set_sample_rate)(builder, i32::try_from(format.sample_rate).unwrap_or(i32::MAX));
            (aa.builder_set_performance_mode)(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
            if let Some(preset) = aa.builder_set_input_preset {
                preset(builder, AAUDIO_INPUT_PRESET_UNPROCESSED);
            }
            let mut stream: Ptr = std::ptr::null_mut();
            let r = (aa.builder_open_stream)(builder, &mut stream);
            (aa.builder_delete)(builder);
            if r != AAUDIO_OK || stream.is_null() {
                // Without RECORD_AUDIO the open is refused.
                return Err(MediaError::Denied(format!(
                    "the microphone would not open ({}); the app needs the microphone permission",
                    aa.error("AAudio", r)
                )));
            }
            // AAudio converts to the asked-for rate on most devices; where it gives the
            // device's own instead, the microphone converts (see `capture`).
            let rate = (aa.stream_get_sample_rate)(stream);
            let channels = (aa.stream_get_channel_count)(stream);
            let format = match (u32::try_from(rate), u16::try_from(channels)) {
                (Ok(r), Ok(c)) if r > 0 && c > 0 => AudioFormat::new(r, c),
                _ => format,
            };
            let r = (aa.stream_request_start)(stream);
            if r != AAUDIO_OK {
                (aa.stream_close)(stream);
                return Err(aa.error("the microphone would not start", r));
            }
            let burst = (aa.stream_get_frames_per_burst)(stream);
            let period = format.duration_of(u64::try_from(burst).unwrap_or(0)).max(Duration::from_millis(1));
            Ok(AAudioIn { aa, stream, format, period })
        }
    }
}

impl AudioInput for AAudioIn {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn period(&self) -> Duration {
        self.period
    }

    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError> {
        let ch = usize::from(self.format.channels);
        let chunk = self.format.frames_in(self.period).max(64) as usize;
        let mut total = 0;
        loop {
            let at = into.len();
            into.resize(at + chunk * ch, 0.0);
            // SAFETY: reading at most `chunk` frames into room for exactly that many, without
            // waiting (timeout 0).
            let r = unsafe {
                (self.aa.stream_read)(
                    self.stream,
                    into.as_mut_ptr().add(at).cast(),
                    i32::try_from(chunk).unwrap_or(i32::MAX),
                    0,
                )
            };
            if r < 0 {
                into.truncate(at);
                return Err(self.aa.error("the microphone stopped", r));
            }
            let got = usize::try_from(r).unwrap_or(0);
            into.truncate(at + got * ch);
            total += got;
            if got < chunk {
                return Ok(total);
            }
        }
    }
}

impl Drop for AAudioIn {
    fn drop(&mut self) {
        // SAFETY: closing the stream `open` made (closing stops it).
        unsafe {
            (self.aa.stream_close)(self.stream);
        }
    }
}
