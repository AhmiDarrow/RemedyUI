//! macOS: AVFoundation's asset reader decodes (with the system's decoders, VideoToolbox
//! hardware included) to NV12 pictures and interleaved float sound, and an Audio Queue plays
//! the sound.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAsset, AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack, AVMediaTypeAudio,
    AVMediaTypeVideo, AVURLAsset,
};
use objc2_core_foundation::CFString;
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeRange, kCMTimePositiveInfinity};
use objc2_core_video::{
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use crate::host::{
    AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture, PixelFormat,
    Plane, Sample, VideoInfo, YuvMatrix,
};

/// Sound the output holds: the queue's own buffers plus as much waiting to go in, each this
/// long. Enough to ride out a busy machine; seeking and pausing drop or hold it.
const DEVICE_BUFFER: Duration = Duration::from_millis(200);
/// The Audio Queue cycles this many buffers (one playing, one ready, one being filled).
const QUEUE_BUFFERS: usize = 3;
/// `kAudioFormatLinearPCM`, `kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked`.
const LINEAR_PCM: u32 = 0x6C70_636D;
const FLOAT_PACKED: u32 = 1 | 8;

#[repr(C)]
struct Asbd {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct QueueBuffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user: *mut c_void,
    packet_capacity: u32,
    packets: *mut c_void,
    packet_count: u32,
}

#[repr(C)]
#[derive(Default)]
struct SmpteTime {
    subframes: i16,
    subframe_divisor: i16,
    counter: u32,
    kind: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}

#[repr(C)]
#[derive(Default)]
struct AudioTimeStamp {
    sample_time: f64,
    host_time: u64,
    rate_scalar: f64,
    word_clock_time: u64,
    smpte: SmpteTime,
    flags: u32,
    reserved: u32,
}

type QueueRef = *mut c_void;
type QueueCallback = unsafe extern "C" fn(*mut c_void, QueueRef, *mut QueueBuffer);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        format: *const Asbd,
        callback: QueueCallback,
        user: *mut c_void,
        run_loop: *const c_void,
        mode: *const c_void,
        flags: u32,
        out: *mut QueueRef,
    ) -> i32;
    fn AudioQueueAllocateBuffer(queue: QueueRef, bytes: u32, out: *mut *mut QueueBuffer) -> i32;
    fn AudioQueueEnqueueBuffer(
        queue: QueueRef,
        buffer: *mut QueueBuffer,
        packets: u32,
        descs: *const c_void,
    ) -> i32;
    fn AudioQueueStart(queue: QueueRef, at: *const AudioTimeStamp) -> i32;
    fn AudioQueuePause(queue: QueueRef) -> i32;
    fn AudioQueueDispose(queue: QueueRef, immediate: u8) -> i32;
    fn AudioQueueGetCurrentTime(
        queue: QueueRef,
        timeline: *mut c_void,
        out: *mut AudioTimeStamp,
        discontinuity: *mut u8,
    ) -> i32;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMAudioFormatDescriptionGetStreamBasicDescription(desc: *const c_void) -> *const Asbd;
}

struct MacHost;

pub(super) fn host() -> Result<Arc<dyn MediaHost>, MediaError> {
    Ok(Arc::new(MacHost))
}

impl MediaHost for MacHost {
    fn name(&self) -> &str {
        "AVFoundation"
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        // shortcut: no Elementary here yet, so a playlist is refused plainly rather than failing in the decoder.
        if crate::hls::is_hls(source) {
            return Err(MediaError::Unsupported("HLS is not read on macOS yet".into()));
        }
        Ok(Box::new(AvDecoder::open(source)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(QueueOut::open(format, crate::host::Latency::Playback)?))
    }

    fn audio_output_with(
        &self,
        format: AudioFormat,
        latency: crate::host::Latency,
    ) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(QueueOut::open(format, latency)?))
    }
}

/// A reader and its picture and sound outputs.
type ReaderParts = (
    Retained<AVAssetReader>,
    Option<Retained<AVAssetReaderTrackOutput>>,
    Option<Retained<AVAssetReaderTrackOutput>>,
);

/// One track's reader output and the sample it has ready.
struct Branch {
    track: Retained<AVAssetTrack>,
    output: Retained<AVAssetReaderTrackOutput>,
    head: Option<Retained<CMSampleBuffer>>,
    done: bool,
}

