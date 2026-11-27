//! macOS HLS: the H.264, AAC and MP3 streams the crate's own HLS reader takes apart, decoded
//! by VideoToolbox (to the same NV12 pictures the asset reader gives) and an AudioToolbox
//! converter (to interleaved float), and playlists and segments fetched through NSURLSession.
//!
//! VideoToolbox, the converter and the session's completion handler are reached through their
//! C interfaces, declared here: no binding crate covers them.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Sel;
use objc2::sel;
use objc2_core_media::{CMTime, kCMTimeInvalid};
use objc2_core_video::CVPixelBuffer;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLRequest,
    NSURLRequestCachePolicy, NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURLSessionDataTask,
};

use super::{Asbd, FLOAT_PACKED, LINEAR_PCM, nv12, settings};
use crate::hls::{
    AudioCodec, AudioConfig, Elementary, Fetch, Unit, VideoConfig, avcc, esds, file_path, first_byte, load,
    quiet, timed_out,
};
use crate::host::{MediaError, Picture, Sample};

/// `kVTVideoDecoderBadDataErr`: a damaged frame, skipped as a damaged frame is.
const BAD_DATA: i32 = -12909;
/// `kCMBlockBufferAssureMemoryNowFlag`.
const ASSURE_MEMORY_NOW: u32 = 1;
const AAC: u32 = u32::from_be_bytes(*b"aac ");
const MP3: u32 = u32::from_be_bytes(*b".mp3");
/// `kAudioConverterDecompressionMagicCookie`.
const MAGIC_COOKIE: u32 = u32::from_be_bytes(*b"dmgc");
/// What the converter's input callback says when it has nothing more for now (not one of the
/// converter's own errors): the converter hands out what it has and stops.
const NO_MORE: i32 = i32::from_be_bytes(*b"nmor");
/// Frames of sound taken from the converter at once (an AAC packet is 1024, HE-AAC 2048).
const OUT_FRAMES: usize = 4096;
/// Sound this far from where the last left off starts the count again (a discontinuity).
const RESYNC: Duration = Duration::from_millis(500);
/// `NSURLErrorTimedOut`.
const URL_TIMED_OUT: isize = -1001;

type OutputCallback = unsafe extern "C" fn(
    refcon: *mut c_void,
    frame: *mut c_void,
    status: i32,
    flags: u32,
    image: *mut CVPixelBuffer,
    pts: CMTime,
    duration: CMTime,
);

#[repr(C)]
struct CallbackRecord {
    callback: OutputCallback,
    refcon: *mut c_void,
}

