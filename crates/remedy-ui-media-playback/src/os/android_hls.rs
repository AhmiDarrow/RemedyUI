//! Android: HLS through the crate's own reader (`crate::hls`), as on Windows and Linux, so a
//! stream starts at zero, grows and seeks the same everywhere. MediaCodec decodes the compressed
//! streams: H.264 configured from the SPS and PPS (into an image reader when a GPU renderer
//! takes pictures there, as for files), AAC from its AudioSpecificConfig, and MP3, the sound
//! handed out in the stream's own rate and channels. Playlists and segments come over HTTP/1.1
//! on a socket of this crate's (http), through Java's `HttpURLConnection` on the app's VM
//! (https: the NDK has no HTTP client), or from disk (`file:`).

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::{
    AMEDIA_OK, BUFFER_FLAG_END_OF_STREAM, CODEC_WAIT_US, Ndk, Out, PCM_FLOAT, Ptr, Reader, copy_path,
    decoder, java_env, output,
};
use crate::hls::{
    AudioCodec, AudioConfig, Elementary, FIRST_BYTE, Fetch, LOAD, MAX_FETCH, QUIET, Unit, VideoConfig,
    file_path, timed_out,
};
use crate::host::{AudioFormat, MediaError, Sample};

/// How long a decoder may go without taking input before it counts as stuck.
const STUCK: Duration = Duration::from_secs(5);
/// How long drained decoders may take to give out the end of what they held.
const DRAIN: Duration = Duration::from_secs(10);
/// Redirects followed for one fetch.
const HOPS: u32 = 5;

/// One stream's decoder.
struct Codec {
    ndk: &'static Ndk,
    codec: Ptr,
    /// The end of the stream was queued as input.
    input_done: bool,
    /// The decoder gave its last output.
    output_done: bool,
    /// It has given output (a format or a buffer): a flush keeps its setup from then on.
    began: bool,
}

impl Drop for Codec {
    fn drop(&mut self) {
        // SAFETY: stopping and deleting the decoder `open` made, once.
        unsafe {
            (self.ndk.codec_stop)(self.codec);
            (self.ndk.codec_delete)(self.codec);
        }
    }
}

/// A started decoder for `mime` set up from `ints` and `buffers`, rendering into `window` when
/// it is not null.
fn open(
    ndk: &'static Ndk,
    mime: &CStr,
    ints: &[(&CStr, i32)],
    buffers: &[(&CStr, &[u8])],
    window: Ptr,
) -> Option<Codec> {
    // SAFETY: a new format, filled and handed to the decoder (which copies it), then deleted.
    let codec = unsafe {
        let f = (ndk.format_new)();
        if f.is_null() {
            return None;
        }
        (ndk.format_set_string)(f, c"mime".as_ptr(), mime.as_ptr());
        for (k, v) in ints {
            (ndk.format_set_int32)(f, k.as_ptr(), *v);
        }
        for (k, v) in buffers {
            (ndk.format_set_buffer)(f, k.as_ptr(), v.as_ptr().cast(), v.len());
        }
        let codec = decoder(ndk, mime.to_str().unwrap_or_default(), f, window);
        (ndk.format_delete)(f);
        codec?
    };
    Some(Codec { ndk, codec, input_done: false, output_done: false, began: false })
}