struct AvDecoder {
    asset: Retained<AVURLAsset>,
    reader: Retained<AVAssetReader>,
    video: Option<Branch>,
    audio: Option<Branch>,
    channels: usize,
    info: MediaInfo,
}

// SAFETY: AVAssetReader and its outputs may be used from any thread, one at a time, which is
// how the player uses a decoder.
unsafe impl Send for AvDecoder {}

fn ns_error(e: &objc2_foundation::NSError) -> String {
    e.localizedDescription().to_string()
}

/// The reader output settings: NV12 (video range) pictures, or packed float sound at the
/// track's own rate and channels.
fn settings(video: bool) -> Retained<NSDictionary<NSString, AnyObject>> {
    if video {
        // SAFETY: CFString and NSString are toll-free bridged.
        let key: &NSString =
            unsafe { &*std::ptr::from_ref::<CFString>(kCVPixelBufferPixelFormatTypeKey).cast() };
        let value = NSNumber::numberWithUnsignedInt(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
        let v: &AnyObject = &value;
        NSDictionary::from_slices(&[key], &[v])
    } else {
        let keys = [
            NSString::from_str("AVFormatIDKey"),
            NSString::from_str("AVLinearPCMBitDepthKey"),
            NSString::from_str("AVLinearPCMIsFloatKey"),
            NSString::from_str("AVLinearPCMIsNonInterleaved"),
            NSString::from_str("AVLinearPCMIsBigEndianKey"),
        ];
        let values = [
            NSNumber::numberWithUnsignedInt(LINEAR_PCM),
            NSNumber::numberWithUnsignedInt(32),
            NSNumber::numberWithBool(true),
            NSNumber::numberWithBool(false),
            NSNumber::numberWithBool(false),
        ];
        let k: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let v: Vec<&AnyObject> = values.iter().map(|v| -> &AnyObject { v }).collect();
        NSDictionary::from_slices(&k, &v)
    }
}

/// The asset's first track of `kind`. The synchronous track list is deprecated in favor of
/// the asynchronous loader; the decoder runs on its own thread, where waiting is what it does.
#[allow(deprecated)]
fn first_track(asset: &AVAsset, kind: Option<&'static NSString>) -> Option<Retained<AVAssetTrack>> {
    // SAFETY: reading the tracks of a live asset.
    kind.and_then(|k| unsafe { asset.tracksWithMediaType(k) }.firstObject())
}

fn seconds(t: CMTime) -> Option<Duration> {
    // SAFETY: a pure conversion of a time value.
    let s = unsafe { t.seconds() };
    (s.is_finite() && s >= 0.0).then(|| Duration::from_secs_f64(s))
}

impl AvDecoder {
    fn open(source: &MediaSource) -> Result<AvDecoder, MediaError> {
        let url = match source {
            MediaSource::File(p) => {
                if !p.exists() {
                    return Err(MediaError::NotFound(p.display().to_string()));
                }
                let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
                NSURL::from_file_path(&abs).ok_or_else(|| MediaError::NotFound(abs.display().to_string()))?
            }
            MediaSource::Url(u) => NSURL::URLWithString(&NSString::from_str(u))
                .ok_or_else(|| MediaError::NotFound(format!("{u} is not a URL")))?,
            MediaSource::Bytes(_) => {
                return Err(MediaError::Unsupported(
                    "bytes in memory are opened through `open_source`".into(),
                ));
            }
        };
        // SAFETY: AVFoundation calls on objects made here.
        unsafe {
            let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
            let (vt, at) = (first_track(&asset, AVMediaTypeVideo), first_track(&asset, AVMediaTypeAudio));
            if vt.is_none() && at.is_none() {
                return Err(MediaError::Format("no sound or video track AVFoundation can read".into()));
            }
            let mut info = MediaInfo::default();
            #[allow(deprecated)]
            let duration = asset.duration();
            info.duration = seconds(duration).filter(|d| !d.is_zero());
            info.seekable = info.duration.is_some();
            let mut d = AvDecoder {
                reader: Self::reader(&asset, None, vt.as_deref(), at.as_deref())?.0,
                asset,
                video: None,
                audio: None,
                channels: 0,
                info,
            };
            d.start(None, vt, at)?;
            d.describe()?;
            Ok(d)
        }
    }

    /// A reader of `asset` from `from`, with outputs for the tracks given.
    fn reader(
        asset: &AVAsset,
        from: Option<Duration>,
        video: Option<&AVAssetTrack>,
        audio: Option<&AVAssetTrack>,
    ) -> Result<ReaderParts, MediaError> {
        // SAFETY: AVFoundation calls on live objects.
        unsafe {
            let reader = AVAssetReader::assetReaderWithAsset_error(asset)
                .map_err(|e| MediaError::Format(ns_error(&e)))?;
            if let Some(t) = from {
                let start = CMTime::new(i64::try_from(t.as_micros()).unwrap_or(i64::MAX), 1_000_000);
                reader.setTimeRange(CMTimeRange { start, duration: kCMTimePositiveInfinity });
            }
            let mut outs = [None, None];
            for (i, (track, video)) in [(video, true), (audio, false)].into_iter().enumerate() {
                let Some(track) = track else { continue };
                let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                    track,
                    Some(&settings(video)),
                );
                out.setAlwaysCopiesSampleData(false);
                if reader.canAddOutput(&out) {
                    reader.addOutput(&out);
                    if let Some(slot) = outs.get_mut(i) {
                        *slot = Some(out);
                    }
                }
            }
            let [v, a] = outs;
            Ok((reader, v, a))
        }
    }

    /// Starts reading from `from` (the start when `None`).
    fn start(
        &mut self,
        from: Option<Duration>,
        video: Option<Retained<AVAssetTrack>>,
        audio: Option<Retained<AVAssetTrack>>,
    ) -> Result<(), MediaError> {
        let (reader, vo, ao) = Self::reader(&self.asset, from, video.as_deref(), audio.as_deref())?;
        // SAFETY: starting a reader made here.
        unsafe {
            if !reader.startReading() {
                let why =
                    reader.error().map_or_else(|| "the reader would not start".to_string(), |e| ns_error(&e));
                return Err(MediaError::Format(why));
            }
        }
        self.reader = reader;
        self.video = video.zip(vo).map(|(track, output)| Branch { track, output, head: None, done: false });
        self.audio = audio.zip(ao).map(|(track, output)| Branch { track, output, head: None, done: false });
        if self.video.is_none() && self.audio.is_none() {
            return Err(MediaError::Format("no track AVFoundation can decode".into()));
        }
        Ok(())
    }

    /// Reads the formats from the first sample of each track (kept as that track's head).
    fn describe(&mut self) -> Result<(), MediaError> {
        if let Some(b) = self.video.as_mut() {
            // SAFETY: reading a sample and its picture's size.
            unsafe {
                b.head = b.output.copyNextSampleBuffer();
                let image = b.head.as_ref().and_then(|s| s.image_buffer());
                let Some(image) = image else {
                    return Err(MediaError::Format("the video gave no first picture".into()));
                };
                let fps = b.track.nominalFrameRate();
                self.info.video = Some(VideoInfo {
                    width: CVPixelBufferGetWidth(&image) as u32,
                    height: CVPixelBufferGetHeight(&image) as u32,
                    frame_rate: (fps > 0.0).then_some(f64::from(fps)),
                });
            }
        }
        if let Some(b) = self.audio.as_mut() {
            // SAFETY: reading a sample and its format description.
            unsafe {
                b.head = b.output.copyNextSampleBuffer();
                let desc = b.head.as_ref().and_then(|s| s.format_description());
                let asbd = desc
                    .map(|d| {
                        CMAudioFormatDescriptionGetStreamBasicDescription(std::ptr::from_ref(&*d).cast())
                    })
                    .filter(|p| !p.is_null());
                let Some(asbd) = asbd else {
                    return Err(MediaError::Format("the sound gave no first samples".into()));
                };
                let (rate, ch) = ((*asbd).sample_rate, (*asbd).channels_per_frame);
                if rate.is_nan() || rate <= 0.0 || ch == 0 {
                    return Err(MediaError::Format("the sound has no rate or channels".into()));
                }
                self.channels = ch as usize;
                self.info.audio = Some(AudioFormat {
                    sample_rate: rate.round() as u32,
                    channels: u16::try_from(ch).unwrap_or(u16::MAX),
                });
            }
        }
        Ok(())
    }

    fn fill(&mut self) {
        for b in [self.video.as_mut(), self.audio.as_mut()].into_iter().flatten() {
            if b.head.is_none() && !b.done {
                // SAFETY: reading the next sample of a live output.
                b.head = unsafe { b.output.copyNextSampleBuffer() };
                b.done = b.head.is_none();
            }
        }
    }

    fn picture(&self, sample: &CMSampleBuffer) -> Result<Picture, MediaError> {
        // SAFETY: the pixel buffer is locked for reading while its planes are copied.
        unsafe {
            let image = sample
                .image_buffer()
                .ok_or_else(|| MediaError::Decode("a sample without a picture".into()))?;
            let format = CVPixelBufferGetPixelFormatType(&image);
            let full_range = format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;
            if format != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange && !full_range {
                return Err(MediaError::Decode(format!("pictures came as {format:#x}, not NV12")));
            }
            if CVPixelBufferGetPlaneCount(&image) != 2 {
                return Err(MediaError::Decode("an NV12 picture without two planes".into()));
            }
            CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
            let (w, h) = (CVPixelBufferGetWidth(&image), CVPixelBufferGetHeight(&image));
            let mut data = Vec::new();
            let mut planes = [Plane::default(); 3];
            let mut ok = true;
            for (i, slot) in planes.iter_mut().take(2).enumerate() {
                let base = CVPixelBufferGetBaseAddressOfPlane(&image, i).cast::<u8>();
                let stride = CVPixelBufferGetBytesPerRowOfPlane(&image, i);
                let rows = CVPixelBufferGetHeightOfPlane(&image, i);
                if base.is_null() {
                    ok = false;
                    break;
                }
                *slot = Plane { offset: data.len(), stride };
                data.extend_from_slice(std::slice::from_raw_parts(base, stride * rows));
            }
            CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
            if !ok {
                return Err(MediaError::Decode("a picture's planes could not be read".into()));
            }
            Ok(Picture {
                width: w as u32,
                height: h as u32,
                format: PixelFormat::Nv12,
                planes,
                matrix: if h < 720 { YuvMatrix::Bt601 } else { YuvMatrix::Bt709 },
                full_range,
                data: Arc::from(data),
                gpu: None,
            })
        }
    }

    fn sound(&self, sample: &CMSampleBuffer) -> Result<Vec<f32>, MediaError> {
        // SAFETY: copying the block buffer's bytes into memory of exactly that length.
        unsafe {
            let Some(block) = sample.data_buffer() else { return Ok(Vec::new()) };
            let len = block.data_length();
            let mut bytes = vec![0u8; len];
            if let Some(dst) = NonNull::new(bytes.as_mut_ptr().cast::<c_void>())
                && len > 0
                && block.copy_data_bytes(0, len, dst) != 0
            {
                return Err(MediaError::Decode("the sound's bytes could not be read".into()));
            }
            let samples: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
            let whole = samples.len() - samples.len() % self.channels.max(1);
            Ok(samples.get(..whole).unwrap_or_default().to_vec())
        }
    }
}