#[repr(C)]
struct SampleTiming {
    duration: CMTime,
    presentation: CMTime,
    decode: CMTime,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
struct PacketDesc {
    start: i64,
    frames: u32,
    size: u32,
}

type InputProc = unsafe extern "C" fn(
    converter: *mut c_void,
    packets: *mut u32,
    data: *mut AudioBufferList,
    descs: *mut *mut PacketDesc,
    user: *mut c_void,
) -> i32;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: *const c_void,
        count: usize,
        sets: *const *const u8,
        sizes: *const usize,
        nal_length_size: i32,
        out: *mut *mut c_void,
    ) -> i32;
    fn CMBlockBufferCreateWithMemoryBlock(
        allocator: *const c_void,
        memory: *mut c_void,
        block_length: usize,
        block_allocator: *const c_void,
        custom_source: *const c_void,
        offset: usize,
        data_length: usize,
        flags: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn CMBlockBufferReplaceDataBytes(
        source: *const c_void,
        block: *mut c_void,
        offset: usize,
        len: usize,
    ) -> i32;
    fn CMSampleBufferCreateReady(
        allocator: *const c_void,
        data: *mut c_void,
        format: *const c_void,
        samples: isize,
        timings: isize,
        timing: *const SampleTiming,
        sizes: isize,
        size: *const usize,
        out: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(
        allocator: *const c_void,
        format: *const c_void,
        decoder_specification: *const c_void,
        image_attributes: *const c_void,
        callback: *const CallbackRecord,
        out: *mut *mut c_void,
    ) -> i32;
    fn VTDecompressionSessionDecodeFrame(
        session: *mut c_void,
        sample: *mut c_void,
        flags: u32,
        frame: *mut c_void,
        info: *mut u32,
    ) -> i32;
    fn VTDecompressionSessionFinishDelayedFrames(session: *mut c_void) -> i32;
    fn VTDecompressionSessionWaitForAsynchronousFrames(session: *mut c_void) -> i32;
    fn VTDecompressionSessionInvalidate(session: *mut c_void);
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioConverterNew(input: *const Asbd, output: *const Asbd, out: *mut *mut c_void) -> i32;
    fn AudioConverterSetProperty(converter: *mut c_void, id: u32, size: u32, data: *const c_void) -> i32;
    fn AudioConverterFillComplexBuffer(
        converter: *mut c_void,
        input: InputProc,
        user: *mut c_void,
        packets: *mut u32,
        output: *mut AudioBufferList,
        descs: *mut PacketDesc,
    ) -> i32;
    fn AudioConverterReset(converter: *mut c_void) -> i32;
    fn AudioConverterDispose(converter: *mut c_void) -> i32;
}

fn cm_time(d: Duration) -> CMTime {
    // SAFETY: a pure constructor.
    unsafe { CMTime::new(i64::try_from(d.as_nanos()).unwrap_or(i64::MAX), 1_000_000_000) }
}

fn from_cm_time(t: CMTime) -> Duration {
    if t.timescale <= 0 {
        return Duration::ZERO;
    }
    let nanos = i128::from(t.value) * 1_000_000_000 / i128::from(t.timescale);
    Duration::from_nanos(u64::try_from(nanos.max(0)).unwrap_or(u64::MAX))
}

/// What the decompression session's callback has handed out since last asked.
#[derive(Default)]
struct Decoded {
    pictures: Vec<(Duration, Picture)>,
    error: Option<MediaError>,
}

unsafe extern "C" fn decoded(
    refcon: *mut c_void,
    _frame: *mut c_void,
    status: i32,
    _flags: u32,
    image: *mut CVPixelBuffer,
    pts: CMTime,
    _duration: CMTime,
) {
    // SAFETY: `refcon` is the session's `Decoded`, alive until the session is invalidated;
    // `image` is the session's for the length of the call (or null for a frame not decoded).
    let (d, image) = unsafe { (&*refcon.cast::<Mutex<Decoded>>(), image.as_ref()) };
    let mut d = d.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(image) = image else {
        if status != 0 && status != BAD_DATA {
            d.error.get_or_insert(MediaError::Decode(format!(
                "VideoToolbox could not decode a picture (error {status})"
            )));
        }
        return;
    };
    match nv12(image) {
        Ok(p) => d.pictures.push((from_cm_time(pts), p)),
        Err(e) => {
            d.error.get_or_insert(e);
        }
    }
}

/// A VideoToolbox decompression session for one H.264 stream.
struct Video {
    /// The `CMVideoFormatDescription` made from the stream's SPS and PPS.
    format: *mut c_void,
    session: *mut c_void,
    /// Where the session's callback puts pictures (boxed: the session holds its address).
    decoded: Box<Mutex<Decoded>>,
    /// Pictures decoded and not yet handed out, in presentation order. VideoToolbox gives them
    /// in decoding order; one goes out once no frame decoded later can come before it.
    held: Vec<(Duration, Picture)>,
    out: VecDeque<Sample>,
    /// The decoding time of the last unit pushed: every frame decoded after it shows at or
    /// after it.
    dts: Duration,
}

// SAFETY: the session and format description may be used from any thread, one at a time,
// which is how the core uses an `Elementary`; the callback touches only `decoded`, behind its lock.
unsafe impl Send for Video {}

impl Video {
    fn open(c: &VideoConfig) -> Result<Video, MediaError> {
        // shortcut: the first SPS and PPS set the session up for the whole stream; make a new
        // session at a key frame whose parameter sets differ if streams that change them come up.
        let sets = [c.sps.as_ptr(), c.pps.as_ptr()];
        let sizes = [c.sps.len(), c.pps.len()];
        let mut v = Video {
            format: null_mut(),
            session: null_mut(),
            decoded: Box::default(),
            held: Vec::new(),
            out: VecDeque::new(),
            dts: Duration::ZERO,
        };
        // SAFETY: the parameter sets outlive the call, and the description is checked.
        let r = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                null(),
                2,
                sets.as_ptr(),
                sizes.as_ptr(),
                4,
                &mut v.format,
            )
        };
        if r != 0 || v.format.is_null() {
            return Err(MediaError::Format(format!("the H.264 parameter sets were refused (error {r})")));
        }
        let record =
            CallbackRecord { callback: decoded, refcon: std::ptr::from_ref(&*v.decoded).cast_mut().cast() };
        let attributes = settings(true);
        // SAFETY: the session copies the callback record; its refcon, `decoded`, is boxed and
        // outlives the session (invalidated on drop, before the box goes). The attributes
        // dictionary is toll-free bridged to a CFDictionary.
        let r = unsafe {
            VTDecompressionSessionCreate(
                null(),
                v.format,
                null(),
                Retained::as_ptr(&attributes).cast(),
                &record,
                &mut v.session,
            )
        };
        if r != 0 || v.session.is_null() {
            v.session = null_mut();
            return Err(MediaError::Unsupported(format!(
                "VideoToolbox will not decode this H.264 stream ({}, error {r})",
                c.codec
            )));
        }
        Ok(v)
    }

    /// One AVCC sample in a `CMSampleBuffer` (released by the caller).
    fn sample(&self, data: &[u8], pts: Duration, dts: Duration) -> Result<*mut c_void, MediaError> {
        let len = data.len();
        let (mut block, mut sample) = (null_mut(), null_mut());
        // SAFETY: a block buffer of `len` bytes is made and `data` copied into it; the sample
        // buffer made from it holds its own reference, so the block is released here.
        let r = unsafe {
            let mut r = CMBlockBufferCreateWithMemoryBlock(
                null(),
                null_mut(),
                len,
                null(),
                null(),
                0,
                len,
                ASSURE_MEMORY_NOW,
                &mut block,
            );
            if r == 0 {
                r = CMBlockBufferReplaceDataBytes(data.as_ptr().cast(), block, 0, len);
            }
            if r == 0 {
                let timing = SampleTiming {
                    duration: kCMTimeInvalid,
                    presentation: cm_time(pts),
                    decode: cm_time(dts),
                };
                r = CMSampleBufferCreateReady(
                    null(),
                    block,
                    self.format,
                    1,
                    1,
                    &timing,
                    1,
                    &len,
                    &mut sample,
                );
            }
            if !block.is_null() {
                CFRelease(block);
            }
            r
        };
        if r != 0 || sample.is_null() {
            return Err(MediaError::Decode(format!("an H.264 sample could not be made (error {r})")));
        }
        Ok(sample)
    }

    fn push(&mut self, data: &[u8], pts: Duration, dts: Duration) -> Result<(), MediaError> {
        let data = avcc(data);
        if data.is_empty() {
            return Ok(());
        }
        let sample = self.sample(&data, pts, dts)?;
        let mut info = 0u32;
        // SAFETY: decoding (synchronously: no flags) a sample made here, then releasing it.
        let r = unsafe {
            let r = VTDecompressionSessionDecodeFrame(self.session, sample, 0, null_mut(), &mut info);
            CFRelease(sample);
            r
        };
        if r != 0 && r != BAD_DATA {
            return Err(MediaError::Decode(format!("VideoToolbox could not decode a frame (error {r})")));
        }
        self.dts = dts;
        self.collect(false)
    }

    /// Takes what the callback handed out; hands out what is due (`all`: everything held).
    fn collect(&mut self, all: bool) -> Result<(), MediaError> {
        {
            let mut d = self.decoded.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(e) = d.error.take() {
                return Err(e);
            }
            self.held.append(&mut d.pictures);
        }
        self.held.sort_by_key(|(at, _)| *at);
        let dts = self.dts;
        let due = if all { self.held.len() } else { self.held.partition_point(|(at, _)| *at <= dts) };
        for (at, picture) in self.held.drain(..due) {
            self.out.push_back(Sample::Video { at, duration: None, picture });
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), MediaError> {
        // SAFETY: a live session.
        unsafe {
            VTDecompressionSessionFinishDelayedFrames(self.session);
            VTDecompressionSessionWaitForAsynchronousFrames(self.session);
        }
        self.collect(true)
    }

    fn flush(&mut self) {
        // SAFETY: a live session.
        unsafe { VTDecompressionSessionWaitForAsynchronousFrames(self.session) };
        *self.decoded.lock().unwrap_or_else(PoisonError::into_inner) = Decoded::default();
        self.held.clear();
        self.out.clear();
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        // SAFETY: invalidating stops the callback before `decoded` goes; each object made
        // here is released once.
        unsafe {
            if !self.session.is_null() {
                VTDecompressionSessionInvalidate(self.session);
                CFRelease(self.session);
            }
            if !self.format.is_null() {
                CFRelease(self.format);
            }
        }
    }
}

