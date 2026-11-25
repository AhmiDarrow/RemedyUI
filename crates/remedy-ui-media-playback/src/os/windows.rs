//! Windows: Media Foundation's source reader decodes (files and network streams, with
//! whatever decoders the system has, hardware ones included), and WASAPI plays the sound in
//! shared mode, converting the stream's rate and channels itself.

use std::sync::Arc;
use std::time::Duration;

use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDCLNT_STREAMOPTIONS_RAW, AudioCategory_Other,
    AudioClientProperties, IAudioCaptureClient, IAudioClient, IAudioClient2, IAudioClock, IAudioRenderClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
    eCapture, eConsole, eRender,
};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFAttributes, IMFByteStream, IMFByteStreamCacheControl2, IMFMediaType, IMFSample,
    IMFSourceReader, MF_BYTE_STREAM_CACHE_RANGE, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_YUV_MATRIX,
    MF_OBJECT_TYPE, MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_RESOLUTION_BYTESTREAM,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ANY_STREAM, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_MEDIASOURCE,
    MF_SOURCE_READER_MEDIASOURCE_CHARACTERISTICS, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_VERSION, MFAudioFormat_Float,
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromByteStream, MFCreateSourceReaderFromURL,
    MFCreateSourceResolver, MFMEDIASOURCE_CAN_SEEK, MFMediaType_Audio, MFMediaType_Video,
    MFNominalRange_0_255, MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoArea, MFVideoFormat_NV12,
    MFVideoTransferMatrix_BT601, MFVideoTransferMatrix_BT709,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToUInt32, PropVariantToUInt64,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoIncrementMTAUsage, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, HSTRING, Interface};

// HLS: Media Foundation transforms decode the streams the crate's own reader takes apart.
#[path = "windows_hls.rs"]
mod hls;

use crate::host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture,
    PixelFormat, Plane, Sample, VideoInfo, YuvMatrix,
};

/// Sound the device buffer holds: enough to ride out a busy machine's scheduling hiccups
/// (the player refills it four times per buffer). Seeking and pausing drop or hold it, so it
/// adds no delay to the controls.
const DEVICE_BUFFER: Duration = Duration::from_millis(200);

/// 100-nanosecond units, Media Foundation's and WASAPI's time.
fn hns(d: Duration) -> i64 {
    i64::try_from(d.as_nanos() / 100).unwrap_or(i64::MAX)
}

fn from_hns(t: i64) -> Duration {
    Duration::from_nanos(u64::try_from(t.max(0)).unwrap_or(0).saturating_mul(100))
}