// `buffered` is left at its default (`None`): the asset reader has no buffering query, and
// AVFoundation's `loadedTimeRanges` belongs to a player item, which is main-thread-only and
// loads the media on its own rather than saying what the reader has. The seek bar shows what
// is decoded ahead.
impl Decoder for AvDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        loop {
            self.fill();
            // SAFETY: reading a sample's time.
            let at = |b: &Option<Branch>| {
                b.as_ref()
                    .and_then(|b| b.head.as_ref())
                    .map(|s| unsafe { seconds(s.presentation_time_stamp()) })
            };
            let (va, aa) = (at(&self.video), at(&self.audio));
            let take_video = match (va, aa) {
                (None, None) => {
                    // SAFETY: reading the reader's state.
                    let failed = unsafe { self.reader.status() } == AVAssetReaderStatus::Failed;
                    if failed {
                        // SAFETY: as above.
                        let why = unsafe { self.reader.error() }
                            .map_or_else(|| "decoding failed".to_string(), |e| ns_error(&e));
                        return Err(MediaError::Decode(why));
                    }
                    return Ok(None);
                }
                (Some(v), Some(a)) => v.unwrap_or_default() <= a.unwrap_or_default(),
                (Some(_), None) => true,
                (None, Some(_)) => false,
            };
            let branch = if take_video { self.video.as_mut() } else { self.audio.as_mut() };
            let Some(sample) = branch.and_then(|b| b.head.take()) else { continue };
            // SAFETY: reading a sample's time.
            let at = unsafe { seconds(sample.presentation_time_stamp()) }.unwrap_or_default();
            if take_video {
                // SAFETY: as above.
                let duration = unsafe { seconds(sample.duration()) }.filter(|d| !d.is_zero());
                let picture = self.picture(&sample)?;
                return Ok(Some(Sample::Video { at, duration, picture }));
            }
            let samples = self.sound(&sample)?;
            if !samples.is_empty() {
                return Ok(Some(Sample::Audio { at, samples }));
            }
        }
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        // An asset reader reads one range once: a new one starts at the target.
        // SAFETY: cancelling the reader in use.
        unsafe { self.reader.cancelReading() };
        let video = self.video.take().map(|b| b.track);
        let audio = self.audio.take().map(|b| b.track);
        self.start(Some(to), video, audio)
    }
}