/// The compressed packet the converter is handed next.
struct Packet {
    data: Vec<u8>,
    desc: PacketDesc,
    channels: u32,
    /// `data` is not yet handed over.
    pending: bool,
    /// No more packets follow: the converter gives out what it holds.
    end: bool,
}

unsafe extern "C" fn input(
    _converter: *mut c_void,
    packets: *mut u32,
    data: *mut AudioBufferList,
    descs: *mut *mut PacketDesc,
    user: *mut c_void,
) -> i32 {
    // SAFETY: `user` is the `Packet` of the conversion under way (borrowed for its length);
    // the converter's pointers are valid for the call, and `data` stays put until the next.
    unsafe {
        let p = &mut *user.cast::<Packet>();
        if !p.pending {
            *packets = 0;
            return if p.end { 0 } else { NO_MORE };
        }
        p.pending = false;
        let size = u32::try_from(p.data.len()).unwrap_or(u32::MAX);
        (*data).count = 1;
        if let Some(b) = (*data).buffers.first_mut() {
            *b = AudioBuffer { channels: p.channels, size, data: p.data.as_mut_ptr().cast() };
        }
        *packets = 1;
        if !descs.is_null() {
            p.desc = PacketDesc { start: 0, frames: 0, size };
            *descs = &raw mut p.desc;
        }
        0
    }
}

