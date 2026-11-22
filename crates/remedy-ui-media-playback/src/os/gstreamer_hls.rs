//! Linux: HLS through the crate's own reader (`crate::hls`), as on Windows and in a page, so a
//! stream starts at zero, grows and seeks the same everywhere. GStreamer decodes the compressed
//! streams (`appsrc ! h264parse ! <the H.264 decoder the stream decoder chose> ! appsink`, and
//! `appsrc ! aacparse|mpegaudioparse ! decodebin ! appsink`) and fetches the playlists and
//! segments through its URI sources (`souphttpsrc` for http and https, `filesrc` for files).

use std::ffi::CStr;
use std::time::{Duration, Instant};

use super::gstreamer::{
    CLOCK_TIME_NONE, GST_MAP_READ, GST_MAP_WRITE, Gst, GstMapInfo, GstStreamDecoder, RESOURCE_NOT_FOUND, gst,
    launch_error, map_error, nv12_picture,
};
use super::gstreamer_stream::{Pipe, pull, pull_sample};
use crate::hls::{
    AudioCodec, AudioConfig, Elementary, FIRST_BYTE, Fetch, LOAD, MAX_FETCH, QUIET, Unit, VideoConfig,
    file_path, missing, timed_out,
};
use crate::host::{MediaError, Sample, VideoCodec};

const PURPOSE: &str = "playing HLS streams";
/// Units a decoder may hold before `pull` waits for it, until its depth is known.
const HOLD: u32 = 8;
/// Up to how many units a decoder that has given nothing out yet is handed without waiting:
/// past an H.264 decoder's most (16 for reordering, 16 more for frame threads).
const FILL: u32 = 64;
/// How long `pull` waits on a decoder holding `hold` units with nothing out before raising
/// its `hold` (a decoder deeper than `FILL`, or one far behind).
const STALL: Duration = Duration::from_millis(250);
/// How long a drained decoder may take to give out the end of what it held.
const DRAIN: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(5);

thread_local! {
    /// Debug builds: the `STALL` waits `pull` gave up on, on this thread (it pulls on the
    /// reader's), so a test can tell a decoder that is only filling is never waited on.
    static STALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

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
    /// Units it may hold before `pull` waits for it: [`HOLD`], then past its depth.
    hold: u32,
    /// The units it held when its first output came: how many it keeps back. Learned once;
    /// a flush keeps the decoder, so its depth too.
    depth: Option<u32>,
    eos: bool,
}

impl Stream {
    fn open(g: &'static Gst, desc: &str, video: bool) -> Result<Stream, MediaError> {
        let pipe = build(g, desc, &[c"src", c"sink"])?;
        pipe.start("the HLS decoder")?;
        Ok(Stream { pipe, video, held: 0, hold: HOLD, depth: None, eos: false })
    }

    /// Emptied for a new stream or a seek, its elements (the decoder's among them) and
    /// what they negotiated kept: a flush through the app source drops what the pipeline
    /// holds and the last stream's end, and its streaming starts again with a new segment.
    fn reset(&mut self, g: &Gst) -> Result<(), MediaError> {
        check(g, &self.pipe)?;
        let src = self.pipe.get(0);
        // SAFETY: flush events made here and handed to the live app source, which takes them.
        let sent = unsafe {
            (g.element_send_event)(src, (g.event_new_flush_start)()) != 0
                && (g.element_send_event)(src, (g.event_new_flush_stop)(1)) != 0
        };
        if !sent {
            return Err(MediaError::Decode("the HLS decoder would not flush".into()));
        }
        (self.held, self.eos) = (0, false);
        Ok(())
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
        let Some(s) = pull_sample(g, sink, wait) else {
            // SAFETY: asking a live app sink.
            self.eos |= unsafe { (g.app_sink_is_eos)(sink) } != 0;
            return Ok(None);
        };
        if self.depth.is_none() {
            self.depth = Some(self.held);
            self.hold = self.held + HOLD;
        }
        self.held = self.held.saturating_sub(1);
        // SAFETY: the sample `pull_sample` handed over, its buffer mapped for reading while its
        // bytes are copied once into the picture or the sound, then released once.
        unsafe {
            let buf = (g.sample_get_buffer)(s);
            let mut map = GstMapInfo::default();
            if buf.is_null() || (g.buffer_map)(buf, &mut map, GST_MAP_READ) == 0 {
                (g.mini_object_unref)(s);
                return Err(MediaError::Decode("a decoded HLS sample could not be read".into()));
            }
            let (pts, duration) = ((*buf).pts, (*buf).duration);
            let bytes: &[u8] =
                if map.data.is_null() { &[] } else { std::slice::from_raw_parts(map.data, map.size) };
            let at = if pts == CLOCK_TIME_NONE { Duration::ZERO } else { Duration::from_nanos(pts) };
            let out = if self.video {
                nv12_picture(g, s, bytes).map(|picture| {
                    let duration = (duration != CLOCK_TIME_NONE).then(|| Duration::from_nanos(duration));
                    Some(Sample::Video { at, duration, picture })
                })
            } else {
                let samples: Vec<f32> =
                    bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
                Ok((!samples.is_empty()).then_some(Sample::Audio { at, samples }))
            };
            (g.buffer_unmap)(buf, &mut map);
            (g.mini_object_unref)(s);
            out
        }
    }
}