/// Joins this thread to the multithreaded COM apartment once (the player's threads are its
/// own; a thread already in a single-threaded apartment keeps it, which Media Foundation's
/// free-threaded objects also allow), and keeps that apartment for the whole process.
fn com() {
    static MTA: std::sync::Once = std::sync::Once::new();
    // When the last thread in the apartment ends, COM tears it down and unloads the WinRT
    // DLLs it loaded (twinapi.appcore.dll), while Media Foundation's HTTP stack
    // (Windows.Networking.Connectivity.dll) keeps pointers into them: the next stream opened
    // over HTTP crashed reading the unloaded DLL.
    // SAFETY: a plain COM call; its cookie is never released, so the apartment lives on.
    MTA.call_once(|| {
        let _ = unsafe { CoIncrementMTAUsage() };
    });
    thread_local! {
        static JOINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    JOINED.with(|j| {
        if !j.replace(true) {
            // SAFETY: plain COM initialization for the calling thread; it is never undone,
            // so no COM object outlives its apartment.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        }
    });
}

/// Media Foundation, kept started while decoders live.
struct Mf(super::Live);

impl Mf {
    fn start() -> Result<Mf, MediaError> {
        com();
        // SAFETY: MFStartup counts calls; each successful one is matched by `Drop`.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|e| MediaError::Missing(crate::deps::media_foundation(&e.to_string())))?;
        Ok(Mf(super::Live::new()))
    }
}

impl Drop for Mf {
    fn drop(&mut self) {
        // SAFETY: matches the MFStartup in `start`.
        let _ = unsafe { MFShutdown() };
    }
}

/// The threads this process runs, from a ToolHelp snapshot: each one's name (its thread
/// description; Rust sets it from the thread's name) and the module its start address is in.
pub(super) fn process_threads() -> Vec<(String, String)> {
    use windows::Wdk::System::Threading::{NtQueryInformationThread, ThreadQuerySetWin32StartAddress};
    use windows::Win32::Foundation::{CloseHandle, HLOCAL, LocalFree};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };
    use windows::Win32::System::Threading::{GetThreadDescription, OpenThread, THREAD_QUERY_INFORMATION};
    use windows::core::PCWSTR;
    let me = std::process::id();
    let mut out = Vec::new();
    // SAFETY: a snapshot and thread handles made, read and closed here; every out-pointer is
    // a local the size the call asks for, and the description is freed as documented.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else { return out };
        let mut e = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
        let mut ok = Thread32First(snap, &mut e).is_ok();
        while ok {
            if e.th32OwnerProcessID == me {
                let (mut name, mut module) = (String::new(), String::new());
                if let Ok(h) = OpenThread(THREAD_QUERY_INFORMATION, false, e.th32ThreadID) {
                    if let Ok(d) = GetThreadDescription(h) {
                        name = d.to_string().unwrap_or_default();
                        let _ = LocalFree(Some(HLOCAL(d.0.cast())));
                    }
                    let mut start = 0usize;
                    let got = NtQueryInformationThread(
                        h,
                        ThreadQuerySetWin32StartAddress,
                        (&raw mut start).cast(),
                        size_of::<usize>() as u32,
                        std::ptr::null_mut(),
                    );
                    let mut m = windows::Win32::Foundation::HMODULE::default();
                    let flags =
                        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
                    if got.is_ok() && GetModuleHandleExW(flags, PCWSTR(start as *const u16), &mut m).is_ok() {
                        let mut path = [0u16; 260];
                        let n = GetModuleFileNameW(Some(m), &mut path) as usize;
                        let path = String::from_utf16_lossy(path.get(..n).unwrap_or_default());
                        module = path.rsplit('\\').next().unwrap_or_default().to_owned();
                    }
                    let _ = CloseHandle(h);
                }
                out.push((name, module));
            }
            ok = Thread32Next(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}
struct WindowsHost {
    /// The last HLS stream's decoders, for the next.
    spare: crate::hls::Spare,
}

pub(super) fn host() -> Result<Arc<dyn MediaHost>, MediaError> {
    // Starting it once proves Media Foundation is there (it is absent on "N" editions
    // without the Media Feature Pack).
    drop(Mf::start()?);
    Ok(Arc::new(WindowsHost { spare: crate::hls::Spare::default() }))
}

impl MediaHost for WindowsHost {
    fn name(&self) -> &str {
        "Media Foundation"
    }

    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        // Media Foundation's source reader refuses playlists: the crate's own reader takes HLS
        // apart and Media Foundation's transforms decode its streams.
        if crate::hls::is_hls(source) {
            return crate::hls::open(&source.to_url(), Box::new(hls::HttpFetch::new()?), &self.spare, || {
                Ok(Box::new(hls::MftElementary::new()?))
            });
        }
        Ok(Box::new(MfDecoder::open(source)?))
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(WasapiInput::open(format, true)?))
    }

    fn audio_input_with(&self, format: AudioFormat, raw: bool) -> Result<Box<dyn AudioInput>, MediaError> {
        Ok(Box::new(WasapiInput::open(format, raw)?))
    }

    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(Wasapi::open(format, crate::host::Latency::Playback)?))
    }

    fn audio_output_with(
        &self,
        format: AudioFormat,
        latency: crate::host::Latency,
    ) -> Result<Box<dyn AudioOutput>, MediaError> {
        Ok(Box::new(Wasapi::open(format, latency)?))
    }
}

fn format_error(e: &windows::core::Error) -> MediaError {
    // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, INET_E_RESOURCE_NOT_FOUND,
    // MF_E_NET_* "could not connect".
    match e.code().0 as u32 {
        0x8007_0002 | 0x8007_0003 | 0x800C_0005 | 0xC00D_4271 => MediaError::NotFound(e.message()),
        // MF_E_UNSUPPORTED_BYTESTREAM_TYPE, MF_E_UNSUPPORTED_FORMAT, MF_E_INVALID_FILE_FORMAT
        0xC00D_36C4 | 0xC00D_36B4 | 0xC00D_3E8C => MediaError::Format(e.message()),
        _ => MediaError::Format(format!("{} ({:#010x})", e.message(), e.code().0)),
    }
}

/// A URL that resolves to a byte stream (HTTP and HTTPS through Media Foundation's own network
/// stack); other schemes (RTSP) fail.
fn byte_stream(url: &str) -> windows::core::Result<IMFByteStream> {
    // SAFETY: resolving a URL into locals that outlive the call.
    unsafe {
        let mut kind = MF_OBJECT_TYPE::default();
        let mut object = None;
        MFCreateSourceResolver()?.CreateObjectFromURL(
            &HSTRING::from(url),
            MF_RESOLUTION_BYTESTREAM.0 as u32,
            None::<&windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore>,
            &mut kind,
            &mut object,
        )?;
        object.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER))?.cast()
    }
}