/// An AudioToolbox converter from AAC or MP3 to interleaved float at the stream's rate.
struct Audio {
    converter: *mut c_void,
    channels: usize,
    rate: u32,
    packet: Packet,
    /// The time of the first sound since a start, a seek or a discontinuity, and the frames
    /// given out since.
    anchor: Option<(Duration, u64)>,
    out: VecDeque<Sample>,
}

// SAFETY: a converter may be used from any thread, one call at a time (`&mut self`).
unsafe impl Send for Audio {}

impl Audio {
    fn open(c: &AudioConfig) -> Result<Audio, MediaError> {
        if c.channels == 0 || c.sample_rate == 0 {
            return Err(MediaError::Format("the sound has no rate or no channels".into()));
        }
        let ch = u32::from(c.channels);
        let rate = f64::from(c.sample_rate);
        let (format_id, frames) = match c.codec {
            AudioCodec::Aac { .. } => (AAC, 1024),
            // MPEG-2 and 2.5 Layer III (under 32 kHz) frames are half as long.
            AudioCodec::Mp3 => (MP3, if c.sample_rate >= 32000 { 1152 } else { 576 }),
        };
        let from = Asbd {
            sample_rate: rate,
            format_id,
            format_flags: 0,
            bytes_per_packet: 0,
            frames_per_packet: frames,
            bytes_per_frame: 0,
            channels_per_frame: ch,
            bits_per_channel: 0,
            reserved: 0,
        };
        let to = Asbd {
            sample_rate: rate,
            format_id: LINEAR_PCM,
            format_flags: FLOAT_PACKED,
            bytes_per_packet: 4 * ch,
            frames_per_packet: 1,
            bytes_per_frame: 4 * ch,
            channels_per_frame: ch,
            bits_per_channel: 32,
            reserved: 0,
        };
        let mut converter = null_mut();
        // SAFETY: two formats and an out pointer, all locals.
        let r = unsafe { AudioConverterNew(&from, &to, &mut converter) };
        if r != 0 || converter.is_null() {
            return Err(MediaError::Unsupported(format!(
                "AudioToolbox will not decode {} (error {r})",
                c.codec_string
            )));
        }
        let packet = Packet {
            data: Vec::new(),
            desc: PacketDesc { start: 0, frames: 0, size: 0 },
            channels: ch,
            pending: false,
            end: false,
        };
        let a = Audio {
            converter,
            channels: usize::from(c.channels),
            rate: c.sample_rate,
            packet,
            anchor: None,
            out: VecDeque::new(),
        };
        if let AudioCodec::Aac { asc } = &c.codec {
            let cookie = esds(asc);
            let size = u32::try_from(cookie.len()).unwrap_or(u32::MAX);
            // SAFETY: the cookie is copied by the converter during the call.
            let r =
                unsafe { AudioConverterSetProperty(a.converter, MAGIC_COOKIE, size, cookie.as_ptr().cast()) };
            if r != 0 {
                return Err(MediaError::Format(format!("the AAC setup was refused (error {r})")));
            }
        }
        Ok(a)
    }

