//! Windows HLS: the H.264, AAC and MP3 streams the crate's own HLS reader takes apart, decoded
//! by Media Foundation transforms (H.264 on the graphics card through the same D3D11 device
//! manager the source reader uses, when a renderer takes pictures there), and playlists and
//! segments fetched through Media Foundation's own HTTP byte stream.

use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::time::Duration;

use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSample, IMFTransform, MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AAC_PAYLOAD_TYPE, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_MT_USER_DATA, MF_SA_D3D11_AWARE, MFAudioFormat_AAC, MFAudioFormat_Float,
    MFAudioFormat_MP3, MFAudioFormat_PCM, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFMediaType_Audio, MFMediaType_Video, MFT_CATEGORY_AUDIO_DECODER, MFT_CATEGORY_VIDEO_DECODER,
    MFT_ENUM_FLAG, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER,
    MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::core::{GUID, Interface};

use super::super::d3d11::Gpu;
use super::{Layout, Mf, MfDecoder, com, copy_path, format_error, from_hns, hns, video_layout};
use crate::capture::Converter;
use crate::hls::{AudioCodec, AudioConfig, Elementary, Fetch, Unit, VideoConfig};
use crate::host::{AudioFormat, MediaError, Picture, Sample};

fn decode_error(e: windows::core::Error) -> MediaError {
    MediaError::Decode(format!("{} ({:#010x})", e.message(), e.code().0))
}

/// The first synchronous decoder transform on this system that takes `subtype`.
fn transform(category: GUID, major: GUID, subtype: GUID) -> Result<IMFTransform, MediaError> {
    let input = MFT_REGISTER_TYPE_INFO { guidMajorType: major, guidSubtype: subtype };
    let flags =
        MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_LOCALMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
    // SAFETY: the activation array MFTEnumEx hands out is read for the count it gives, each
    // entry is taken (released when dropped), and the array is freed with the COM allocator.
    unsafe {
        let (mut list, mut n) = (std::ptr::null_mut(), 0u32);
        MFTEnumEx(category, flags, Some(&input), None, &mut list, &mut n).map_err(decode_error)?;
        let mut found = None;
        if !list.is_null() {
            for a in std::slice::from_raw_parts_mut(list, n as usize) {
                if let Some(a) = a.take()
                    && found.is_none()
                {
                    found = a.ActivateObject::<IMFTransform>().ok();
                }
            }
            CoTaskMemFree(Some(list.cast_const().cast()));
        }
        found.ok_or_else(|| MediaError::Unsupported(format!("no Media Foundation decoder for {subtype:?}")))
    }
}

fn media_type(major: &GUID, subtype: &GUID) -> windows::core::Result<IMFMediaType> {
    // SAFETY: a new media type filled with plain values.
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, major)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        Ok(t)
    }
}