/// A parameter set with its Annex B start code, as MediaCodec takes `csd-0` and `csd-1`.
fn annex_b(nal: &[u8]) -> Vec<u8> {
    [&[0, 0, 0, 1], nal].concat()
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// [`Elementary`] on MediaCodec.
pub(super) struct NdkElementary {
    ndk: &'static Ndk,
    config: (Option<VideoConfig>, Option<AudioConfig>),
    // The decoders go before `vout`, which holds the reader the video decoder renders into.
    video: Option<Codec>,
    audio: Option<Codec>,
    vout: Out,
    aout: Out,
    /// Decoded, each in presentation order.
    pictures: VecDeque<Sample>,
    sound: VecDeque<Sample>,
    /// Sound frames as decoded and as handed out since the last flush (for rate conversion).
    frames: (u64, u64),
    /// No more units follow.
    ended: bool,
}

// SAFETY: MediaCodec calls may come from any thread; the core uses this from one at a time.
unsafe impl Send for NdkElementary {}

impl NdkElementary {
    pub(super) fn new(ndk: &'static Ndk) -> NdkElementary {
        NdkElementary {
            ndk,
            config: (None, None),
            video: None,
            audio: None,
            vout: Out::default(),
            aout: Out::default(),
            pictures: VecDeque::new(),
            sound: VecDeque::new(),
            frames: (0, 0),
            ended: false,
        }
    }

    fn open_video(&mut self) -> Result<(), MediaError> {
        let Some(v) = &self.config.0 else { return Ok(()) };
        let ndk = self.ndk;
        let (w, h) = (i32::try_from(v.width).unwrap_or(0), i32::try_from(v.height).unwrap_or(0));
        let (sps, pps) = (annex_b(&v.sps), annex_b(&v.pps));
        // Room for a key frame however the decoder sizes its buffers by default.
        let ints = [(c"width", w), (c"height", h), (c"max-input-size", (w * h * 3 / 2).max(1 << 16))];
        let buffers: [(&CStr, &[u8]); 2] = [(c"csd-0", &sps), (c"csd-1", &pps)];
        self.video = None;
        self.vout = Out::default();
        // On the GPU when a renderer takes pictures there, else copied.
        if remedy_ui_paint::gpu_video_adapter().is_some() {
            match Reader::new(ndk, w, h) {
                Ok((reader, window)) => match open(ndk, c"video/avc", &ints, &buffers, window) {
                    Some(c) => {
                        self.video = Some(c);
                        self.vout.reader = Some(Arc::new(reader));
                    }
                    None => copy_path("the decoder would not render into an image reader"),
                },
                Err(why) => copy_path(&why),
            }
        }
        if self.video.is_none() {
            self.video =
                Some(open(ndk, c"video/avc", &ints, &buffers, std::ptr::null_mut()).ok_or_else(|| {
                    MediaError::Unsupported(format!(
                        "this device has no H.264 decoder for {} ({w}x{h})",
                        v.codec
                    ))
                })?);
        }
        Ok(())
    }

    fn open_audio(&mut self) -> Result<(), MediaError> {
        let Some(a) = &self.config.1 else { return Ok(()) };
        let (rate, ch) = (i32::try_from(a.sample_rate).unwrap_or(0), i32::from(a.channels));
        // Float samples where the decoder gives them (Android 9; 16-bit ones are converted),
        // and every channel rather than a stereo downmix.
        let ints = [
            (c"sample-rate", rate),
            (c"channel-count", ch),
            (c"pcm-encoding", PCM_FLOAT),
            (c"aac-max-output-channel_count", ch),
            (c"max-output-channel_count", ch),
        ];
        let (mime, asc): (&CStr, &[u8]) = match &a.codec {
            AudioCodec::Aac { asc } => (c"audio/mp4a-latm", asc),
            AudioCodec::Mp3 => (c"audio/mpeg", &[]),
        };
        let buffers: &[(&CStr, &[u8])] = if asc.is_empty() { &[] } else { &[(c"csd-0", asc)] };
        self.audio = None;
        self.aout = Out::default();
        self.audio = Some(open(self.ndk, mime, &ints, buffers, std::ptr::null_mut()).ok_or_else(|| {
            MediaError::Unsupported(format!("this device has no {} decoder", a.codec_string))
        })?);
        Ok(())
    }

    /// Takes what a decoder has finished, waiting up to `wait_us` for the first; whether
    /// anything came.
    fn drain(&mut self, video: bool, wait_us: i64) -> Result<bool, MediaError> {
        let (codec, out, queue) = if video {
            (self.video.as_mut(), &mut self.vout, &mut self.pictures)
        } else {
            (self.audio.as_mut(), &mut self.aout, &mut self.sound)
        };
        let Some(c) = codec.filter(|c| !c.output_done) else { return Ok(false) };
        let mut got = false;
        while let Some((sample, eos)) = output(self.ndk, c.codec, video, out, if got { 0 } else { wait_us })?
        {
            got = true;
            c.began = true;
            c.output_done |= eos;
            queue.extend(sample);
            if eos {
                break;
            }
        }
        Ok(got)
    }

    /// Queues `data` at `at` (with `flags`) into a decoder, taking its output while it has no
    /// room.
    fn queue(&mut self, video: bool, data: &[u8], at: Duration, flags: u32) -> Result<(), MediaError> {
        let ndk = self.ndk;
        let since = Instant::now();
        loop {
            let codec = if video { self.video.as_ref() } else { self.audio.as_ref() };
            let codec = codec
                .ok_or_else(|| {
                    MediaError::Format(
                        if video { "video with no video stream" } else { "sound with no sound stream" }
                            .into(),
                    )
                })?
                .codec;
            // SAFETY: an input buffer of a live decoder, filled with no more than it holds.
            unsafe {
                let i = (ndk.codec_dequeue_input_buffer)(codec, 0);
                if i >= 0 {
                    let mut cap = 0usize;
                    let buf = (ndk.codec_get_input_buffer)(codec, i as usize, &mut cap);
                    if buf.is_null() || cap < data.len() {
                        return Err(MediaError::Decode(format!(
                            "a compressed piece of {} bytes does not fit the decoder's {cap}",
                            data.len()
                        )));
                    }
                    std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
                    if (ndk.codec_queue_input_buffer)(codec, i as usize, 0, data.len(), micros(at), flags)
                        != AMEDIA_OK
                    {
                        return Err(MediaError::Decode("the decoder refused a compressed piece".into()));
                    }
                    return Ok(());
                }
            }
            // Room comes as its output is taken.
            self.drain(video, CODEC_WAIT_US)?;
            if since.elapsed() > STUCK {
                return Err(MediaError::Decode("the decoder stopped taking the stream".into()));
            }
        }
    }

    /// The earlier of the two streams' next samples, the sound in the stream's own format.
    fn pop(&mut self) -> Option<Sample> {
        let at = |q: &VecDeque<Sample>| match q.front() {
            Some(Sample::Video { at, .. } | Sample::Audio { at, .. }) => Some(*at),
            _ => None,
        };
        let sound_first = match (at(&self.pictures), at(&self.sound)) {
            (Some(v), Some(a)) => a < v,
            (None, Some(_)) => true,
            _ => false,
        };
        if !sound_first {
            return self.pictures.pop_front().or_else(|| self.sound.pop_front());
        }
        let s = self.sound.pop_front()?;
        let (Sample::Audio { at, samples }, Some(a)) = (&s, &self.config.1) else { return Some(s) };
        let channels = if self.aout.channels > 0 { self.aout.channels } else { usize::from(a.channels) };
        let rate = if self.aout.rate > 0 { self.aout.rate } else { a.sample_rate };
        let to = AudioFormat::new(a.sample_rate, a.channels);
        Some(Sample::Audio { at: *at, samples: convert(samples, channels, rate, to, &mut self.frames) })
    }
}

/// Sound as a decoder gave it (`channels` at `rate`) in the format `to`: channels mapped (a
/// mono stream given as stereo is averaged back), the rate converted linearly (an AAC decoder
/// doing SBR on a stream that did not say so doubles it). `frames` counts the frames in and out
/// since the last flush, so the length comes out right however the pieces fall.
fn convert(
    samples: &[f32],
    channels: usize,
    rate: u32,
    to: AudioFormat,
    frames: &mut (u64, u64),
) -> Vec<f32> {
    let (from, want) = (channels.max(1), usize::from(to.channels.max(1)));
    let mapped: Vec<f32> = if from == want {
        samples.to_vec()
    } else {
        samples
            .chunks_exact(from)
            .flat_map(|f| {
                let mean = f.iter().sum::<f32>() / from as f32;
                (0..want).map(
                    move |c| if want == 1 { mean } else { f.get(c.min(from - 1)).copied().unwrap_or(0.0) },
                )
            })
            .collect()
    };
    if rate == to.sample_rate || rate == 0 {
        return mapped;
    }
    let n = mapped.len() / want;
    frames.0 += n as u64;
    let total = frames.0 * u64::from(to.sample_rate) / u64::from(rate);
    let m = total.saturating_sub(frames.1);
    frames.1 = total;
    let frame = |i: usize| mapped.get(i * want..(i + 1) * want);
    let step = n as f64 / m.max(1) as f64;
    (0..m as usize)
        .flat_map(|j| {
            let x = j as f64 * step;
            let i = x as usize;
            let t = (x - i as f64) as f32;
            let (a, b) = (frame(i), frame((i + 1).min(n.saturating_sub(1))));
            (0..want).map(move |c| {
                let a = a.and_then(|f| f.get(c)).copied().unwrap_or(0.0);
                let b = b.and_then(|f| f.get(c)).copied().unwrap_or(a);
                a + (b - a) * t
            })
        })
        .collect()
}

impl Elementary for NdkElementary {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        self.config = (video.cloned(), audio.cloned());
        self.open_video()?;
        self.open_audio()
    }

    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        match unit {
            Unit::Video { pts, data, .. } => self.queue(true, &data, pts, 0),
            Unit::Audio { pts, data } => self.queue(false, &data, pts, 0),
        }
    }

    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        let since = Instant::now();
        loop {
            self.drain(true, 0)?;
            self.drain(false, 0)?;
            if let Some(s) = self.pop() {
                return Ok(Some(s));
            }
            let video_left = self.video.as_ref().is_some_and(|c| !c.output_done);
            let audio_left = self.audio.as_ref().is_some_and(|c| !c.output_done);
            if !self.ended || !(video_left || audio_left) {
                return Ok(None);
            }
            // Only the decoders' tails are left: wait for them.
            if since.elapsed() > DRAIN {
                return Err(MediaError::Decode("the decoder did not give out the end of the stream".into()));
            }
            self.drain(video_left, CODEC_WAIT_US)?;
        }
    }

    fn end(&mut self) -> Result<(), MediaError> {
        for video in [true, false] {
            let c = if video { self.video.as_ref() } else { self.audio.as_ref() };
            if c.is_some_and(|c| !c.input_done) {
                self.queue(video, &[], Duration::ZERO, BUFFER_FLAG_END_OF_STREAM)?;
                let c = if video { self.video.as_mut() } else { self.audio.as_mut() };
                if let Some(c) = c {
                    c.input_done = true;
                }
            }
        }
        self.ended = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        self.pictures.clear();
        self.sound.clear();
        self.frames = (0, 0);
        self.ended = false;
        for video in [true, false] {
            let c = if video { self.video.as_mut() } else { self.audio.as_mut() };
            match c {
                // A decoder flushed before its first output loses the setup it took from its
                // format: a new one.
                Some(c) if !c.began => {
                    if video {
                        self.open_video()?
                    } else {
                        self.open_audio()?
                    }
                }
                Some(c) => {
                    // SAFETY: flushing a live decoder.
                    if unsafe { (self.ndk.codec_flush)(c.codec) } != AMEDIA_OK {
                        return Err(MediaError::Decode("the decoder would not flush".into()));
                    }
                    c.input_done = false;
                    c.output_done = false;
                }
                None => {}
            }
        }
        Ok(())
    }
}