    /// The time of the next sound to come out.
    fn next(&self) -> Option<Duration> {
        self.anchor.map(|(at, frames)| {
            at + Duration::from_nanos(frames.saturating_mul(1_000_000_000) / u64::from(self.rate))
        })
    }

    fn push(&mut self, pts: Duration, data: Vec<u8>) -> Result<(), MediaError> {
        if self.next().is_none_or(|n| n.abs_diff(pts) > RESYNC) {
            self.anchor = Some((pts, 0));
        }
        self.packet.data = data;
        self.packet.pending = true;
        self.convert()
    }

    /// Takes every sound the converter gives for what it has been handed.
    fn convert(&mut self) -> Result<(), MediaError> {
        let ch = self.channels.max(1);
        loop {
            let mut pcm = vec![0f32; OUT_FRAMES * ch];
            let mut list = AudioBufferList {
                count: 1,
                buffers: [AudioBuffer {
                    channels: ch as u32,
                    size: u32::try_from(pcm.len() * 4).unwrap_or(u32::MAX),
                    data: pcm.as_mut_ptr().cast(),
                }],
            };
            let mut frames = OUT_FRAMES as u32;
            // SAFETY: the output list points into `pcm`, which holds `frames` frames; the
            // input callback gets `packet`, borrowed for the call.
            let r = unsafe {
                AudioConverterFillComplexBuffer(
                    self.converter,
                    input,
                    (&raw mut self.packet).cast(),
                    &mut frames,
                    &mut list,
                    null_mut(),
                )
            };
            if r != 0 && r != NO_MORE {
                return Err(MediaError::Decode(format!(
                    "AudioToolbox could not decode the sound (error {r})"
                )));
            }
            let n = (frames as usize).min(OUT_FRAMES);
            if n > 0 {
                let at = self.next().unwrap_or_default();
                if let Some((_, given)) = &mut self.anchor {
                    *given += n as u64;
                }
                pcm.truncate(n * ch);
                self.out.push_back(Sample::Audio { at, samples: pcm });
            }
            if r != 0 || n < OUT_FRAMES {
                return Ok(());
            }
        }
    }

    fn end(&mut self) -> Result<(), MediaError> {
        self.packet.end = true;
        let r = self.convert();
        self.reset();
        r
    }

    fn reset(&mut self) {
        // SAFETY: a live converter.
        unsafe { AudioConverterReset(self.converter) };
        self.packet.pending = false;
        self.packet.end = false;
        self.anchor = None;
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        // SAFETY: the converter made here, disposed once.
        unsafe { AudioConverterDispose(self.converter) };
    }
}