/// Sets the first output type of `subtype` the transform offers; whether it offered one.
fn set_output(t: &IMFTransform, subtype: &GUID) -> windows::core::Result<bool> {
    // SAFETY: plain calls on a live transform.
    unsafe {
        for i in 0.. {
            let Ok(o) = t.GetOutputAvailableType(0, i) else { return Ok(false) };
            if o.GetGUID(&MF_MT_SUBTYPE)? == *subtype {
                t.SetOutputType(0, &o, 0)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Hands `data` to the transform as one sample at `pts`; `false` when it takes no input
/// until its output is collected.
fn input(t: &IMFTransform, data: &[u8], pts: Duration) -> Result<bool, MediaError> {
    let len = u32::try_from(data.len()).map_err(|_| MediaError::Format("a piece over 4 GB".into()))?;
    // SAFETY: the buffer is locked while `data` is copied into it (its length is `len`), then
    // unlocked; the sample holds it.
    unsafe {
        let buffer = MFCreateMemoryBuffer(len.max(1)).map_err(decode_error)?;
        let mut base = std::ptr::null_mut();
        buffer.Lock(&mut base, None, None).map_err(decode_error)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), base, data.len());
        let _ = buffer.Unlock();
        buffer.SetCurrentLength(len).map_err(decode_error)?;
        let sample = MFCreateSample().map_err(decode_error)?;
        sample.AddBuffer(&buffer).map_err(decode_error)?;
        sample.SetSampleTime(hns(pts)).map_err(decode_error)?;
        match t.ProcessInput(0, &sample, 0) {
            Ok(()) => Ok(true),
            Err(e) if e.code() == MF_E_NOTACCEPTING => Ok(false),
            Err(e) => Err(decode_error(e)),
        }
    }
}

/// What the transform's output gives.
enum Out {
    Sample(IMFSample),
    /// It needs more input (or, after a drain, is empty).
    More,
    /// Its output's format changed: set the output type again.
    Changed,
}

fn output(t: &IMFTransform) -> Result<Out, MediaError> {
    // SAFETY: the output buffer's sample and event collection are owned here after the call
    // (taken out of their ManuallyDrop and released when dropped), whichever way it went.
    unsafe {
        let info = t.GetOutputStreamInfo(0).map_err(decode_error)?;
        let provides = info.dwFlags
            & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32
            != 0;
        let sample = if provides {
            None
        } else {
            let s = MFCreateSample().map_err(decode_error)?;
            s.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(1)).map_err(decode_error)?)
                .map_err(decode_error)?;
            Some(s)
        };
        let mut buffer = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(sample),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        }];
        let mut status = 0u32;
        let r = t.ProcessOutput(0, &mut buffer, &mut status);
        let [b] = buffer;
        drop(ManuallyDrop::into_inner(b.pEvents));
        let sample = ManuallyDrop::into_inner(b.pSample);
        match r {
            Ok(()) => Ok(sample.map_or(Out::More, Out::Sample)),
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(Out::More),
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => Ok(Out::Changed),
            Err(e) => Err(decode_error(e)),
        }
    }
}

fn message(
    t: &IMFTransform,
    m: windows::Win32::Media::MediaFoundation::MFT_MESSAGE_TYPE,
) -> Result<(), MediaError> {
    // SAFETY: a message with no parameter to a live transform.
    unsafe { t.ProcessMessage(m, 0) }.map_err(decode_error)
}

fn time_of(s: &IMFSample) -> Duration {
    // SAFETY: reading a property of a live sample.
    from_hns(unsafe { s.GetSampleTime() }.unwrap_or(0))
}

struct Video {
    t: IMFTransform,
    /// How pictures sit in the output; `None` until the transform knows (its first frames).
    layout: Option<Layout>,
    /// Decoding on the graphics card, into its textures.
    gpu: Option<Gpu>,
    out: VecDeque<Sample>,
}