/// How a decoded picture sits in its buffer.
#[derive(Clone, Copy)]
struct Layout {
    /// The buffer's rows (coded height, often rounded up to 16).
    coded_height: u32,
    /// What shows: the display aperture inside the coded picture.
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    matrix: YuvMatrix,
    full_range: bool,
    /// The stride the media type states, for buffers that are not 2D.
    stride: Option<u32>,
}

/// Says once, in the log, why pictures go through CPU memory although a renderer takes them on
/// the GPU: a driver or format that cannot share. Playback is the same, only slower.
fn copy_path(why: &str) {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!("remedy-ui-media-playback: video pictures are copied through CPU memory: {why}");
    }
}

struct MfDecoder {
    reader: IMFSourceReader,
    /// A URL's byte stream, made here so its download cache can be asked (the MPEG-4 source
    /// does not hand out the one it reads from). `None` for files and non-byte-stream URLs.
    stream: Option<IMFByteStream>,
    /// Hardware decoding into textures shared with the renderer, when one takes them.
    gpu: Option<super::d3d11::Gpu>,
    info: MediaInfo,
    video: Option<(u32, Layout)>,
    audio: Option<u32>,
    video_done: bool,
    audio_done: bool,
    _mf: Mf,
}

// SAFETY: the source reader is free-threaded (Media Foundation objects live in the
// multithreaded apartment); the player uses a decoder from one thread at a time.
unsafe impl Send for MfDecoder {}

fn u64_attr(a: &IMFAttributes, key: &GUID) -> Option<u64> {
    // SAFETY: reading a typed attribute from a live attribute store.
    unsafe { a.GetUINT64(key) }.ok()
}

fn u32_attr(a: &IMFAttributes, key: &GUID) -> Option<u32> {
    // SAFETY: as above.
    unsafe { a.GetUINT32(key) }.ok()
}

fn video_layout(t: &IMFMediaType) -> Result<(Layout, Option<f64>), MediaError> {
    let size = u64_attr(t, &MF_MT_FRAME_SIZE)
        .ok_or_else(|| MediaError::Format("the video has no frame size".into()))?;
    let (cw, ch) = ((size >> 32) as u32, size as u32);
    let mut l = Layout {
        coded_height: ch,
        x: 0,
        y: 0,
        width: cw,
        height: ch,
        matrix: if ch < 720 { YuvMatrix::Bt601 } else { YuvMatrix::Bt709 },
        full_range: false,
        stride: u32_attr(t, &MF_MT_DEFAULT_STRIDE).map(|s| s as i32).filter(|s| *s > 0).map(|s| s as u32),
    };
    let mut area = [0u8; std::mem::size_of::<MFVideoArea>()];
    // SAFETY: the aperture is a blob the size of MFVideoArea.
    if unsafe { t.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut area, None) }.is_ok() {
        // MFVideoArea: OffsetX {fract u16, value i16}, OffsetY {fract, value}, SIZE {cx i32, cy i32}.
        let i16_at = |i: usize| {
            area.get(i..i + 2).and_then(|b| b.try_into().ok()).map(i16::from_le_bytes).unwrap_or(0)
        };
        let i32_at = |i: usize| {
            area.get(i..i + 4).and_then(|b| b.try_into().ok()).map(i32::from_le_bytes).unwrap_or(0)
        };
        let (ox, oy, w, h) = (i16_at(2), i16_at(6), i32_at(8), i32_at(12));
        if ox >= 0 && oy >= 0 && w > 0 && h > 0 && ox as u32 + w as u32 <= cw && oy as u32 + h as u32 <= ch {
            l.x = ox as u32 & !1;
            l.y = oy as u32 & !1;
            l.width = w as u32;
            l.height = h as u32;
        }
    }
    match u32_attr(t, &MF_MT_YUV_MATRIX) {
        Some(m) if m == MFVideoTransferMatrix_BT709.0 as u32 => l.matrix = YuvMatrix::Bt709,
        Some(m) if m == MFVideoTransferMatrix_BT601.0 as u32 => l.matrix = YuvMatrix::Bt601,
        _ => {}
    }
    l.full_range = u32_attr(t, &MF_MT_VIDEO_NOMINAL_RANGE) == Some(MFNominalRange_0_255.0 as u32);
    let fps = u64_attr(t, &MF_MT_FRAME_RATE)
        .map(|r| ((r >> 32) as u32, r as u32))
        .filter(|(n, d)| *n > 0 && *d > 0)
        .map(|(n, d)| f64::from(n) / f64::from(d));
    Ok((l, fps))
}