/// [`Elementary`] with VideoToolbox (H.264) and AudioToolbox (AAC, MP3).
#[derive(Default)]
pub(crate) struct ToolboxElementary {
    video: Option<Video>,
    audio: Option<Audio>,
}

impl Elementary for ToolboxElementary {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        self.video = video.map(Video::open).transpose()?;
        self.audio = audio.map(Audio::open).transpose()?;
        Ok(())
    }

    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        match unit {
            Unit::Video { pts, dts, data, .. } => self
                .video
                .as_mut()
                .ok_or_else(|| MediaError::Format("video with no video stream".into()))?
                .push(&data, pts, dts),
            Unit::Audio { pts, data } => self
                .audio
                .as_mut()
                .ok_or_else(|| MediaError::Format("sound with no sound stream".into()))?
                .push(pts, data),
        }
    }

    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        let at = |q: &VecDeque<Sample>| match q.front() {
            Some(Sample::Video { at, .. } | Sample::Audio { at, .. }) => Some(*at),
            _ => None,
        };
        let v = self.video.as_mut().map(|v| &mut v.out);
        let a = self.audio.as_mut().map(|a| &mut a.out);
        // The earlier of the two streams' next samples.
        Ok(match (v, a) {
            (Some(v), Some(a)) => match (at(v), at(a)) {
                (Some(tv), Some(ta)) if ta < tv => a.pop_front(),
                (Some(_), _) => v.pop_front(),
                _ => a.pop_front(),
            },
            (Some(q), None) | (None, Some(q)) => q.pop_front(),
            (None, None) => None,
        })
    }

    fn end(&mut self) -> Result<(), MediaError> {
        if let Some(v) = &mut self.video {
            v.end()?;
        }
        if let Some(a) = &mut self.audio {
            a.end()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        if let Some(v) = &mut self.video {
            v.flush();
        }
        if let Some(a) = &mut self.audio {
            a.reset();
            a.out.clear();
        }
        Ok(())
    }
}

/// What a fetch's completion handler brings: the HTTP status and the body, or the
/// `NSURLError` code and its description.
type Answer = Result<(isize, Vec<u8>), (isize, String)>;

/// Where a fetch's completion handler leaves its answer.
struct Slot {
    answer: Mutex<Option<Answer>>,
    came: Condvar,
}

#[repr(C)]
struct BlockDescriptor {
    reserved: usize,
    size: usize,
}

/// A block (the Blocks ABI's layout) for `dataTaskWithRequest:completionHandler:`. It has no
/// copy or dispose helpers, so the copy the session keeps is these bytes moved to the heap.
#[repr(C)]
struct Handler {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*mut Handler, *mut NSData, *mut NSURLResponse, *mut NSError),
    descriptor: *const BlockDescriptor,
    /// One strong count of the fetch's slot, taken back when the handler runs (once).
    slot: *const c_void,
}

static DESCRIPTOR: BlockDescriptor = BlockDescriptor { reserved: 0, size: size_of::<Handler>() };

unsafe extern "C" {
    static _NSConcreteStackBlock: [*const c_void; 32];
}

#[link(name = "objc")]
unsafe extern "C" {
    #[link_name = "objc_msgSend"]
    fn send_data_task(
        session: *const NSURLSession,
        sel: Sel,
        request: *const NSURLRequest,
        handler: *const Handler,
    ) -> *mut NSURLSessionDataTask;
}

unsafe extern "C" fn answered(
    block: *mut Handler,
    data: *mut NSData,
    response: *mut NSURLResponse,
    error: *mut NSError,
) {
    // SAFETY: the session calls the handler once, with its own copy of the block (whose slot
    // pointer holds one count) and with objects (or nulls) alive for the call.
    unsafe {
        let slot = Arc::from_raw((*block).slot.cast::<Slot>());
        let answer = match error.as_ref() {
            Some(e) => Err((e.code(), e.localizedDescription().to_string())),
            None => {
                let status = response
                    .as_ref()
                    .and_then(|r| r.downcast_ref::<NSHTTPURLResponse>())
                    .map_or(0, |h| h.statusCode());
                Ok((status, data.as_ref().map_or_else(Vec::new, NSData::to_vec)))
            }
        };
        *slot.answer.lock().unwrap_or_else(PoisonError::into_inner) = Some(answer);
        slot.came.notify_all();
    }
}

