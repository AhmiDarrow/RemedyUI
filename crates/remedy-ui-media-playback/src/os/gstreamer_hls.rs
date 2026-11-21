//! Linux: HLS through the crate's own reader (`crate::hls`), as on Windows and in a page, so a
//! stream starts at zero, grows and seeks the same everywhere. GStreamer decodes the compressed
//! streams (`appsrc ! h264parse ! <the H.264 decoder the stream decoder chose> ! appsink`, and
//! `appsrc ! aacparse|mpegaudioparse ! decodebin ! appsink`) and fetches the playlists and
//! segments through its URI sources (`souphttpsrc` for http and https, `filesrc` for files).

use std::ffi::CStr;
use std::time::{Duration, Instant};

use super::gstreamer::{
    CLOCK_TIME_NONE, GST_MAP_WRITE, Gst, GstMapInfo, GstStreamDecoder, gst, launch_error, map_error,
    nv12_picture,
};
use super::gstreamer_stream::{Pipe, pull};
use crate::hls::{
    AudioCodec, AudioConfig, Elementary, FIRST_BYTE, Fetch, LOAD, QUIET, Unit, VideoConfig, file_path,
    timed_out,
};
use crate::host::{MediaError, Sample, VideoCodec};

const PURPOSE: &str = "playing HLS streams";
/// Units a decoder may hold before `pull` waits for it; doubled each time a decoder holds
/// more (frame-threaded decoders hold a picture per thread).
const HOLD: u32 = 8;
/// How long `pull` waits on a decoder holding `HOLD` units before raising its `HOLD`.
const STALL: Duration = Duration::from_millis(250);
/// How long a drained decoder may take to give out the end of what it held.
const DRAIN: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(5);

/// `desc` built, a missing element (or URI handler) named as the package to install.
fn build(g: &'static Gst, desc: &str, names: &[&CStr]) -> Result<Pipe, MediaError> {
    Pipe::build(g, desc, names).map_err(|e| match e {
        MediaError::Unsupported(t) if t.contains("for URI") => {
            MediaError::Missing(crate::deps::element("souphttpsrc", PURPOSE))
        }
        MediaError::Unsupported(t) => launch_error(t, PURPOSE, MediaError::Unsupported),
        e => e,
    })
}

/// An error posted on the pipeline's bus, as the error it is.
fn check(g: &Gst, p: &Pipe) -> Result<(), MediaError> {
    match g.bus_error(p.pipeline) {
        Some((code, text)) => Err(match map_error(code, text) {
            MediaError::Format(t) => MediaError::Decode(t),
            e => e,
        }),
        None => Ok(()),
    }
}

/// One stream's `appsrc ! … ! appsink` (named `src` and `sink`).
struct Stream {
    pipe: Pipe,
    video: bool,
    /// Units pushed and not yet out.
    held: u32,
    hold: u32,
    eos: bool,
}

impl Stream {
    fn open(g: &'static Gst, desc: &str, video: bool) -> Result<Stream, MediaError> {
        let pipe = build(g, desc, &[c"src", c"sink"])?;
        pipe.start("the HLS decoder")?;
        Ok(Stream { pipe, video, held: 0, hold: HOLD, eos: false })
    }