/// [`Fetch`] on Android: http over a socket of this crate's, https through Java (the app's VM),
/// `file:` from disk.
pub(super) struct NdkFetch;

impl Fetch for NdkFetch {
    fn get(&mut self, url: &str, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
        fetch(url, 0, stop)
    }
}

fn scheme<'a>(url: &'a str, scheme: &str) -> Option<&'a str> {
    url.get(..scheme.len()).filter(|s| s.eq_ignore_ascii_case(scheme)).and(url.get(scheme.len()..))
}

fn fetch(url: &str, hops: u32, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
    if let Some(path) = file_path(url) {
        return std::fs::read(&path).map_err(|e| MediaError::NotFound(format!("{path}: {e}")));
    }
    if let Some(rest) = scheme(url, "http://") {
        return http(url, rest, hops, stop);
    }
    if scheme(url, "https://").is_some() {
        return java_get(url, stop);
    }
    Err(MediaError::NotFound(format!("{url}: not an http, https or file URL")))
}

/// A failed read or write of `url`'s connection; one that waited too long (`quiet`) is a timeout.
fn io_error(url: &str, e: &std::io::Error, quiet: &str) -> MediaError {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => timed_out(url, quiet),
        _ => MediaError::NotFound(format!("{url}: {e}")),
    }
}