/// [`Elementary`] on GStreamer.
pub(crate) struct GstElementary {
    g: &'static Gst,
    /// The H.264 decoder to use, when not the best this machine has (tests).
    h264: Option<&'static str>,
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
        Ok(GstElementary {
            g: gst()?,
            h264: None,
            config: (None, None),
            video: None,
            audio: None,
            ended: false,
        })
    }

    /// The pipelines for the configured streams: those there emptied and started again (new
    /// if that fails), the others new.
    fn start(&mut self) -> Result<(), MediaError> {
        let g = self.g;
        self.ended = false;
        for s in [&mut self.video, &mut self.audio] {
            if s.as_mut().is_some_and(|s| s.reset(g).is_err()) {
                *s = None;
            }
        }
        if self.config.0.is_some() && self.video.is_none() {
            let dec = match self.h264 {
                Some(name) => name,
                None => GstStreamDecoder::choice(g, VideoCodec::H264, true)?,
            };
            let desc = format!(
                "appsrc name=src format=time caps=video/x-h264,stream-format=byte-stream,alignment=au \
                 ! h264parse ! {dec} ! videoconvert ! video/x-raw,format=NV12 \
                 ! appsink name=sink sync=false max-buffers=4"
            );
            self.video = Some(Stream::open(g, &desc, true)?);
        }
        if let Some(a) = self.config.1.as_ref().filter(|_| self.audio.is_none()) {
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
        // The video pipeline is the same for every H.264 stream (h264parse takes new
        // parameter sets in-band, and a new size); the sound's is set up from its config.
        if video.is_none() {
            self.video = None;
        }
        if self.config.1.as_ref() != audio {
            self.audio = None;
        }
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
            // Nothing out yet: it is filling (a frame-threaded decoder holds a picture a
            // thread, avdec_h264 up to 16, besides reordering), not busy. More at once.
            if !ended && s.depth.is_none() && s.hold < FILL {
                s.hold *= 2;
                return Ok(None);
            }
            if since.elapsed() > if ended { DRAIN } else { STALL } {
                if cfg!(debug_assertions) && !ended {
                    STALLS.set(STALLS.get() + 1);
                }
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
    fn get(&mut self, url: &str, stop: &std::sync::atomic::AtomicBool) -> Result<Vec<u8>, MediaError> {
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
                // souphttpsrc says an HTTP 404 or 410 is a resource not found.
                if code == RESOURCE_NOT_FOUND {
                    return Err(missing(url, &text));
                }
                return Err(map_error(code, format!("{url}: {text}")));
            }
            if eos {
                return Ok(out);
            }
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(MediaError::Decode(format!("{url}: the reader has gone")));
            }
            match pull(g, p.get(0), POLL * 20) {
                Some((bytes, _, s)) => {
                    // SAFETY: the sample `pull` handed over, released once.
                    unsafe { (g.mini_object_unref)(s) };
                    out.extend_from_slice(&bytes);
                    if out.len() > MAX_FETCH {
                        return Err(MediaError::Format(format!("{url}: over {MAX_FETCH} bytes")));
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Decoder;

    /// The fixture stream (H.264, three segments of 25 pictures) on `h264`, through the crate's
    /// HLS reader; `None` when that decoder will not start here.
    fn open(h264: &'static str) -> Option<Box<dyn Decoder>> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls/video/index.m3u8");
        let url = crate::host::MediaSource::file(path).to_url();
        let make = || -> Result<Box<dyn Elementary>, MediaError> {
            let mut es = GstElementary::new()?;
            es.h264 = Some(h264);
            Ok(Box::new(es))
        };
        match crate::hls::open(&url, Box::new(GstFetch::new().ok()?), &crate::hls::Spare::default(), make) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("skipped {h264}: {e}");
                None
            }
        }
    }

    /// The first picture at or past `from`.
    fn first(d: &mut dyn Decoder, from: Duration) -> Duration {
        loop {
            match d.read().unwrap_or_else(|e| unreachable!("reading: {e}")) {
                Some(Sample::Video { at, .. }) if at + Duration::from_millis(41) > from => return at,
                Some(_) => {}
                None => unreachable!("the end before a picture at {from:?}"),
            }
        }
    }

    /// A decoder only filling (nothing out yet: on opening, after each seek) is handed more at
    /// once, never waited on: no `STALL` on opening and seeking around, whatever its depth
    /// (`avdec_h264` holds one picture a frame thread, `nvh264dec` a few).
    #[test]
    fn a_filling_decoder_is_never_waited_on() {
        if !cfg!(debug_assertions) {
            return;
        }
        let mut tried = 0;
        for h264 in ["avdec_h264", "nvh264dec"] {
            let Some(mut d) = open(h264) else { continue };
            tried += 1;
            let before = STALLS.get();
            assert_eq!(first(d.as_mut(), Duration::ZERO), Duration::ZERO);
            for to in [1200, 300, 2700, 0, 2000] {
                let to = Duration::from_millis(to);
                d.seek(to).unwrap_or_else(|e| unreachable!("seeking: {e}"));
                let at = first(d.as_mut(), to);
                assert!(
                    at.abs_diff(to) <= Duration::from_millis(40),
                    "{h264}: a picture at {at:?} for {to:?}"
                );
            }
            assert_eq!(STALLS.get() - before, 0, "{h264} was waited on while only filling");
        }
        if tried == 0 {
            eprintln!("skipped: neither avdec_h264 nor nvh264dec starts here");
        }
    }
}