/// [`Fetch`] through an NSURLSession (http and https, through the system's proxy, its answers
/// never cached), waiting for each answer; `file:` URLs are read from disk.
pub(crate) struct UrlFetch {
    session: Retained<NSURLSession>,
}

// SAFETY: NSURLSession is thread-safe.
unsafe impl Send for UrlFetch {}

impl UrlFetch {
    pub(crate) fn new() -> UrlFetch {
        let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
        // A playlist fetched again must come from the server.
        config.setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
        UrlFetch { session: NSURLSession::sessionWithConfiguration(&config) }
    }

    fn fetch(&self, url: &str, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
        let ns_url = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| MediaError::NotFound(format!("{url} is not a URL")))?;
        let request = NSMutableURLRequest::requestWithURL(&ns_url);
        request.setCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
        let slot = Arc::new(Slot { answer: Mutex::new(None), came: Condvar::new() });
        let handler = Handler {
            // The runtime's stack block class.
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: 0,
            reserved: 0,
            invoke: answered,
            descriptor: &DESCRIPTOR,
            slot: Arc::into_raw(slot.clone()).cast(),
        };
        // SAFETY: `dataTaskWithRequest:completionHandler:` with a live session and request and
        // a block it copies before returning; the task comes autoreleased (a pool is around
        // each fetch) and is retained here. A task that is not made never calls the handler,
        // whose count of the slot is then left (a few bytes).
        let task = unsafe {
            let request: &NSURLRequest = &request;
            Retained::retain(send_data_task(
                Retained::as_ptr(&self.session),
                sel!(dataTaskWithRequest:completionHandler:),
                request,
                &handler,
            ))
        };
        let task = task.ok_or_else(|| MediaError::NotFound(format!("{url}: no request could be made")))?;
        task.resume();
        // No answer for first_byte(), no byte for quiet() once bytes came, or load() in all: the
        // task is cancelled and the fetch timed out. So is one whose reader is gone (`stop`).
        let started = Instant::now();
        let (mut seen, mut last) = (0i64, started);
        let mut answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
        let answer = loop {
            if let Some(a) = answer.take() {
                break a;
            }
            let got = task.countOfBytesReceived();
            if got != seen {
                (seen, last) = (got, Instant::now());
            }
            if stop.load(Ordering::Relaxed) {
                drop(answer);
                task.cancel();
                return Err(timed_out(url, "the reader went away"));
            }
            let limit = if seen == 0 { first_byte() } else { quiet() };
            if last.elapsed() > limit || started.elapsed() > load() {
                drop(answer);
                task.cancel();
                let why = if seen == 0 { "no answer, or too long in all" } else { "nothing arrived" };
                return Err(timed_out(url, why));
            }
            answer = slot
                .came
                .wait_timeout(answer, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        match answer {
            Err((URL_TIMED_OUT, why)) => Err(timed_out(url, &why)),
            Err((_, why)) => Err(MediaError::NotFound(format!("{url}: {why}"))),
            Ok((200, body)) => Ok(body),
            Ok((status, _)) => Err(MediaError::NotFound(format!("{url}: the server answered {status}"))),
        }
    }
}

impl Fetch for UrlFetch {
    fn get(&mut self, url: &str, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
        if let Some(path) = file_path(url) {
            return std::fs::read(&path).map_err(|e| MediaError::NotFound(format!("{path}: {e}")));
        }
        autoreleasepool(|_| self.fetch(url, stop))
    }
}

impl Drop for UrlFetch {
    fn drop(&mut self) {
        self.session.invalidateAndCancel();
    }
}