/// What the Audio Queue's callback plays from, and what it has played.
struct Ring {
    samples: VecDeque<f32>,
    channels: usize,
    /// Frames handed to the queue so far, sound and silence.
    handed: u64,
    /// The buffers handed and not yet known to be played: (first frame, frames of sound,
    /// frames in all).
    log: VecDeque<(u64, u64, u64)>,
    /// Frames of sound in buffers already played.
    played: u64,
}

impl Ring {
    /// Frames of sound heard once the queue has played `at` frames (sound and silence).
    fn heard(&mut self, at: u64) -> u64 {
        while let Some(&(start, real, len)) = self.log.front() {
            if start + len > at {
                break;
            }
            self.played += real;
            self.log.pop_front();
        }
        let partial = self.log.front().map_or(0, |&(start, real, _)| at.saturating_sub(start).min(real));
        self.played + partial
    }

    /// Fills `buf` with what is waiting (silence after it), and logs it.
    fn fill(&mut self, buf: &mut QueueBuffer) {
        let ch = self.channels.max(1);
        let frames = buf.capacity as usize / (ch * 4);
        let real = frames.min(self.samples.len() / ch);
        if !buf.data.is_null() {
            // SAFETY: the buffer holds `capacity` bytes, and `frames * ch` floats fit in it.
            let dst = unsafe { std::slice::from_raw_parts_mut(buf.data.cast::<f32>(), frames * ch) };
            for (d, s) in dst.iter_mut().zip(self.samples.drain(..real * ch).chain(std::iter::repeat(0.0))) {
                *d = s;
            }
        }
        buf.size = u32::try_from(frames * ch * 4).unwrap_or(0);
        self.log.push_back((self.handed, real as u64, frames as u64));
        self.handed += frames as u64;
    }
}