/// GET `url` (`rest` is what follows `http://`) over HTTP/1.1: no answer for [`FIRST_BYTE`],
/// no byte for [`QUIET`] once it answered, or [`LOAD`] in all is a timeout; set `stop` and the
/// socket is shut at once (a watcher thread), ending the read waiting on it.
fn http(url: &str, rest: &str, hops: u32, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
    let started = Instant::now();
    let rest = rest.split('#').next().unwrap_or(rest);
    let (authority, path) = rest.find(['/', '?']).map_or((rest, "/"), |i| rest.split_at(i));
    let path = if path.starts_with('?') { format!("/{path}") } else { path.to_owned() };
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !p.contains(']') => {
            (h, p.parse::<u16>().map_err(|_| MediaError::NotFound(format!("{url}: no port {p}")))?)
        }
        _ => (hostport, 80),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs = (host, port).to_socket_addrs().map_err(|e| MediaError::NotFound(format!("{url}: {e}")))?;
    let mut last = None;
    let mut stream = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, FIRST_BYTE) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let Some(s) = stream else {
        return Err(last.map_or_else(
            || MediaError::NotFound(format!("{url}: no address for {host}")),
            |e| io_error(url, &e, "no answer"),
        ));
    };
    let watched = s.try_clone().map_err(|e| io_error(url, &e, "no answer"))?;
    let (done, finished) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        // Named, so the leak tests count it; without it the socket's own timeouts still end the call.
        let _ = std::thread::Builder::new().name("remedy-hls-stop".into()).spawn_scoped(scope, move || {
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = finished.recv_timeout(WATCH) {
                if stop.load(Ordering::Relaxed) {
                    let _ = watched.shutdown(std::net::Shutdown::Both);
                    return;
                }
            }
        });
        let got = exchange(s, url, authority, &path, hops, started, stop);
        drop(done);
        got
    })
}