impl MfDecoder {
    fn open(source: &MediaSource) -> Result<MfDecoder, MediaError> {
        let mf = Mf::start()?;
        let url = match source {
            MediaSource::File(p) => {
                if !p.exists() {
                    return Err(MediaError::NotFound(p.display().to_string()));
                }
                std::path::absolute(p).unwrap_or_else(|_| p.clone()).to_string_lossy().into_owned()
            }
            MediaSource::Url(u) => u.clone(),
            MediaSource::Bytes(_) => {
                return Err(MediaError::Unsupported(
                    "bytes in memory are opened through `open_source`".into(),
                ));
            }
        };
        // Pictures stay on the GPU when a renderer takes them there.
        let mut gpu = match remedy_ui_engine::paint::gpu_video_adapter() {
            Some(remedy_ui_engine::paint::GpuAdapter::Luid(l)) => {
                super::d3d11::Gpu::new(l).map_err(|e| copy_path(&e)).ok()
            }
            _ => None,
        };
        let make = |gpu: Option<&super::d3d11::Gpu>| -> Result<(IMFSourceReader, Option<IMFByteStream>), MediaError> {
            // SAFETY: Media Foundation calls on objects made here; every pointer passed lives
            // across the call.
            unsafe {
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 3).map_err(|e| MediaError::Unsupported(e.message()))?;
                let attrs = attrs.ok_or_else(|| MediaError::Unsupported("no attribute store".into()))?;
                // Color conversion and deinterlacing to NV12, whatever the decoder gives.
                attrs
                    .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
                    .map_err(|e| MediaError::Unsupported(e.message()))?;
                if let Some(g) = gpu {
                    // Decoding (and converting) on the graphics card, into its textures.
                    attrs
                        .SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &g.manager)
                        .map_err(|e| MediaError::Unsupported(e.message()))?;
                    attrs
                        .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                        .map_err(|e| MediaError::Unsupported(e.message()))?;
                }
                if matches!(source, MediaSource::Url(_)) {
                    // A URL that resolves to a byte stream (HTTP): read through one kept here.
                    // Other schemes (RTSP) fail to and go the URL way.
                    if let Ok(stream) = byte_stream(&url) {
                        let reader = MFCreateSourceReaderFromByteStream(&stream, &attrs)
                            .map_err(|e| format_error(&e))?;
                        return Ok((reader, Some(stream)));
                    }
                }
                MFCreateSourceReaderFromURL(&HSTRING::from(url.as_str()), &attrs)
                    .map(|r| (r, None))
                    .map_err(|e| format_error(&e))
            }
        };
        let (reader, stream) = match make(gpu.as_ref()) {
            Err(e) if gpu.is_some() => {
                copy_path(&format!("the source reader would not open on the GPU ({e})"));
                gpu = None;
                make(None)?
            }
            r => r?,
        };
        // SAFETY: Media Foundation calls on objects this function owns; every pointer passed
        // lives across the call.
        unsafe {
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                .map_err(|e| MediaError::Format(e.message()))?;

            // The first video and the first audio stream.
            let (mut video_index, mut audio_index) = (None, None);
            for i in 0u32.. {
                let Ok(t) = reader.GetNativeMediaType(i, 0) else { break };
                let Ok(major) = t.GetGUID(&MF_MT_MAJOR_TYPE) else { continue };
                if major == MFMediaType_Video && video_index.is_none() {
                    video_index = Some(i);
                } else if major == MFMediaType_Audio && audio_index.is_none() {
                    audio_index = Some(i);
                }
            }
            let mut info = MediaInfo::default();
            let mut video = None;
            if let Some(i) = video_index {
                let t = MFCreateMediaType().map_err(|e| MediaError::Unsupported(e.message()))?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .map_err(|e| MediaError::Format(e.message()))?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                    .map_err(|e| MediaError::Format(e.message()))?;
                if reader.SetCurrentMediaType(i, None, &t).is_ok()
                    && reader.SetStreamSelection(i, true).is_ok()
                {
                    let cur = reader.GetCurrentMediaType(i).map_err(|e| MediaError::Format(e.message()))?;
                    let (layout, fps) = video_layout(&cur)?;
                    info.video =
                        Some(VideoInfo { width: layout.width, height: layout.height, frame_rate: fps });
                    video = Some((i, layout));
                }
            }
            let mut audio = None;
            if let Some(i) = audio_index {
                let t = MFCreateMediaType().map_err(|e| MediaError::Unsupported(e.message()))?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(|e| MediaError::Format(e.message()))?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
                    .map_err(|e| MediaError::Format(e.message()))?;
                if reader.SetCurrentMediaType(i, None, &t).is_ok()
                    && reader.SetStreamSelection(i, true).is_ok()
                {
                    let cur = reader.GetCurrentMediaType(i).map_err(|e| MediaError::Format(e.message()))?;
                    let rate = cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0);
                    let channels = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0);
                    if rate > 0 && channels > 0 {
                        info.audio = Some(AudioFormat {
                            sample_rate: rate,
                            channels: u16::try_from(channels).unwrap_or(u16::MAX),
                        });
                        audio = Some(i);
                    }
                }
            }
            if video.is_none() && audio.is_none() {
                return Err(MediaError::Format(
                    "no sound or video stream this system can decode (a codec may be missing)".into(),
                ));
            }
            let source_id = MF_SOURCE_READER_MEDIASOURCE.0 as u32;
            if let Ok(mut pv) = reader.GetPresentationAttribute(source_id, &MF_PD_DURATION) {
                info.duration = PropVariantToUInt64(&pv).ok().filter(|d| *d > 0).map(|d| from_hns(d as i64));
                let _ = PropVariantClear(&mut pv);
            }
            if let Ok(mut pv) =
                reader.GetPresentationAttribute(source_id, &MF_SOURCE_READER_MEDIASOURCE_CHARACTERISTICS)
            {
                let c = PropVariantToUInt32(&pv).unwrap_or(0);
                info.seekable = c & MFMEDIASOURCE_CAN_SEEK.0 as u32 != 0;
                let _ = PropVariantClear(&mut pv);
            }
            Ok(MfDecoder {
                reader,
                stream,
                gpu,
                info,
                video,
                audio,
                video_done: video_index.is_none(),
                audio_done: audio.is_none(),
                _mf: mf,
            })
        }
    }

    /// The decoded picture, on the GPU where it can stay there, else copied into CPU memory.
    fn picture(&mut self, sample: &IMFSample, l: Layout) -> Result<Picture, MediaError> {
        if let Some(g) = &mut self.gpu {
            match g.picture(sample, l.x, l.y, l.width, l.height, l.matrix, l.full_range) {
                Ok(p) => return Ok(p),
                Err(e) => copy_path(&e),
            }
        }
        Self::copy(sample, l)
    }

    /// The picture's bytes copied out of the sample's locked buffer.
    fn copy(sample: &IMFSample, l: Layout) -> Result<Picture, MediaError> {
        // SAFETY: the buffer is locked while its bytes are copied, and unlocked after.
        unsafe {
            let buffer = sample.GetBufferByIndex(0).map_err(|e| MediaError::Decode(e.message()))?;
            let rows = l.coded_height as usize;
            let copy = |base: *const u8, pitch: usize, len: usize| -> Result<Picture, MediaError> {
                if pitch < l.width as usize {
                    return Err(MediaError::Decode("a picture's rows are narrower than it".into()));
                }
                let need = pitch * rows + pitch * rows.div_ceil(2);
                if base.is_null() || len < need {
                    return Err(MediaError::Decode("a picture's buffer is shorter than its size".into()));
                }
                let bytes = std::slice::from_raw_parts(base, need);
                let (x, y) = (l.x as usize, l.y as usize);
                Ok(Picture {
                    width: l.width,
                    height: l.height,
                    format: PixelFormat::Nv12,
                    planes: [
                        Plane { offset: y * pitch + x, stride: pitch },
                        Plane { offset: pitch * rows + (y / 2) * pitch + x, stride: pitch },
                        Plane::default(),
                    ],
                    matrix: l.matrix,
                    full_range: l.full_range,
                    data: Arc::from(bytes),
                    gpu: None,
                })
            };
            if let Ok(b2) = buffer.cast::<IMF2DBuffer>() {
                let mut base = std::ptr::null_mut();
                let mut pitch = 0i32;
                b2.Lock2D(&mut base, &mut pitch).map_err(|e| MediaError::Decode(e.message()))?;
                let len = b2.GetContiguousLength().unwrap_or(0) as usize;
                let pitch_ok = usize::try_from(pitch).ok().filter(|p| *p > 0);
                let r = match pitch_ok {
                    Some(p) => copy(base, p, len.max(p * rows * 3 / 2)),
                    None => Err(MediaError::Decode("bottom-up NV12 pictures are not a valid layout".into())),
                };
                let _ = b2.Unlock2D();
                return r;
            }
            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| MediaError::Decode(e.message()))?;
            let mut base = std::ptr::null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut base, None, Some(&mut len)).map_err(|e| MediaError::Decode(e.message()))?;
            let pitch = l.stride.unwrap_or(l.width) as usize;
            let r = copy(base, pitch, len as usize);
            let _ = buffer.Unlock();
            r
        }
    }

    fn sound(&self, sample: &IMFSample) -> Result<Vec<f32>, MediaError> {
        // SAFETY: the buffer is locked while its bytes are read, and unlocked after.
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| MediaError::Decode(e.message()))?;
            let mut base = std::ptr::null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut base, None, Some(&mut len)).map_err(|e| MediaError::Decode(e.message()))?;
            let out = if base.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(base, len as usize)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect()
            };
            let _ = buffer.Unlock();
            Ok(out)
        }
    }
}