type Shared = Arc<Mutex<Ring>>;

unsafe extern "C" fn refill(user: *mut c_void, queue: QueueRef, buf: *mut QueueBuffer) {
    // SAFETY: `user` is the ring this queue was made with, alive until the queue is disposed;
    // `buf` is one of its buffers, handed back to be refilled.
    unsafe {
        let ring = &*user.cast::<Mutex<Ring>>();
        if let Some(b) = buf.as_mut() {
            ring.lock().unwrap_or_else(PoisonError::into_inner).fill(b);
            AudioQueueEnqueueBuffer(queue, buf, 0, std::ptr::null());
        }
    }
}

/// An Audio Queue and the ring its callback plays from.
struct QueueOut {
    queue: QueueRef,
    ring: Shared,
    /// The ring's pointer given to the queue (one strong count, released after disposal).
    user: *const Mutex<Ring>,
    format: AudioFormat,
    /// How much sound the queue's buffers hold together (and as much again waits to go in).
    depth: Duration,
    capacity: usize,
    started: bool,
    /// The queue's time when last read, for while it is paused.
    last: u64,
}

// SAFETY: the queue is used from one thread at a time; its callback runs on the queue's own
// thread and touches only the ring, behind its lock.
unsafe impl Send for QueueOut {}

impl QueueOut {
    fn open(format: AudioFormat, latency: crate::host::Latency) -> Result<QueueOut, MediaError> {
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        // Interactive: three buffers of 4 ms, and as much waiting.
        let depth = match latency {
            crate::host::Latency::Interactive => Duration::from_millis(12),
            _ => DEVICE_BUFFER,
        };
        let ch = usize::from(format.channels);
        let capacity = usize::try_from(format.frames_in(depth)).unwrap_or(usize::MAX);
        let ring = Arc::new(Mutex::new(Ring {
            samples: VecDeque::with_capacity(capacity * ch),
            channels: ch,
            handed: 0,
            log: VecDeque::new(),
            played: 0,
        }));
        let (queue, user) = Self::make(&ring, format, depth)?;
        Ok(QueueOut { queue, ring, user, format, depth, capacity, started: false, last: 0 })
    }