/// The request and answer of [`http`] on the connected `s`.
fn exchange(
    mut s: TcpStream,
    url: &str,
    authority: &str,
    path: &str,
    hops: u32,
    started: Instant,
    stop: &AtomicBool,
) -> Result<Vec<u8>, MediaError> {
    let io = |e: std::io::Error| io_error(url, &e, "no answer");
    s.set_read_timeout(Some(FIRST_BYTE)).map_err(io)?;
    s.set_write_timeout(Some(FIRST_BYTE)).map_err(io)?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: remedy-ui-media-playback\r\n\
         Accept-Encoding: identity\r\nConnection: close\r\n\r\n"
    )
    .map_err(io)?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).map_err(io)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| MediaError::NotFound(format!("{url}: not an HTTP answer")))?;
    let (mut length, mut chunked, mut location) = (None, false, None);
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(io)? == 0 {
            break;
        }
        let Some((name, value)) = line.trim_end().split_once(':') else { break };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse::<usize>().ok(),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            "location" => location = Some(value.to_owned()),
            _ => {}
        }
    }
    if (300..400).contains(&status)
        && let Some(to) = location
    {
        if hops >= HOPS {
            return Err(MediaError::NotFound(format!("{url}: too many redirects")));
        }
        let to = if to.contains("://") {
            to
        } else {
            format!("http://{authority}/{}", to.trim_start_matches('/'))
        };
        return fetch(&to, hops + 1, stop);
    }
    if !(200..300).contains(&status) {
        return Err(MediaError::NotFound(format!("{url}: HTTP {status}")));
    }
    r.get_ref().set_read_timeout(Some(QUIET)).map_err(io)?;
    let mut body = Vec::with_capacity(length.unwrap_or(0).min(64 << 20));
    if chunked {
        loop {
            line.clear();
            r.read_line(&mut line).map_err(|e| io_error(url, &e, "nothing arrived"))?;
            let size = line.trim().split(';').next().unwrap_or_default();
            let size = usize::from_str_radix(size, 16)
                .map_err(|_| MediaError::NotFound(format!("{url}: a broken chunked answer")))?;
            if size == 0 {
                break;
            }
            read_body(&mut r, Some(size), &mut body, url, started)?;
            line.clear();
            r.read_line(&mut line).map_err(|e| io_error(url, &e, "nothing arrived"))?;
        }
    } else {
        read_body(&mut r, length, &mut body, url, started)?;
    }
    Ok(body)
}

/// Reads `len` bytes into `into` (to the end of the connection when `None`).
fn read_body(
    r: &mut impl Read,
    len: Option<usize>,
    into: &mut Vec<u8>,
    url: &str,
    started: Instant,
) -> Result<(), MediaError> {
    let mut buf = vec![0u8; 64 << 10];
    let mut left = len;
    while left != Some(0) {
        if started.elapsed() > LOAD {
            return Err(timed_out(url, "the download took too long"));
        }
        let want = left.map_or(buf.len(), |l| l.min(buf.len()));
        let room = buf.get_mut(..want).unwrap_or_default();
        match r.read(room) {
            Ok(0) if left.is_some() => {
                return Err(MediaError::NotFound(format!("{url}: the connection closed early")));
            }
            Ok(0) => return Ok(()),
            Ok(n) => {
                into.extend_from_slice(room.get(..n).unwrap_or_default());
                if into.len() > MAX_FETCH {
                    return Err(MediaError::Format(format!("{url}: over {MAX_FETCH} bytes")));
                }
                left = left.map(|l| l - n);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_error(url, &e, "nothing arrived")),
        }
    }
    Ok(())
}

/// GET `url` (https) through Java's `HttpURLConnection` on this thread, attached to the app's
/// Java VM.
fn java_get(url: &str, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
    let env = java_env().ok_or_else(|| {
        MediaError::Unsupported(format!("{url}: https needs the app's Java VM (it is read through Java)"))
    })?;
    let j = Jni(env);
    // SAFETY: JNI calls on this thread's environment; every local reference made is freed
    // with the frame.
    unsafe {
        if j.f::<unsafe extern "C" fn(Ptr, i32) -> i32>(PUSH_LOCAL_FRAME)(env, 32) != 0 {
            j.clear();
            return Err(MediaError::NotFound(format!("{url}: no room for Java references")));
        }
        let got = j.get(url, stop);
        j.f::<unsafe extern "C" fn(Ptr, Ptr) -> Ptr>(POP_LOCAL_FRAME)(env, std::ptr::null_mut());
        got
    }
}

// `JNINativeInterface`'s functions, by index.
const FIND_CLASS: usize = 6;
const EXCEPTION_OCCURRED: usize = 15;
const EXCEPTION_CLEAR: usize = 17;
const PUSH_LOCAL_FRAME: usize = 19;
const POP_LOCAL_FRAME: usize = 20;
const NEW_GLOBAL_REF: usize = 21;
const DELETE_GLOBAL_REF: usize = 22;
const NEW_OBJECT_A: usize = 30;
const IS_INSTANCE_OF: usize = 32;
const GET_METHOD_ID: usize = 33;
const CALL_OBJECT_METHOD_A: usize = 36;
const CALL_INT_METHOD_A: usize = 51;
const CALL_VOID_METHOD_A: usize = 63;
const NEW_STRING_UTF: usize = 167;
const GET_STRING_UTF_CHARS: usize = 169;
const RELEASE_STRING_UTF_CHARS: usize = 170;
const NEW_BYTE_ARRAY: usize = 176;
const GET_BYTE_ARRAY_REGION: usize = 200;