impl Decoder for MfDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        com();
        loop {
            if self.video_done && self.audio_done {
                return Ok(None);
            }
            let mut index = 0u32;
            let mut flags = 0u32;
            let mut at = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: a synchronous read into locals that outlive the call.
            unsafe {
                self.reader.ReadSample(
                    MF_SOURCE_READER_ANY_STREAM.0 as u32,
                    0,
                    Some(&mut index),
                    Some(&mut flags),
                    Some(&mut at),
                    Some(&mut sample),
                )
            }
            .map_err(|e| MediaError::Decode(e.message()))?;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                return Err(MediaError::Decode("the decoder reported an error".into()));
            }
            let is_video = self.video.is_some_and(|(i, _)| i == index);
            let is_audio = self.audio == Some(index);
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 && is_video {
                // SAFETY: reading the new type of a selected stream.
                let t = unsafe { self.reader.GetCurrentMediaType(index) }
                    .map_err(|e| MediaError::Decode(e.message()))?;
                let (layout, _) = video_layout(&t)?;
                self.video = Some((index, layout));
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                if is_video {
                    self.video_done = true;
                }
                if is_audio {
                    self.audio_done = true;
                }
            }
            let Some(sample) = sample else { continue };
            let at = from_hns(at);
            if is_video && let Some((_, layout)) = self.video {
                // SAFETY: reading a property of a live sample.
                let duration = unsafe { sample.GetSampleDuration() }.ok().filter(|d| *d > 0).map(from_hns);
                let picture = self.picture(&sample, layout)?;
                return Ok(Some(Sample::Video { at, duration, picture }));
            }
            if is_audio {
                let samples = self.sound(&sample)?;
                if !samples.is_empty() {
                    return Ok(Some(Sample::Audio { at, samples }));
                }
            }
        }
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        com();
        let mut pv = PROPVARIANT::default();
        // SAFETY: filling the VT_I8 member of a zeroed PROPVARIANT, then passing it to a
        // synchronous call that only reads it.
        unsafe {
            (*pv.Anonymous.Anonymous).vt = VT_I8;
            (*pv.Anonymous.Anonymous).Anonymous.hVal = hns(to);
            self.reader
                .SetCurrentPosition(&GUID::zeroed(), &pv)
                .map_err(|e| MediaError::Decode(e.message()))?;
        }
        self.video_done = self.video.is_none();
        self.audio_done = self.audio.is_none();
        Ok(())
    }

    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        // What the URL's byte stream has in its cache: a network byte stream (HTTP) says
        // which byte ranges it has downloaded. They become times in proportion to the
        // stream's length (Media Foundation does not map bytes to times), so they are an
        // estimate of where each stretch lies. A file has no kept byte stream: `None`.
        com();
        let length = self.info.duration?;
        let stream = self.stream.as_ref()?;
        // SAFETY: the kept byte stream asked for its cache; the cache's range list is read
        // for the count it gives and freed with the COM allocator it came from.
        unsafe {
            let cache: IMFByteStreamCacheControl2 = stream.cast().ok()?;
            let bytes = stream.GetLength().ok().filter(|l| *l > 0 && *l != u64::MAX)?;
            let (mut n, mut list) = (0u32, std::ptr::null_mut::<MF_BYTE_STREAM_CACHE_RANGE>());
            cache.GetByteRanges(&mut n, &mut list).ok()?;
            let ranges = if list.is_null() {
                Vec::new()
            } else {
                let at = |b: u64| length.mul_f64(b.min(bytes) as f64 / bytes as f64);
                let ranges = std::slice::from_raw_parts(list, n as usize)
                    .iter()
                    .map(|r| at(r.qwStartOffset)..at(r.qwEndOffset))
                    .collect();
                CoTaskMemFree(Some(list.cast_const().cast()));
                ranges
            };
            Some(ranges)
        }
    }
}