impl Video {
    fn open(config: &VideoConfig, gpu: Option<Gpu>) -> Result<Video, MediaError> {
        let t = transform(MFT_CATEGORY_VIDEO_DECODER, MFMediaType_Video, MFVideoFormat_H264)?;
        let gpu = gpu.filter(|g| {
            // SAFETY: reading the transform's attributes and handing it the device manager,
            // which it keeps a reference to.
            let r = unsafe {
                t.GetAttributes().and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE)).and_then(
                    |aware| match aware {
                        0 => Err(windows::Win32::Foundation::E_NOTIMPL.into()),
                        _ => t.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, g.manager.as_raw() as usize),
                    },
                )
            };
            r.map_err(|e| {
                copy_path(&format!("the H.264 decoder will not decode on the GPU ({})", e.message()))
            })
            .is_ok()
        });
        // Annex B in; low latency stays off, so pictures come out in presentation order.
        let input = media_type(&MFMediaType_Video, &MFVideoFormat_H264).map_err(decode_error)?;
        // SAFETY: filling a media type and setting it on a live transform.
        unsafe {
            input
                .SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(config.width) << 32) | u64::from(config.height))
                .map_err(decode_error)?;
            t.SetInputType(0, &input, 0).map_err(|e| MediaError::Format(e.message()))?;
        }
        let mut v = Video { t, layout: None, gpu, out: VecDeque::new() };
        // Before its first frames it may not offer an output yet; it says when it does.
        let _ = v.set_output();
        message(&v.t, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING)?;
        message(&v.t, MFT_MESSAGE_NOTIFY_START_OF_STREAM)?;
        Ok(v)
    }

    fn set_output(&mut self) -> Result<(), MediaError> {
        if !set_output(&self.t, &MFVideoFormat_NV12).map_err(decode_error)? {
            return Err(MediaError::Format("the H.264 decoder gives no NV12 pictures".into()));
        }
        // SAFETY: reading the type just set.
        let t = unsafe { self.t.GetOutputCurrentType(0) }.map_err(decode_error)?;
        self.layout = Some(video_layout(&t)?.0);
        Ok(())
    }

    fn picture(&mut self, sample: &IMFSample, l: Layout) -> Result<Picture, MediaError> {
        if let Some(g) = &mut self.gpu {
            match g.picture(sample, l.x, l.y, l.width, l.height, l.matrix, l.full_range) {
                Ok(p) => return Ok(p),
                Err(e) => copy_path(&e),
            }
        }
        MfDecoder::copy(sample, l)
    }

    /// Collects every picture the transform has ready.
    fn collect(&mut self) -> Result<(), MediaError> {
        loop {
            match output(&self.t)? {
                Out::More => return Ok(()),
                Out::Changed => self.set_output()?,
                Out::Sample(s) => {
                    let l = self
                        .layout
                        .ok_or_else(|| MediaError::Decode("a picture before its format".into()))?;
                    // SAFETY: reading a property of a live sample.
                    let duration = unsafe { s.GetSampleDuration() }.ok().filter(|d| *d > 0).map(from_hns);
                    let picture = self.picture(&s, l)?;
                    self.out.push_back(Sample::Video { at: time_of(&s), duration, picture });
                }
            }
        }
    }
}

struct Audio {
    t: IMFTransform,
    /// The output is 32-bit float; else 16-bit integers.
    float: bool,
    /// The format the decoder said it would give when opened: the one the player is told,
    /// which every sample is given in.
    format: AudioFormat,
    /// The format it gives now. The AAC decoder guesses SBR (twice the rate, and stereo)
    /// for a stream at 24 kHz or less until its first frame says otherwise.
    actual: AudioFormat,
    /// From `actual` to `format`, while they differ.
    convert: Option<Converter>,
    out: VecDeque<Sample>,
}