/// A thread's `JNIEnv`. Arguments go as `jvalue`s: a reference or a `jint` in the low bytes of
/// a 64-bit slot (both Android targets are little-endian).
struct Jni(Ptr);

type Call<R> = unsafe extern "C" fn(Ptr, Ptr, Ptr, *const u64) -> R;

impl Jni {
    /// The environment's function at `index`, as `F` (its JNI signature).
    unsafe fn f<F: Copy>(&self, index: usize) -> F {
        // SAFETY: the caller names the function's JNI signature; the table lives with the VM.
        unsafe {
            let table = *self.0.cast::<*const Ptr>();
            std::mem::transmute_copy::<Ptr, F>(&*table.add(index))
        }
    }

    /// Clears a pending exception: whether it was a timeout, and what it said.
    unsafe fn clear(&self) -> Option<(bool, String)> {
        // SAFETY: JNI calls by their signatures on this thread's environment.
        unsafe {
            let e = self.f::<unsafe extern "C" fn(Ptr) -> Ptr>(EXCEPTION_OCCURRED)(self.0);
            if e.is_null() {
                return None;
            }
            self.f::<unsafe extern "C" fn(Ptr)>(EXCEPTION_CLEAR)(self.0);
            let find = self.f::<unsafe extern "C" fn(Ptr, *const c_char) -> Ptr>(FIND_CLASS);
            let timeout = find(self.0, c"java/net/SocketTimeoutException".as_ptr());
            let is = !timeout.is_null()
                && self.f::<unsafe extern "C" fn(Ptr, Ptr, Ptr) -> u8>(IS_INSTANCE_OF)(self.0, e, timeout)
                    != 0;
            self.f::<unsafe extern "C" fn(Ptr)>(EXCEPTION_CLEAR)(self.0);
            let text =
                self.method(c"java/lang/Throwable", c"toString", c"()Ljava/lang/String;")
                    .ok()
                    .map(|m| self.f::<Call<Ptr>>(CALL_OBJECT_METHOD_A)(self.0, e, m, std::ptr::null()))
                    .filter(|s| !s.is_null())
                    .map(|s| {
                        let chars = self.f::<unsafe extern "C" fn(Ptr, Ptr, *mut u8) -> *const c_char>(
                            GET_STRING_UTF_CHARS,
                        )(self.0, s, std::ptr::null_mut());
                        let t = if chars.is_null() {
                            String::new()
                        } else {
                            CStr::from_ptr(chars).to_string_lossy().into_owned()
                        };
                        if !chars.is_null() {
                            self.f::<unsafe extern "C" fn(Ptr, Ptr, *const c_char)>(RELEASE_STRING_UTF_CHARS)(
                                self.0, s, chars,
                            );
                        }
                        t
                    })
                    .unwrap_or_else(|| "a Java exception".into());
            self.f::<unsafe extern "C" fn(Ptr)>(EXCEPTION_CLEAR)(self.0);
            Some((is, text))
        }
    }

    /// `Ok` when no exception is pending; else the error it is for `url`.
    unsafe fn check(&self, url: &str) -> Result<(), MediaError> {
        // SAFETY: as in `clear`.
        match unsafe { self.clear() } {
            None => Ok(()),
            Some((true, t)) => Err(timed_out(url, &t)),
            Some((false, t)) => Err(MediaError::NotFound(format!("{url}: {t}"))),
        }
    }

    /// The method `name` (`sig`) of the class `class`.
    unsafe fn method(&self, class: &CStr, name: &CStr, sig: &CStr) -> Result<Ptr, MediaError> {
        // SAFETY: JNI calls by their signatures; a failed lookup's exception is cleared.
        unsafe {
            let c =
                self.f::<unsafe extern "C" fn(Ptr, *const c_char) -> Ptr>(FIND_CLASS)(self.0, class.as_ptr());
            let m =
                if c.is_null() {
                    std::ptr::null_mut()
                } else {
                    self.f::<unsafe extern "C" fn(Ptr, Ptr, *const c_char, *const c_char) -> Ptr>(
                        GET_METHOD_ID,
                    )(self.0, c, name.as_ptr(), sig.as_ptr())
                };
            if m.is_null() {
                self.f::<unsafe extern "C" fn(Ptr)>(EXCEPTION_CLEAR)(self.0);
                return Err(MediaError::Unsupported(format!(
                    "Java has no {}.{}",
                    class.to_string_lossy(),
                    name.to_string_lossy()
                )));
            }
            Ok(m)
        }
    }