/// WASAPI in shared mode, writing into the device buffer as room appears.
struct Wasapi {
    client: IAudioClient,
    render: IAudioRenderClient,
    clock: IAudioClock,
    frequency: u64,
    frames: u32,
    format: AudioFormat,
    started: bool,
    /// The system's timers were asked to tick every millisecond while this output lives
    /// (an interactive output is refilled every few), to be given back when it goes.
    fine_timer: bool,
}

// SAFETY: the audio client and its services live in the multithreaded apartment and are
// used from one thread at a time (the player's presenting thread).
unsafe impl Send for Wasapi {}

/// The usual speaker positions for a channel count (front left and right first).
fn channel_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x33,
        5 => 0x37,
        6 => 0x3F,
        7 => 0x13F,
        8 => 0x63F,
        _ => 0,
    }
}

fn device_error(e: windows::core::Error) -> MediaError {
    MediaError::Device(format!("{} ({:#010x})", e.message(), e.code().0))
}

impl Wasapi {
    fn open(format: AudioFormat, latency: crate::host::Latency) -> Result<Wasapi, MediaError> {
        com();
        let interactive = latency == crate::host::Latency::Interactive;
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        let wave = float_wave(format);
        // SAFETY: COM calls on objects created here; the format outlives Initialize.
        unsafe {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(device_error)?;
            let device = devices
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| MediaError::Device(format!("no sound output device: {}", e.message())))?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(device_error)?;
            let mut period = 0i64;
            client.GetDevicePeriod(Some(&mut period), None).map_err(device_error)?;
            // Interactive: two device periods (about 20 ms), the least a refill every few
            // milliseconds keeps from running dry.
            let buffer = if interactive {
                period.saturating_mul(2).max(hns(Duration::from_millis(6)))
            } else {
                hns(DEVICE_BUFFER).max(period.saturating_mul(4))
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    buffer,
                    0,
                    &wave.Format,
                    None,
                )
                .map_err(device_error)?;
            let frames = client.GetBufferSize().map_err(device_error)?;
            let render: IAudioRenderClient = client.GetService().map_err(device_error)?;
            let clock: IAudioClock = client.GetService().map_err(device_error)?;
            let frequency = clock.GetFrequency().map_err(device_error)?;
            if frequency == 0 {
                return Err(MediaError::Device("the device clock reports no frequency".into()));
            }
            // Sleeps are as coarse as the system's timer (15.6 ms unless asked): finer than
            // the buffer for as long as it has to be kept fed.
            let fine_timer = interactive && windows::Win32::Media::timeBeginPeriod(1) == 0;
            Ok(Wasapi { client, render, clock, frequency, frames, format, started: false, fine_timer })
        }
    }
}