impl Audio {
    fn open(config: &AudioConfig) -> Result<Audio, MediaError> {
        let subtype = match config.codec {
            AudioCodec::Aac { .. } => MFAudioFormat_AAC,
            AudioCodec::Mp3 => MFAudioFormat_MP3,
        };
        let t = transform(MFT_CATEGORY_AUDIO_DECODER, MFMediaType_Audio, subtype)?;
        let input = media_type(&MFMediaType_Audio, &subtype).map_err(decode_error)?;
        // SAFETY: filling a media type and setting it on a live transform.
        unsafe {
            input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sample_rate).map_err(decode_error)?;
            input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(config.channels)).map_err(decode_error)?;
            if let AudioCodec::Aac { asc } = &config.codec {
                // Raw frames (payload type 0), described by HEAACWAVEINFO's tail (payload
                // type, profile 0xFE "unspecified", the rest zero) and the AudioSpecificConfig.
                let mut user = vec![0, 0, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0, 0];
                user.extend_from_slice(asc);
                input.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0).map_err(decode_error)?;
                input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(decode_error)?;
                input.SetBlob(&MF_MT_USER_DATA, &user).map_err(decode_error)?;
            }
            t.SetInputType(0, &input, 0).map_err(|e| MediaError::Format(e.message()))?;
        }
        let none = AudioFormat::new(0, 0);
        let mut a = Audio { t, float: true, format: none, actual: none, convert: None, out: VecDeque::new() };
        a.set_output()?;
        a.format = a.actual;
        a.reset();
        message(&a.t, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING)?;
        message(&a.t, MFT_MESSAGE_NOTIFY_START_OF_STREAM)?;
        Ok(a)
    }

    fn set_output(&mut self) -> Result<(), MediaError> {
        self.float = set_output(&self.t, &MFAudioFormat_Float).map_err(decode_error)?;
        if !self.float && !set_output(&self.t, &MFAudioFormat_PCM).map_err(decode_error)? {
            return Err(MediaError::Format("the sound decoder gives neither float nor PCM".into()));
        }
        // SAFETY: reading the type just set.
        let (rate, channels) = unsafe {
            let t = self.t.GetOutputCurrentType(0).map_err(decode_error)?;
            (
                t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0),
                t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0),
            )
        };
        if rate == 0 || channels == 0 {
            return Err(MediaError::Format("the sound decoder gives no rate or no channels".into()));
        }
        self.actual = AudioFormat::new(rate, u16::try_from(channels).unwrap_or(u16::MAX));
        self.reset();
        Ok(())
    }

    /// Starts converting afresh (after a format change or a seek).
    fn reset(&mut self) {
        self.convert = (self.actual != self.format).then(|| Converter::new(self.actual, self.format));
    }

    fn collect(&mut self) -> Result<(), MediaError> {
        loop {
            match output(&self.t)? {
                Out::More => return Ok(()),
                Out::Changed => self.set_output()?,
                Out::Sample(s) => {
                    // SAFETY: the buffer is locked while its bytes are read, and unlocked after.
                    let samples: Vec<f32> = unsafe {
                        let buffer = s.ConvertToContiguousBuffer().map_err(decode_error)?;
                        let (mut base, mut len) = (std::ptr::null_mut(), 0u32);
                        buffer.Lock(&mut base, None, Some(&mut len)).map_err(decode_error)?;
                        let bytes = if base.is_null() {
                            &[][..]
                        } else {
                            std::slice::from_raw_parts(base, len as usize)
                        };
                        let v = if self.float {
                            bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
                        } else {
                            bytes
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
                                .collect()
                        };
                        let _ = buffer.Unlock();
                        v
                    };
                    let samples = match &mut self.convert {
                        Some(c) => {
                            let mut out = Vec::new();
                            c.convert(&samples, &mut out);
                            out
                        }
                        None => samples,
                    };
                    if !samples.is_empty() {
                        self.out.push_back(Sample::Audio { at: time_of(&s), samples });
                    }
                }
            }
        }
    }
}

/// Pushes `data`, collecting output whenever the transform asks for it to be taken first.
fn feed(
    t: &IMFTransform,
    data: &[u8],
    pts: Duration,
    mut collect: impl FnMut() -> Result<(), MediaError>,
) -> Result<(), MediaError> {
    while !input(t, data, pts)? {
        collect()?;
    }
    collect()
}

/// [`Elementary`] with Media Foundation transforms.
pub(crate) struct MftElementary {
    gpu: Option<Gpu>,
    video: Option<Video>,
    audio: Option<Audio>,
    _mf: Mf,
}

// SAFETY: the transforms and D3D11 objects are free-threaded (multithreaded apartment); the
// core uses this from one thread at a time.
unsafe impl Send for MftElementary {}

impl MftElementary {
    /// On the GPU when a renderer takes pictures there, as [`MfDecoder`] does.
    pub(crate) fn new() -> Result<MftElementary, MediaError> {
        let gpu = match remedy_ui_paint::gpu_video_adapter() {
            Some(remedy_ui_paint::GpuAdapter::Luid(l)) => Gpu::new(l).map_err(|e| copy_path(&e)).ok(),
            _ => None,
        };
        Self::with_gpu(gpu)
    }

    fn with_gpu(gpu: Option<Gpu>) -> Result<MftElementary, MediaError> {
        Ok(MftElementary { gpu, video: None, audio: None, _mf: Mf::start()? })
    }