    unsafe fn get(&self, url: &str, stop: &AtomicBool) -> Result<Vec<u8>, MediaError> {
        let env = self.0;
        let ms = |d: Duration| d.as_millis() as u64;
        let curl = CString::new(url).map_err(|e| MediaError::NotFound(format!("{url}: {e}")))?;
        // SAFETY: JNI calls by their signatures, each checked for an exception; the byte array
        // is copied out no further than `read` said it filled.
        unsafe {
            let obj = self.f::<Call<Ptr>>(CALL_OBJECT_METHOD_A);
            let void = self.f::<Call<()>>(CALL_VOID_METHOD_A);
            let s =
                self.f::<unsafe extern "C" fn(Ptr, *const c_char) -> Ptr>(NEW_STRING_UTF)(env, curl.as_ptr());
            self.check(url)?;
            let class = self.f::<unsafe extern "C" fn(Ptr, *const c_char) -> Ptr>(FIND_CLASS)(
                env,
                c"java/net/URL".as_ptr(),
            );
            self.check(url)?;
            let init = self.method(c"java/net/URL", c"<init>", c"(Ljava/lang/String;)V")?;
            let u = self.f::<Call<Ptr>>(NEW_OBJECT_A)(env, class, init, [s as u64].as_ptr());
            self.check(url)?;
            let open = self.method(c"java/net/URL", c"openConnection", c"()Ljava/net/URLConnection;")?;
            let conn = obj(env, u, open, std::ptr::null());
            self.check(url)?;
            let http = c"java/net/HttpURLConnection";
            let set_connect = self.method(http, c"setConnectTimeout", c"(I)V")?;
            let set_read = self.method(http, c"setReadTimeout", c"(I)V")?;
            let disconnect = self.method(http, c"disconnect", c"()V")?;
            void(env, conn, set_connect, [ms(FIRST_BYTE)].as_ptr());
            // Java's own read timeout cannot change once connected: it only backs the watchdog up.
            void(env, conn, set_read, [ms(FIRST_BYTE)].as_ptr());
            self.check(url)?;
            let dog = Watchdog::start(self, conn, disconnect, stop)?;
            let got = self.read_all(url, conn, &dog);
            void(env, conn, disconnect, std::ptr::null());
            self.clear();
            let gave_up = dog.stop(self);
            match (got, gave_up) {
                (Err(_), Some(why)) => Err(timed_out(url, why)),
                (got, _) => got,
            }
        }
    }

    /// The answer's body: the connection's status, then its bytes, telling `dog` of each.
    unsafe fn read_all(&self, url: &str, conn: Ptr, dog: &Watchdog) -> Result<Vec<u8>, MediaError> {
        let env = self.0;
        let http = c"java/net/HttpURLConnection";
        // SAFETY: as in `get`.
        unsafe {
            let obj = self.f::<Call<Ptr>>(CALL_OBJECT_METHOD_A);
            let int = self.f::<Call<i32>>(CALL_INT_METHOD_A);
            let void = self.f::<Call<()>>(CALL_VOID_METHOD_A);
            let code = int(env, conn, self.method(http, c"getResponseCode", c"()I")?, std::ptr::null());
            self.check(url)?;
            dog.progress(QUIET);
            if !(200..300).contains(&code) {
                return Err(MediaError::NotFound(format!("{url}: HTTP {code}")));
            }
            let input = obj(
                env,
                conn,
                self.method(http, c"getInputStream", c"()Ljava/io/InputStream;")?,
                std::ptr::null(),
            );
            self.check(url)?;
            let read = self.method(c"java/io/InputStream", c"read", c"([B)I")?;
            let close = self.method(c"java/io/InputStream", c"close", c"()V")?;
            const CHUNK: i32 = 64 << 10;
            let buf = self.f::<unsafe extern "C" fn(Ptr, i32) -> Ptr>(NEW_BYTE_ARRAY)(env, CHUNK);
            self.check(url)?;
            let region = self.f::<unsafe extern "C" fn(Ptr, Ptr, i32, i32, *mut i8)>(GET_BYTE_ARRAY_REGION);
            let mut out = Vec::new();
            let result = loop {
                let n = int(env, input, read, [buf as u64].as_ptr());
                if let Err(e) = self.check(url) {
                    break Err(e);
                }
                let Ok(len) = usize::try_from(n) else { break Ok(()) };
                dog.progress(QUIET);
                let at = out.len();
                if at + len > MAX_FETCH {
                    break Err(MediaError::Format(format!("{url}: over {MAX_FETCH} bytes")));
                }
                out.resize(at + len, 0);
                region(env, buf, 0, n.min(CHUNK), out.as_mut_ptr().add(at).cast());
            };
            void(env, input, close, std::ptr::null());
            self.clear();
            result.map(|()| out)
        }
    }
}