    fn push(&mut self, g: &Gst, data: &[u8], pts: Duration, dts: Duration) -> Result<(), MediaError> {
        check(g, &self.pipe)?;
        let ns = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(CLOCK_TIME_NONE);
        // SAFETY: a new buffer of the piece's size, mapped for writing while filled, its times
        // set, then handed to the source (which takes ownership).
        unsafe {
            let buf = (g.buffer_new_allocate)(std::ptr::null_mut(), data.len(), std::ptr::null_mut());
            if buf.is_null() {
                return Err(MediaError::Decode("no memory for a compressed piece".into()));
            }
            let mut map = GstMapInfo::default();
            if (g.buffer_map)(buf, &mut map, GST_MAP_WRITE) == 0
                || map.data.is_null()
                || map.size < data.len()
            {
                (g.mini_object_unref)(buf.cast());
                return Err(MediaError::Decode("a compressed piece could not be written".into()));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), map.data, data.len());
            (g.buffer_unmap)(buf, &mut map);
            (*buf).pts = ns(pts);
            (*buf).dts = ns(dts);
            if (g.app_src_push_buffer)(self.pipe.get(0), buf) != 0 {
                check(g, &self.pipe)?;
                return Err(MediaError::Decode("the decoder stopped taking the stream".into()));
            }
        }
        self.held += 1;
        Ok(())
    }

    /// The next decoded sample, waiting up to `wait`.
    fn take(&mut self, g: &Gst, wait: Duration) -> Result<Option<Sample>, MediaError> {
        check(g, &self.pipe)?;
        let sink = self.pipe.get(1);
        let Some((bytes, _, s)) = pull(g, sink, wait) else {
            // SAFETY: asking a live app sink.
            self.eos |= unsafe { (g.app_sink_is_eos)(sink) } != 0;
            return Ok(None);
        };
        self.held = self.held.saturating_sub(1);
        // SAFETY: reading the times of the sample `pull` handed over, then releasing it once.
        let (pts, duration) = unsafe {
            let buf = (g.sample_get_buffer)(s);
            if buf.is_null() { (CLOCK_TIME_NONE, CLOCK_TIME_NONE) } else { ((*buf).pts, (*buf).duration) }
        };
        let at = if pts == CLOCK_TIME_NONE { Duration::ZERO } else { Duration::from_nanos(pts) };
        let out = if self.video {
            nv12_picture(g, s, &bytes).map(|picture| {
                let duration = (duration != CLOCK_TIME_NONE).then(|| Duration::from_nanos(duration));
                Some(Sample::Video { at, duration, picture })
            })
        } else {
            let samples: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
            Ok((!samples.is_empty()).then_some(Sample::Audio { at, samples }))
        };
        // SAFETY: as above.
        unsafe { (g.mini_object_unref)(s) };
        out
    }
}

/// [`Elementary`] on GStreamer.
pub(crate) struct GstElementary {
    g: &'static Gst,
    config: (Option<VideoConfig>, Option<AudioConfig>),
    video: Option<Stream>,
    audio: Option<Stream>,
    ended: bool,
}

// SAFETY: GStreamer elements are thread-safe objects; the core uses this from one thread at a
// time.
unsafe impl Send for GstElementary {}

impl GstElementary {
    pub(crate) fn new() -> Result<GstElementary, MediaError> {
        Ok(GstElementary { g: gst()?, config: (None, None), video: None, audio: None, ended: false })
    }

    /// The pipelines for the configured streams, new.
    fn start(&mut self) -> Result<(), MediaError> {
        let g = self.g;
        self.ended = false;
        self.video = None;
        self.audio = None;
        if self.config.0.is_some() {
            let dec = GstStreamDecoder::choice(g, VideoCodec::H264, true)?;
            let desc = format!(
                "appsrc name=src format=time caps=video/x-h264,stream-format=byte-stream,alignment=au \
                 ! h264parse ! {dec} ! videoconvert ! video/x-raw,format=NV12 \
                 ! appsink name=sink sync=false max-buffers=4"
            );
            self.video = Some(Stream::open(g, &desc, true)?);
        }
        if let Some(a) = &self.config.1 {
            let (caps, parse) = match &a.codec {
                AudioCodec::Aac { asc } => {
                    let hex: String = asc.iter().map(|b| format!("{b:02x}")).collect();
                    (
                        format!(
                            "audio/mpeg,mpegversion=(int)4,stream-format=(string)raw,codec_data=(buffer){hex}"
                        ),
                        "aacparse",
                    )
                }
                AudioCodec::Mp3 => {
                    ("audio/mpeg,mpegversion=(int)1,layer=(int)3".to_owned(), "mpegaudioparse")
                }
            };
            let desc = format!(
                "appsrc name=src format=time caps=\"{caps}\" ! {parse} ! decodebin ! audioconvert ! audioresample \
                 ! audio/x-raw,format=F32LE,layout=interleaved,rate={},channels={} \
                 ! appsink name=sink sync=false max-buffers=64",
                a.sample_rate, a.channels
            );
            self.audio = Some(Stream::open(g, &desc, false)?);
        }
        Ok(())
    }
}