    fn each(&mut self, f: impl Fn(&IMFTransform) -> Result<(), MediaError>) -> Result<(), MediaError> {
        if let Some(v) = &self.video {
            f(&v.t)?;
        }
        if let Some(a) = &self.audio {
            f(&a.t)?;
        }
        Ok(())
    }
}

impl Elementary for MftElementary {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        com();
        self.video = match video {
            Some(c) => Some(match Video::open(c, self.gpu.take()) {
                Ok(v) => v,
                Err(e @ MediaError::Unsupported(_)) => return Err(e),
                // The GPU way would not take it: the CPU way.
                Err(e) => {
                    copy_path(&format!("the H.264 decoder would not open on the GPU ({e})"));
                    Video::open(c, None)?
                }
            }),
            None => None,
        };
        self.audio = audio.map(Audio::open).transpose()?;
        Ok(())
    }

    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        com();
        match unit {
            Unit::Video { pts, data, .. } => {
                let v = self
                    .video
                    .as_mut()
                    .ok_or_else(|| MediaError::Format("video with no video stream".into()))?;
                let t = v.t.clone();
                feed(&t, &data, pts, || v.collect())
            }
            Unit::Audio { pts, data } => {
                let a = self
                    .audio
                    .as_mut()
                    .ok_or_else(|| MediaError::Format("sound with no sound stream".into()))?;
                let t = a.t.clone();
                feed(&t, &data, pts, || a.collect())
            }
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
        com();
        self.each(|t| {
            message(t, MFT_MESSAGE_NOTIFY_END_OF_STREAM)?;
            message(t, MFT_MESSAGE_COMMAND_DRAIN)
        })?;
        if let Some(v) = &mut self.video {
            v.collect()?;
        }
        if let Some(a) = &mut self.audio {
            a.collect()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        com();
        self.each(|t| message(t, MFT_MESSAGE_COMMAND_FLUSH))?;
        if let Some(v) = &mut self.video {
            v.out.clear();
        }
        if let Some(a) = &mut self.audio {
            a.out.clear();
            a.reset();
        }
        Ok(())
    }

    fn audio_format(&self) -> Option<AudioFormat> {
        self.audio.as_ref().map(|a| a.format)
    }
}