/// Ends a Java fetch that waits too long: no answer for [`FIRST_BYTE`], no byte for [`QUIET`]
/// once it answered, or [`LOAD`] in all, or at once when `stop` is set. A thread of its own calls the connection's
/// `disconnect()`, which ends the call waiting on it, as the WinHTTP watchdog does on Windows
/// (Java's own read timeout cannot change once connected).
struct Watchdog {
    progress: Arc<Progress>,
    /// Dropped to end the thread at once.
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// A global reference to the connection, for the watchdog's thread.
    conn: Ptr,
}

struct Progress {
    start: Instant,
    /// Milliseconds after `start` that something last came, and how long it may then be quiet.
    last: AtomicU64,
    limit: AtomicU64,
    gave_up: AtomicBool,
}

/// How often the watchdog looks.
const WATCH: Duration = Duration::from_millis(50);

impl Watchdog {
    unsafe fn start(j: &Jni, conn: Ptr, disconnect: Ptr, halt: &AtomicBool) -> Result<Watchdog, MediaError> {
        // SAFETY: a global reference (usable from any thread) to a live local one.
        let global = unsafe { j.f::<unsafe extern "C" fn(Ptr, Ptr) -> Ptr>(NEW_GLOBAL_REF)(j.0, conn) };
        if global.is_null() {
            return Err(MediaError::NotFound("no room for a Java reference".into()));
        }
        let progress = Arc::new(Progress {
            start: Instant::now(),
            last: AtomicU64::new(0),
            limit: AtomicU64::new(FIRST_BYTE.as_millis() as u64),
            gave_up: AtomicBool::new(false),
        });
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let p = progress.clone();
        // The thread is joined before the watchdog goes (`stop` or drop), so `stop` outlives it.
        let (conn_ref, method, halt) =
            (global as usize, disconnect as usize, halt as *const AtomicBool as usize);
        let thread = std::thread::Builder::new()
            .name("remedy-hls-watchdog".into())
            .spawn(move || {
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(WATCH) {
                    let since = p.start.elapsed().as_millis() as u64;
                    let quiet = since.saturating_sub(p.last.load(Ordering::Relaxed));
                    // SAFETY: `halt` borrows the caller's flag, alive until this thread is joined.
                    let halted = unsafe { &*(halt as *const AtomicBool) }.load(Ordering::Relaxed);
                    if !halted && since <= LOAD.as_millis() as u64 && quiet <= p.limit.load(Ordering::Relaxed)
                    {
                        continue;
                    }
                    p.gave_up.store(!halted, Ordering::Relaxed);
                    if let Some(env) = java_env() {
                        let j = Jni(env);
                        // SAFETY: `disconnect()` on the connection the global reference holds
                        // (deleted only after this thread is joined), from this attached thread.
                        unsafe {
                            j.f::<Call<()>>(CALL_VOID_METHOD_A)(
                                env,
                                conn_ref as Ptr,
                                method as Ptr,
                                std::ptr::null(),
                            );
                            j.clear();
                        }
                    }
                    return;
                }
            })
            .map_err(|e| MediaError::NotFound(format!("could not start the fetch's watchdog: {e}")))?;
        Ok(Watchdog { progress, stop: Some(stop), thread: Some(thread), conn: global })
    }

    fn join(&mut self) {
        self.stop.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Something came: the watch starts again, allowing `quiet` until the next.
    fn progress(&self, quiet: Duration) {
        let p = &self.progress;
        p.last.store(p.start.elapsed().as_millis() as u64, Ordering::Relaxed);
        p.limit.store(quiet.as_millis() as u64, Ordering::Relaxed);
    }

    /// Ends the watch; why it gave up, if it did.
    fn stop(mut self, j: &Jni) -> Option<&'static str> {
        self.join();
        // SAFETY: the watchdog's thread is done with the global reference.
        unsafe { j.f::<unsafe extern "C" fn(Ptr, Ptr)>(DELETE_GLOBAL_REF)(j.0, self.conn) };
        let p = &self.progress;
        p.gave_up.load(Ordering::Relaxed).then(|| {
            if p.start.elapsed() > LOAD {
                "the download took too long"
            } else if p.limit.load(Ordering::Relaxed) == QUIET.as_millis() as u64 {
                "nothing arrived"
            } else {
                "no answer"
            }
        })
    }
}

impl Drop for Watchdog {
    /// An early return still joins the thread, which borrows the caller's `stop`.
    fn drop(&mut self) {
        self.join();
    }
}