/// 32-bit float samples in `format`, as WASAPI describes them.
fn float_wave(format: AudioFormat) -> WAVEFORMATEXTENSIBLE {
    let block = format.channels.saturating_mul(4);
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xFFFE, // WAVE_FORMAT_EXTENSIBLE
            nChannels: format.channels,
            nSamplesPerSec: format.sample_rate,
            nAvgBytesPerSec: format.sample_rate.saturating_mul(u32::from(block)),
            nBlockAlign: block,
            wBitsPerSample: 32,
            cbSize: (std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>())
                as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: 32 },
        dwChannelMask: channel_mask(format.channels),
        // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT is the same GUID as Media Foundation's float.
        SubFormat: MFAudioFormat_Float,
    }
}

impl AudioOutput for Wasapi {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn buffer(&self) -> Duration {
        self.format.duration_of(u64::from(self.frames))
    }

    fn room(&mut self) -> Result<usize, MediaError> {
        // SAFETY: a query on a live client.
        let padding = unsafe { self.client.GetCurrentPadding() }.map_err(device_error)?;
        Ok(self.frames.saturating_sub(padding) as usize)
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let ch = usize::from(self.format.channels);
        let n = samples.len() / ch;
        if n == 0 {
            return Ok(());
        }
        let n32 = u32::try_from(n).map_err(|_| MediaError::Device("too many frames at once".into()))?;
        // SAFETY: GetBuffer hands out room for `n` frames (n * ch floats); exactly that many
        // are copied in before it is released.
        unsafe {
            let dst = self.render.GetBuffer(n32).map_err(device_error)?;
            std::ptr::copy_nonoverlapping(samples.as_ptr(), dst.cast::<f32>(), n * ch);
            self.render.ReleaseBuffer(n32, 0).map_err(device_error)?;
        }
        Ok(())
    }

    fn played(&mut self) -> Result<u64, MediaError> {
        let mut pos = 0u64;
        // SAFETY: a query into a local.
        unsafe { self.clock.GetPosition(&mut pos, None) }.map_err(device_error)?;
        let frames = u128::from(pos) * u128::from(self.format.sample_rate) / u128::from(self.frequency);
        Ok(u64::try_from(frames).unwrap_or(u64::MAX))
    }