/// The path a `file:` URL names (`file:///C:/a%20b/c.ts`, as [`MediaSource::to_url`] makes,
/// or with a segment name joined on unescaped); `None` for other schemes.
///
/// shortcut: a literal `%` in an unescaped segment name is read as an escape; parse the
/// playlist's names apart from the base if that ever matters.
///
/// [`MediaSource::to_url`]: crate::host::MediaSource::to_url
fn file_path(url: &str) -> Option<String> {
    let rest = url.get(..5).filter(|s| s.eq_ignore_ascii_case("file:")).and(url.get(5..))?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    // `file:///C:/…` and `file://localhost/C:/…` are drive paths; `file://server/share/…` a
    // UNC one (kept as `//server/share/…`).
    let path = rest.strip_prefix("///").or_else(|| rest.strip_prefix("//localhost/")).unwrap_or(rest);
    Some(unescape(path))
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(v) if c == b'%' => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// [`Fetch`] through Media Foundation's HTTP byte stream (http and https); `file:` URLs are
/// read from disk.
pub(crate) struct MfFetch {
    _mf: Mf,
}

impl MfFetch {
    pub(crate) fn new() -> Result<MfFetch, MediaError> {
        Ok(MfFetch { _mf: Mf::start()? })
    }
}

impl Fetch for MfFetch {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, MediaError> {
        if let Some(path) = file_path(url) {
            return std::fs::read(&path).map_err(|e| MediaError::NotFound(format!("{path}: {e}")));
        }
        com();
        // Media Foundation fails to open an unreachable host and an HTTP error status alike.
        let stream =
            super::byte_stream(url).map_err(|e| MediaError::NotFound(format!("{url}: {}", e.message())))?;
        let mut out = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            // SAFETY: a synchronous read into a buffer that outlives the call.
            unsafe { stream.Read(&mut chunk, &mut n) }.map_err(|e| format_error(&e))?;
            if n == 0 {
                return Ok(out);
            }
            out.extend_from_slice(chunk.get(..n as usize).unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hls::AudioCodec;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name))
            .unwrap_or_else(|e| unreachable!("{name}: {e}"))
    }

    /// `clip.h264` (Annex B) split into access units: each slice NAL with the parameter sets
    /// and SEI before it (the clip has one slice per picture and no B-frames).
    fn access_units() -> Vec<Vec<u8>> {
        let data = fixture("clip.h264");
        let at = |i: usize| data.get(i).copied().unwrap_or(0xFF);
        // Each NAL's start (its zero_byte included) and its header byte's position.
        let nals: Vec<(usize, usize)> = (0..data.len())
            .filter(|&i| at(i) == 0 && at(i + 1) == 0 && at(i + 2) == 1)
            .map(|i| (i - usize::from(i > 0 && at(i.saturating_sub(1)) == 0), i + 3))
            .collect();
        let (mut units, mut from) = (Vec::new(), 0);
        for (k, &(_, header)) in nals.iter().enumerate() {
            if matches!(at(header) & 0x1F, 1 | 5) {
                let end = nals.get(k + 1).map_or(data.len(), |n| n.0);
                units.extend(data.get(from..end).map(<[u8]>::to_vec));
                from = end;
            }
        }
        units
    }

    fn config() -> VideoConfig {
        VideoConfig { codec: "avc1.42c00d".into(), width: 176, height: 100, sps: Vec::new(), pps: Vec::new() }
    }

    /// Luma at the picture's middle, from CPU memory or read back from the GPU.
    fn luma(p: &Picture) -> u8 {
        let (data, [plane, ..]) = match &p.gpu {
            Some(g) => {
                let px = g.0.read_back().unwrap_or_else(|| unreachable!("a GPU picture read back"));
                (px.data, px.planes)
            }
            None => (p.data.clone(), p.planes),
        };
        let i = plane.offset + (p.height as usize / 2) * plane.stride + p.width as usize / 2;
        data.get(i).copied().unwrap_or_else(|| unreachable!("the middle of the picture"))
    }

    fn decode_video(e: &mut MftElementary) -> Vec<(Duration, Picture)> {
        let units = access_units();
        assert_eq!(units.len(), 75, "access units");
        e.configure(Some(&config()), None).unwrap_or_else(|e| unreachable!("configure: {e}"));
        let mut out = Vec::new();
        let mut drain = |e: &mut MftElementary| {
            while let Some(s) = e.pull().unwrap_or_else(|e| unreachable!("pull: {e}")) {
                if let Sample::Video { at, picture, .. } = s {
                    out.push((at, picture));
                }
            }
        };
        for (i, u) in units.into_iter().enumerate() {
            let pts = Duration::from_millis(40 * i as u64);
            e.push(Unit::Video { pts, dts: pts, keyframe: i == 0, data: u })
                .unwrap_or_else(|e| unreachable!("push: {e}"));
            drain(e);
        }
        e.end().unwrap_or_else(|e| unreachable!("end: {e}"));
        drain(e);
        out
    }

    fn check_video(pictures: &[(Duration, Picture)]) {
        assert_eq!(pictures.len(), 75);
        for (i, (at, p)) in pictures.iter().enumerate() {
            assert_eq!(*at, Duration::from_millis(40 * i as u64), "picture {i}'s time");
            assert_eq!((p.width, p.height), (176, 100));
        }
        // Red, green, blue in BT.709 video range: Y about 63, 173, 32.
        for (i, y) in [(10, 63u8), (35, 173), (60, 32)] {
            let got = pictures.get(i).map_or(0, |(_, p)| luma(p));
            assert!(got.abs_diff(y) < 8, "picture {i}'s luma {got}, not about {y}");
        }
    }

    #[test]
    fn h264_on_the_cpu() {
        let mut e = MftElementary::with_gpu(None).unwrap_or_else(|e| unreachable!("{e}"));
        let pictures = decode_video(&mut e);
        assert!(pictures.iter().all(|(_, p)| p.gpu.is_none()));
        check_video(&pictures);
    }

    #[test]
    fn h264_on_the_gpu() {
        // The first hardware adapter's LUID.
        // SAFETY: plain DXGI calls into locals.
        let luid = unsafe {
            let f: windows::Win32::Graphics::Dxgi::IDXGIFactory1 =
                windows::Win32::Graphics::Dxgi::CreateDXGIFactory1().unwrap_or_else(|e| unreachable!("{e}"));
            let d = f.EnumAdapters1(0).and_then(|a| a.GetDesc1()).unwrap_or_else(|e| unreachable!("{e}"));
            (u64::from(d.AdapterLuid.HighPart as u32) << 32) | u64::from(d.AdapterLuid.LowPart)
        };
        let gpu = match Gpu::new(luid) {
            Ok(g) => g,
            Err(why) => return eprintln!("skipped: no D3D11 video device ({why})"),
        };
        let mut e = MftElementary::with_gpu(Some(gpu)).unwrap_or_else(|e| unreachable!("{e}"));
        let pictures = decode_video(&mut e);
        assert!(pictures.iter().all(|(_, p)| p.gpu.is_some()), "pictures on the GPU");
        check_video(&pictures);
    }

    #[test]
    fn flush_then_key_frame() {
        let mut e = MftElementary::with_gpu(None).unwrap_or_else(|e| unreachable!("{e}"));
        let units = access_units();
        e.configure(Some(&config()), None).unwrap_or_else(|e| unreachable!("{e}"));
        for (i, u) in units.iter().take(30).enumerate() {
            let pts = Duration::from_millis(40 * i as u64);
            e.push(Unit::Video { pts, dts: pts, keyframe: i == 0, data: u.clone() })
                .unwrap_or_else(|e| unreachable!("{e}"));
        }
        e.flush().unwrap_or_else(|e| unreachable!("{e}"));
        assert!(e.pull().unwrap_or_else(|e| unreachable!("{e}")).is_none(), "nothing after a flush");
        // Again from the key frame, as after a seek.
        let pts = Duration::from_secs(5);
        let key = units.first().cloned().unwrap_or_default();
        e.push(Unit::Video { pts, dts: pts, keyframe: true, data: key })
            .unwrap_or_else(|e| unreachable!("{e}"));
        e.end().unwrap_or_else(|e| unreachable!("{e}"));
        let Some(Sample::Video { at, .. }) = e.pull().unwrap_or_else(|e| unreachable!("{e}")) else {
            unreachable!("a picture after the flush")
        };
        assert_eq!(at, pts);
    }

    /// Decodes `frames` (each with its time) and checks the sound's length and order.
    fn decode_audio(codec: AudioCodec, frames: Vec<(Duration, Vec<u8>)>, min: Duration) {
        let mut e = MftElementary::with_gpu(None).unwrap_or_else(|e| unreachable!("{e}"));
        let config = AudioConfig { codec, codec_string: String::new(), sample_rate: 48000, channels: 2 };
        e.configure(None, Some(&config)).unwrap_or_else(|e| unreachable!("configure: {e}"));
        let (mut samples, mut last) = (0usize, None);
        let mut drain = |e: &mut MftElementary| {
            while let Some(s) = e.pull().unwrap_or_else(|e| unreachable!("pull: {e}")) {
                let Sample::Audio { at, samples: s } = s else { unreachable!("only sound") };
                assert!(last.is_none_or(|l| at > l), "sound in order");
                last = Some(at);
                samples += s.len();
            }
        };
        for (pts, data) in frames {
            e.push(Unit::Audio { pts, data }).unwrap_or_else(|e| unreachable!("push: {e}"));
            drain(&mut e);
        }
        e.end().unwrap_or_else(|e| unreachable!("end: {e}"));
        drain(&mut e);
        let length = Duration::from_secs_f64(samples as f64 / 2.0 / 48000.0);
        assert!(length >= min && length <= Duration::from_millis(3100), "sound length {length:?}");
    }

    #[test]
    fn aac() {
        let data = fixture("clip.aac");
        let (mut frames, mut rest) = (Vec::new(), data.as_slice());
        // ADTS: the frame's length in 13 bits from the fourth byte; 9 header bytes with a CRC.
        while let [_, b1, _, b3, b4, b5, ..] = *rest {
            let len = (usize::from(b3 & 3) << 11) | (usize::from(b4) << 3) | usize::from(b5 >> 5);
            let header = if b1 & 1 == 0 { 9 } else { 7 };
            let pts = Duration::from_secs_f64(frames.len() as f64 * 1024.0 / 48000.0);
            let Some(frame) = rest.get(header..len) else { break };
            frames.push((pts, frame.to_vec()));
            rest = rest.get(len..).unwrap_or_default();
        }
        assert!(frames.len() > 140, "ADTS frames: {}", frames.len());
        // AAC-LC, 48 kHz (index 3), stereo.
        decode_audio(AudioCodec::Aac { asc: vec![0x11, 0x90] }, frames, Duration::from_millis(2900));
    }

    #[test]
    fn mp3() {
        // 64 kb/s at 48 kHz with no padding: 192-byte frames of 1152 samples.
        let frames: Vec<_> = fixture("clip.mp3")
            .chunks(192)
            .enumerate()
            .map(|(i, f)| (Duration::from_secs_f64(i as f64 * 1152.0 / 48000.0), f.to_vec()))
            .collect();
        decode_audio(AudioCodec::Mp3, frames, Duration::from_millis(2800));
    }

    #[test]
    fn fetch_over_http() {
        use std::io::{BufRead, BufReader, Write};
        // Answers `/n` with `n` bytes of `n % 256` and a different body for every request to
        // `/grows` (a playlist fetched again must not come from a cache); anything else is 404.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        std::thread::spawn(move || {
            for (k, s) in listener.incoming().flatten().enumerate() {
                let mut line = String::new();
                let mut r = BufReader::new(&s);
                let _ = r.read_line(&mut line);
                let path = line.split(' ').nth(1).unwrap_or_default().trim_start_matches('/').to_string();
                while r.read_line(&mut line).is_ok_and(|n| n > 2) {}
                let body = match path.parse::<usize>() {
                    Ok(n) => Some(vec![(n % 256) as u8; n]),
                    Err(_) if path == "grows" => Some(format!("request {k}").into_bytes()),
                    Err(_) => None,
                };
                let mut s = &s;
                let _ = match body {
                    Some(b) => write!(
                        s,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        b.len()
                    )
                    .and_then(|()| s.write_all(&b)),
                    None => {
                        write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    }
                };
            }
        });
        let mut f = MfFetch::new().unwrap_or_else(|e| unreachable!("{e}"));
        let url = |p: &str| format!("http://127.0.0.1:{port}/{p}");
        let big = f.get(&url("200000")).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(big.len() == 200_000 && big.iter().all(|b| *b == (200_000 % 256) as u8));
        let a = f.get(&url("grows")).unwrap_or_else(|e| unreachable!("{e}"));
        let b = f.get(&url("grows")).unwrap_or_else(|e| unreachable!("{e}"));
        assert_ne!(a, b, "fetched again, not cached");
        let r = f.get(&url("missing.ts"));
        assert!(matches!(r, Err(MediaError::NotFound(_))), "{r:?}");
    }

    #[test]
    fn fetch_missing() {
        let mut f = MfFetch::new().unwrap_or_else(|e| unreachable!("{e}"));
        let r = f.get("http://127.0.0.1:9/nothing.m3u8");
        assert!(matches!(r, Err(MediaError::NotFound(_))), "{r:?}");
    }
}