    /// A new queue playing from `ring`, with its buffers primed.
    fn make(
        ring: &Shared,
        format: AudioFormat,
        depth: Duration,
    ) -> Result<(QueueRef, *const Mutex<Ring>), MediaError> {
        let ch = u32::from(format.channels);
        let asbd = Asbd {
            sample_rate: f64::from(format.sample_rate),
            format_id: LINEAR_PCM,
            format_flags: FLOAT_PACKED,
            bytes_per_packet: 4 * ch,
            frames_per_packet: 1,
            bytes_per_frame: 4 * ch,
            channels_per_frame: ch,
            bits_per_channel: 32,
            reserved: 0,
        };
        let user = Arc::into_raw(ring.clone());
        let mut queue: QueueRef = std::ptr::null_mut();
        // SAFETY: making a queue whose callback gets `user` (kept alive until disposal), then
        // allocating, filling and enqueueing its buffers.
        unsafe {
            let r = AudioQueueNewOutput(
                &asbd,
                refill,
                user.cast_mut().cast(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut queue,
            );
            if r != 0 || queue.is_null() {
                drop(Arc::from_raw(user));
                return Err(MediaError::Device(format!("no sound output (Audio Queue error {r})")));
            }
            let frames = (format.frames_in(depth) / QUEUE_BUFFERS as u64).max(1);
            let bytes = u32::try_from(frames * u64::from(ch) * 4).unwrap_or(u32::MAX);
            for _ in 0..QUEUE_BUFFERS {
                let mut buf: *mut QueueBuffer = std::ptr::null_mut();
                if AudioQueueAllocateBuffer(queue, bytes, &mut buf) != 0 {
                    continue;
                }
                if let Some(b) = buf.as_mut() {
                    ring.lock().unwrap_or_else(PoisonError::into_inner).fill(b);
                    AudioQueueEnqueueBuffer(queue, buf, 0, std::ptr::null());
                }
            }
        }
        Ok((queue, user))
    }

    fn dispose(&mut self) {
        if self.queue.is_null() {
            return;
        }
        // SAFETY: disposing synchronously stops the callback; only then is its ring released.
        unsafe {
            AudioQueueDispose(self.queue, 1);
            drop(Arc::from_raw(self.user));
        }
        self.queue = std::ptr::null_mut();
        self.user = std::ptr::null();
    }
}

impl AudioOutput for QueueOut {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn buffer(&self) -> Duration {
        self.depth * 2
    }

    fn room(&mut self) -> Result<usize, MediaError> {
        if self.queue.is_null() {
            return Err(MediaError::Device("the sound output could not be made again".into()));
        }
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.capacity.saturating_sub(ring.samples.len() / ring.channels.max(1)))
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        self.ring.lock().unwrap_or_else(PoisonError::into_inner).samples.extend(samples.iter().copied());
        Ok(())
    }

    fn played(&mut self) -> Result<u64, MediaError> {
        let mut t = AudioTimeStamp::default();
        // SAFETY: a query into a local on a live queue.
        let r = unsafe {
            AudioQueueGetCurrentTime(self.queue, std::ptr::null_mut(), &mut t, std::ptr::null_mut())
        };
        if r == 0 && t.sample_time.is_finite() && t.sample_time >= 0.0 {
            self.last = t.sample_time as u64;
        }
        Ok(self.ring.lock().unwrap_or_else(PoisonError::into_inner).heard(self.last))
    }

    fn pause(&mut self) -> Result<(), MediaError> {
        if self.started {
            // SAFETY: pausing a live queue.
            let r = unsafe { AudioQueuePause(self.queue) };
            if r != 0 {
                return Err(MediaError::Device(format!("pausing sound failed (Audio Queue error {r})")));
            }
            self.started = false;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), MediaError> {
        if !self.started {
            // SAFETY: starting a live queue now.
            let r = unsafe { AudioQueueStart(self.queue, std::ptr::null()) };
            if r != 0 {
                return Err(MediaError::Device(format!("starting sound failed (Audio Queue error {r})")));
            }
            self.started = true;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        // A new queue drops what the old one held and starts its clock at zero.
        self.dispose();
        {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            ring.samples.clear();
            ring.log.clear();
            ring.handed = 0;
            ring.played = 0;
        }
        self.started = false;
        self.last = 0;
        let (queue, user) = Self::make(&self.ring, self.format, self.depth)?;
        self.queue = queue;
        self.user = user;
        Ok(())
    }
}

impl Drop for QueueOut {
    fn drop(&mut self) {
        self.dispose();
    }
}