impl Elementary for GstElementary {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        self.config = (video.cloned(), audio.cloned());
        self.start()
    }

    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        let g = self.g;
        match unit {
            Unit::Video { pts, dts, data, .. } => self
                .video
                .as_mut()
                .ok_or_else(|| MediaError::Format("video with no video stream".into()))?
                .push(g, &data, pts, dts),
            Unit::Audio { pts, data } => self
                .audio
                .as_mut()
                .ok_or_else(|| MediaError::Format("sound with no sound stream".into()))?
                .push(g, &data, pts, pts),
        }
    }

    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        let g = self.g;
        let since = Instant::now();
        loop {
            for s in [self.video.as_mut(), self.audio.as_mut()].into_iter().flatten() {
                if let Some(out) = s.take(g, Duration::ZERO)? {
                    return Ok(Some(out));
                }
            }
            // Decoding runs on GStreamer's threads: wait for a decoder holding much, or for the
            // end of what a drained one holds, rather than pushing on.
            let ended = self.ended;
            let Some(s) = [self.video.as_mut(), self.audio.as_mut()]
                .into_iter()
                .flatten()
                .find(|s| !s.eos && (ended || s.held >= s.hold))
            else {
                return Ok(None);
            };
            if since.elapsed() > if ended { DRAIN } else { STALL } {
                s.hold *= 2;
                s.eos |= ended;
                continue;
            }
            if let Some(out) = s.take(g, POLL)? {
                return Ok(Some(out));
            }
        }
    }

    fn end(&mut self) -> Result<(), MediaError> {
        for s in [self.video.as_ref(), self.audio.as_ref()].into_iter().flatten() {
            // SAFETY: signalling the end on a live app source.
            unsafe { (self.g.app_src_end_of_stream)(s.pipe.get(0)) };
        }
        self.ended = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        // New pipelines: simpler than flushing events, and a seek is rare.
        self.start()
    }
}

/// [`Fetch`] through GStreamer's URI sources: `<url> ! appsink`.
pub(crate) struct GstFetch {
    g: &'static Gst,
}

impl GstFetch {
    pub(crate) fn new() -> Result<GstFetch, MediaError> {
        Ok(GstFetch { g: gst()? })
    }
}

impl Fetch for GstFetch {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, MediaError> {
        // A local playlist's segments are read from disk: their names joined on unescaped
        // (spaces and all) are no URL GStreamer opens.
        if let Some(path) = file_path(url) {
            return std::fs::read(&path).map_err(|e| MediaError::NotFound(format!("{path}: {e}")));
        }
        let g = self.g;
        if url.contains(char::is_whitespace) || url.contains('!') {
            return Err(MediaError::NotFound(format!("{url}: not a URL GStreamer can open")));
        }
        let p = build(g, &format!("{url} ! appsink name=sink sync=false"), &[c"sink"])?;
        p.start("fetching")?;
        let (mut out, mut last, started) = (Vec::new(), Instant::now(), Instant::now());
        loop {
            // SAFETY: asking a live app sink. Read before the bus: a failed source (an HTTP
            // 404) posts its error, then ends the stream.
            let eos = unsafe { (g.app_sink_is_eos)(p.get(0)) } != 0;
            if let Some((code, text)) = g.bus_error(p.pipeline) {
                return Err(map_error(code, format!("{url}: {text}")));
            }
            if eos {
                return Ok(out);
            }
            match pull(g, p.get(0), POLL * 20) {
                Some((bytes, _, s)) => {
                    // SAFETY: the sample `pull` handed over, released once.
                    unsafe { (g.mini_object_unref)(s) };
                    out.extend_from_slice(&bytes);
                    last = Instant::now();
                }
                // Nothing yet waits for the first byte; a download gone quiet is started again.
                None if last.elapsed() > if out.is_empty() { FIRST_BYTE } else { QUIET } => {
                    return Err(timed_out(url, "nothing arrived"));
                }
                None if started.elapsed() > LOAD => return Err(timed_out(url, "the download took too long")),
                None => {}
            }
        }
    }
}