    fn pause(&mut self) -> Result<(), MediaError> {
        if self.started {
            // SAFETY: stopping a started stream.
            unsafe { self.client.Stop() }.map_err(device_error)?;
            self.started = false;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), MediaError> {
        if !self.started {
            // SAFETY: starting an initialized stream.
            unsafe { self.client.Start() }.map_err(device_error)?;
            self.started = true;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        self.pause()?;
        // SAFETY: Reset on a stopped stream drops its queued sound and zeroes its clock.
        unsafe { self.client.Reset() }.map_err(device_error)
    }
}

impl Drop for Wasapi {
    fn drop(&mut self) {
        let _ = self.pause();
        if self.fine_timer {
            // SAFETY: pairs the request made when the output opened.
            let _ = unsafe { windows::Win32::Media::timeEndPeriod(1) };
        }
    }
}

/// How much sound the capture buffer holds: room for a busy machine to read late without
/// losing any, while a reader that keeps up hears it a device period after it was spoken.
const INPUT_BUFFER: Duration = Duration::from_millis(500);

/// The default microphone through WASAPI in shared mode, converting the device's own rate
/// and channels to the format asked for.
struct WasapiInput {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: AudioFormat,
    period: Duration,
}

// SAFETY: the audio client and its services live in the multithreaded apartment and are
// used from one thread at a time (the microphone's worker).
unsafe impl Send for WasapiInput {}

/// The error an input device gives, saying plainly when Windows' privacy settings keep the
/// app from the microphone (`E_ACCESSDENIED`).
fn input_error(e: windows::core::Error) -> MediaError {
    if e.code().0 as u32 == 0x8007_0005 {
        return MediaError::Denied(
            "Windows' privacy settings do not let apps use the microphone (Settings, Privacy & security, Microphone)"
                .into(),
        );
    }
    device_error(e)
}

impl WasapiInput {
    fn open(format: AudioFormat, raw: bool) -> Result<WasapiInput, MediaError> {
        com();
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(MediaError::Device("a sound format with no channels or no rate".into()));
        }
        let wave = float_wave(format);
        // SAFETY: COM calls on objects created here; the format outlives Initialize.
        unsafe {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(device_error)?;
            let device = devices
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .map_err(|e| MediaError::NotFound(format!("no microphone: {}", e.message())))?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(input_error)?;
            if raw {
                // The raw stream skips the driver's own effects (automatic gain, noise
                // suppression, echo cancellation), which bend a held note. A driver with no
                // raw mode refuses, and the stream is then its usual one.
                if let Ok(client2) = client.cast::<IAudioClient2>() {
                    let properties = AudioClientProperties {
                        cbSize: std::mem::size_of::<AudioClientProperties>() as u32,
                        bIsOffload: false.into(),
                        eCategory: AudioCategory_Other,
                        Options: AUDCLNT_STREAMOPTIONS_RAW,
                    };
                    let _ = client2.SetClientProperties(&properties);
                }
            }
            let mut period = 0i64;
            client.GetDevicePeriod(Some(&mut period), None).map_err(input_error)?;
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    hns(INPUT_BUFFER).max(period.saturating_mul(4)),
                    0,
                    &wave.Format,
                    None,
                )
                .map_err(input_error)?;
            let capture: IAudioCaptureClient = client.GetService().map_err(input_error)?;
            client.Start().map_err(input_error)?;
            Ok(WasapiInput {
                client,
                capture,
                format,
                period: from_hns(period).max(Duration::from_millis(1)),
            })
        }
    }
}

impl AudioInput for WasapiInput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn period(&self) -> Duration {
        self.period
    }

    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError> {
        let ch = usize::from(self.format.channels);
        let mut total = 0;
        loop {
            // SAFETY: a query on a live capture client.
            let next = unsafe { self.capture.GetNextPacketSize() }.map_err(input_error)?;
            if next == 0 {
                return Ok(total);
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            // SAFETY: GetBuffer hands out `frames` frames of `ch` floats at `data`, readable
            // until ReleaseBuffer, which is called with the same count after copying.
            unsafe {
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(input_error)?;
                let n = frames as usize * ch;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    into.resize(into.len() + n, 0.0);
                } else {
                    into.extend_from_slice(std::slice::from_raw_parts(data.cast::<f32>(), n));
                }
                self.capture.ReleaseBuffer(frames).map_err(input_error)?;
            }
            total += frames as usize;
        }
    }
}

impl Drop for WasapiInput {
    fn drop(&mut self) {
        // SAFETY: stopping a started stream.
        let _ = unsafe { self.client.Stop() };
    }
}
