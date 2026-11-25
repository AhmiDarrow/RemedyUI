//! The HLS reader driven whole: playlists and segments from the fixtures served from memory,
//! and decoders that turn each unit into a sample at its time.

use super::*;
use crate::MediaLimits;
use crate::host::{Picture, PixelFormat};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const ROOT: &str = "https://test.example/hls/";

/// What the decoders were set up with.
type Setup = (Option<VideoConfig>, Option<AudioConfig>);

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls").join(name);
    std::fs::read(path).unwrap_or_else(|e| unreachable!("{name}: {e}"))
}

/// Serves the fixtures by URL; a playlist with several versions hands them out in turn (the
/// last again and again), as a growing playlist does.
#[derive(Default)]
struct Mem {
    files: HashMap<String, Vec<u8>>,
    versions: HashMap<String, Vec<String>>,
    asked: Arc<Mutex<Vec<String>>>,
    /// URLs that fail this many more times before they are served, and whether as timeouts.
    late: HashMap<String, (u32, bool)>,
    /// URLs answered 404 this many more times before they are served (not yet written).
    absent: HashMap<String, u32>,
    /// After this many fetches, each next one waits for a permit (the test sets the pace).
    gate: Option<(usize, std::sync::mpsc::Receiver<()>)>,
}

impl Mem {
    /// Every segment of the fixture `dir`, and `playlist` (or the fixture's own) as its index.
    fn of(dir: &str, playlist: Option<&str>) -> Self {
        let mut m = Mem::default();
        for i in 0..3 {
            let name = format!("{dir}/seg0000{i}.ts");
            if let Ok(d) = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls").join(&name),
            ) {
                m.files.insert(format!("{ROOT}{name}"), d);
            }
        }
        let index = playlist.map_or_else(|| fixture(&format!("{dir}/index.m3u8")), |p| p.as_bytes().to_vec());
        m.files.insert(format!("{ROOT}{dir}/index.m3u8"), index);
        m
    }
}

impl Fetch for Mem {
    fn get(&mut self, url: &str, _: &std::sync::atomic::AtomicBool) -> Result<Vec<u8>, MediaError> {
        lock(&self.asked).push(url.to_string());
        if let Some((free, permits)) = &self.gate
            && lock(&self.asked).len() > *free
        {
            let _ = permits.recv();
        }
        if let Some((n, timeout)) = self.late.get_mut(url)
            && *n > 0
        {
            *n -= 1;
            return Err(if *timeout {
                timed_out(url, "nothing arrived")
            } else {
                MediaError::NotFound(url.into())
            });
        }
        if let Some(n) = self.absent.get_mut(url)
            && *n > 0
        {
            *n -= 1;
            return Err(missing(url, "the server answered 404"));
        }
        if let Some(v) = self.versions.get_mut(url) {
            let text = if v.len() > 1 { v.remove(0) } else { v[0].clone() };
            // An empty version is the playlist gone (a 404).
            if text.is_empty() {
                return Err(missing(url, "the server answered 404"));
            }
            return Ok(text.into_bytes());
        }
        self.files.get(url).cloned().ok_or_else(|| missing(url, "the server answered 404"))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Decoders that hand each unit straight back as a sample at its time (a 2x2 picture, or one
/// frame of silence), remembering the units.
#[derive(Default)]
struct Echo {
    out: VecDeque<Sample>,
    units: Arc<Mutex<Vec<Unit>>>,
    configured: Arc<Mutex<Option<Setup>>>,
}

impl Elementary for Echo {
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError> {
        *lock(&self.configured) = Some((video.cloned(), audio.cloned()));
        Ok(())
    }
    fn push(&mut self, unit: Unit) -> Result<(), MediaError> {
        self.out.push_back(match &unit {
            Unit::Video { pts, .. } => Sample::Video {
                at: *pts,
                duration: None,
                picture: Picture::packed(2, 2, PixelFormat::Rgba, vec![0u8; 16]),
            },
            Unit::Audio { pts, .. } => Sample::Audio { at: *pts, samples: vec![0.0; 2] },
        });
        lock(&self.units).push(unit);
        Ok(())
    }
    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        Ok(self.out.pop_front())
    }
    fn end(&mut self) -> Result<(), MediaError> {
        Ok(())
    }
    fn flush(&mut self) -> Result<(), MediaError> {
        self.out.clear();
        Ok(())
    }
}

struct Run {
    dec: HlsDecoder,
    units: Arc<Mutex<Vec<Unit>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

thread_local! {
    /// How long this test's clock has waited (each test runs on a thread of its own).
    static WAITED: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// A clock that only moves when waited on: the tests do not wait.
fn fake_now() -> std::time::Instant {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *BASE.get_or_init(std::time::Instant::now) + WAITED.get()
}

/// Lets `d` pass on the tests' clock.
fn pass(d: Duration) {
    WAITED.set(WAITED.get() + d);
}

const CLOCK: Clock = Clock { now: fake_now, sleep: pass };

fn start(dir: &str, mem: Mem) -> Result<Run, MediaError> {
    let es = Echo::default();
    let units = es.units.clone();
    let asked = mem.asked.clone();
    let dec = open_with(&format!("{ROOT}{dir}/index.m3u8"), Box::new(mem), None, || Ok(Box::new(es)), CLOCK)?;
    Ok(Run { dec, units, asked })
}

fn run(dir: &str) -> Run {
    start(dir, Mem::of(dir, None)).unwrap_or_else(|e| unreachable!("{dir} opens: {e}"))
}

/// Every sample to the end: (is video, time).
fn read_all(dec: &mut HlsDecoder) -> Result<Vec<(bool, Duration)>, MediaError> {
    let mut out = Vec::new();
    while let Some(s) = dec.read()? {
        out.push((matches!(s, Sample::Video { .. }), s.at()));
        assert!(out.len() < 10_000, "the reader ends");
    }
    Ok(out)
}

fn ms(t: u64) -> Duration {
    Duration::from_millis(t)
}

/// Video units' times in order, and audio units' times in order, from what was pushed.
fn times(units: &[Unit]) -> (Vec<(Duration, bool)>, Vec<Duration>) {
    let mut v = Vec::new();
    let mut a = Vec::new();
    for u in units {
        match u {
            Unit::Video { dts, keyframe, .. } => v.push((*dts, *keyframe)),
            Unit::Audio { pts, .. } => a.push(*pts),
        }
    }
    (v, a)
}

fn ordered(t: &[Duration]) -> bool {
    t.windows(2).all(|w| w[0] <= w[1])
}

#[test]
fn h264_and_aac_play_from_the_start_to_the_end_in_order() {
    let mut r = run("aac");
    let info = r.dec.info().clone();
    assert_eq!(info.duration, Some(ms(3000)));
    assert!(info.seekable);
    assert_eq!(info.video.as_ref().map(|v| (v.width, v.height)), Some((176, 100)));
    assert_eq!(info.audio, Some(AudioFormat::new(48000, 2)));
    let Ok(samples) = read_all(&mut r.dec) else { unreachable!("plays to the end") };
    let units = lock(&r.units);
    let (video, audio) = times(&units);
    assert_eq!(video.len(), 75, "every picture");
    assert_eq!(video.iter().filter(|(_, k)| *k).count(), 3, "a key frame a segment");
    assert!(video[0].1, "starts on a key frame");
    assert!(ordered(&video.iter().map(|v| v.0).collect::<Vec<_>>()));
    assert!(ordered(&audio));
    // The earliest time in the first segment is zero, whatever the stream's clock said (1.46 s).
    assert_eq!(audio[0], Duration::ZERO);
    let last = samples.iter().map(|s| s.1).max().unwrap_or_default();
    assert!(last > ms(2900) && last < ms(3100), "ends near 3 s: {last:?}");
    // Every key frame carries its parameter sets.
    for u in units.iter() {
        if let Unit::Video { keyframe: true, data, .. } = u {
            assert!(ts::nal_units(data).iter().any(|n| n[0] & 0x1f == 7));
        }
    }
    assert!(r.dec.read().is_ok_and(|s| s.is_none()), "stays at the end");
}

#[test]
fn the_decoders_are_set_up_from_the_first_segment() {
    let es = Echo::default();
    let configured = es.configured.clone();
    let url = format!("{ROOT}aac/index.m3u8");
    let opened = open_with(&url, Box::new(Mem::of("aac", None)), None, || Ok(Box::new(es)), CLOCK);
    assert!(opened.is_ok());
    let Some((Some(v), Some(a))) = lock(&configured).clone() else { unreachable!("both set up") };
    assert!(v.codec.starts_with("avc1.64"));
    assert_eq!(v.sps[0] & 0x1f, 7);
    assert_eq!(v.pps[0] & 0x1f, 8);
    assert_eq!(a.codec, AudioCodec::Aac { asc: vec![0x11, 0x90] });
    assert_eq!(a.codec_string, "mp4a.40.2");
}

#[test]
fn the_next_stream_takes_the_last_ones_decoders() {
    let spare = Spare::default();
    let url = format!("{ROOT}aac/index.m3u8");
    let es = Echo::default();
    let units = es.units.clone();
    let mut d = open_with(&url, Box::new(Mem::of("aac", None)), Some(&spare), || Ok(Box::new(es)), CLOCK)
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(read_all(&mut d).is_ok());
    let read = lock(&units).len();
    assert!(lock(&spare).is_none(), "held while the stream plays");
    drop(d);
    assert!(lock(&spare).is_some(), "handed back on the drop");
    let mut d =
        open_with(&url, Box::new(Mem::of("aac", None)), Some(&spare), || unreachable!("made anew"), CLOCK)
            .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(read_all(&mut d).is_ok());
    assert_eq!(lock(&units).len(), 2 * read, "the same decoders decoded the second stream");
    // Dropped with the host: nothing holds them past it.
    drop(d);
    let weak = std::sync::Arc::downgrade(&spare);
    drop(spare);
    assert!(weak.upgrade().is_none());
}

#[test]
fn h264_and_mp3_play() {
    let mut r = run("mp3");
    assert_eq!(r.dec.info().audio, Some(AudioFormat::new(44100, 2)));
    assert!(read_all(&mut r.dec).is_ok());
    let (video, audio) = times(&lock(&r.units));
    assert_eq!(video.len(), 50);
    assert!(ordered(&audio));
    assert!(audio.last().is_some_and(|t| *t > ms(1900) && *t < ms(2100)));
}

#[test]
fn video_without_sound_plays() {
    let mut r = run("video");
    assert_eq!(r.dec.info().audio, None);
    assert!(read_all(&mut r.dec).is_ok());
    let (video, audio) = times(&lock(&r.units));
    assert_eq!(video.len(), 75);
    assert!(audio.is_empty());
    assert_eq!(video[0].0, Duration::ZERO);
}

#[test]
fn a_clock_far_in_that_wraps_mid_stream_plays_from_zero() {
    let mut r = run("wrap");
    assert!(read_all(&mut r.dec).is_ok());
    let (video, audio) = times(&lock(&r.units));
    assert_eq!(video.len(), 50);
    assert!(ordered(&audio) && ordered(&video.iter().map(|v| v.0).collect::<Vec<_>>()));
    assert_eq!(audio[0], Duration::ZERO);
    assert!(audio.last().is_some_and(|t| *t < ms(2100)), "no jump at the wrap: {:?}", audio.last());
}

#[test]
fn a_seek_starts_at_the_segment_holding_the_target() {
    let mut r = run("aac");
    // Read a little, then go to 1.5 s.
    for _ in 0..10 {
        assert!(r.dec.read().is_ok());
    }
    assert!(r.dec.seek(ms(1500)).is_ok());
    lock(&r.units).clear();
    let Ok(samples) = read_all(&mut r.dec) else { unreachable!("plays on") };
    let first_video = samples.iter().find(|s| s.0).map(|s| s.1);
    assert!(first_video.is_some_and(|t| t >= ms(990) && t <= ms(1100)), "from 1 s: {first_video:?}");
    let (video, _) = times(&lock(&r.units));
    assert!(video[0].1, "a key frame first");
    assert_eq!(video.len(), 50, "the second and third segments");
    assert_eq!(r.dec.buffered(), Some(vec![ms(1000)..ms(3000)]));
}

#[test]
fn a_growing_playlist_is_asked_again_until_it_ends() {
    let head = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:EVENT\n";
    let seg = |i: u32| format!("#EXTINF:1.000000,\nseg0000{i}.ts\n");
    let v1 = format!("{head}{}", seg(0));
    let v2 = format!("{head}{}{}", seg(0), seg(1));
    let v3 = format!("{head}{}{}{}#EXT-X-ENDLIST\n", seg(0), seg(1), seg(2));
    let mut mem = Mem::of("aac", None);
    let url = format!("{ROOT}aac/index.m3u8");
    mem.versions.insert(url.clone(), vec![v1, v2.clone(), v2, v3]);
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    assert_eq!(r.dec.info().duration, Some(ms(1000)));
    let mut lengths = vec![];
    while let Ok(Some(_)) = r.dec.read() {
        lengths.push(r.dec.info().duration);
    }
    assert_eq!(lengths.last(), Some(&Some(ms(3000))), "the length grew to the whole");
    assert!(lengths.contains(&Some(ms(2000))));
    assert_eq!(times(&lock(&r.units)).0.len(), 75, "every picture");
    assert_eq!(lock(&r.asked).iter().filter(|u| **u == url).count(), 4);
}

#[test]
fn a_missing_segment_is_retried_then_errors() {
    let mut mem = Mem::of("aac", None);
    let gone = format!("{ROOT}aac/seg00001.ts");
    mem.files.remove(&gone);
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    let result = read_all(&mut r.dec);
    assert!(matches!(result, Err(MediaError::NotFound(_))), "{result:?}");
    let rounds = (1 + MediaLimits::DEFAULT.recoveries) as usize;
    assert_eq!(
        lock(&r.asked).iter().filter(|u| **u == gone).count(),
        (1 + MediaLimits::DEFAULT.error_retries as usize) * rounds
    );
}

#[test]
fn a_playlist_that_does_not_exist_fails_the_open_after_a_short_grace() {
    let url = format!("{ROOT}aac/index.m3u8");
    let mut mem = Mem::of("aac", None);
    mem.files.remove(&url);
    let asked = mem.asked.clone();
    let result = start("aac", mem).map(|_| ());
    assert!(matches!(&result, Err(MediaError::NotFound(m)) if m.contains("404")), "{result:?}");
    assert_eq!(
        lock(&asked).len(),
        1 + MediaLimits::DEFAULT.missing_retries as usize,
        "asked, then twice again, then failed"
    );
    assert_eq!(
        MediaLimits::DEFAULT.max_retry_pause * MediaLimits::DEFAULT.missing_retries,
        Duration::from_secs(2),
        "the grace is about 2 s"
    );
}

#[test]
fn a_playlist_written_within_the_grace_opens() {
    let url = format!("{ROOT}aac/index.m3u8");
    let mut mem = Mem::of("aac", None);
    mem.absent.insert(url.clone(), MediaLimits::DEFAULT.missing_retries);
    let mut r = start("aac", mem).unwrap_or_else(|e| unreachable!("opens: {e}"));
    assert!(read_all(&mut r.dec).is_ok_and(|s| !s.is_empty()));
}

#[test]
fn a_growing_playlist_that_goes_missing_keeps_its_patient_retries() {
    // Read once, the playlist 404s on each reload after: retried as a late segment is.
    let url = format!("{ROOT}aac/index.m3u8");
    let first = String::from_utf8_lossy(&fixture("aac/index.m3u8")).replace("#EXT-X-ENDLIST", "");
    let mut mem = Mem::of("aac", None);
    mem.versions.insert(url.clone(), vec![first, String::new()]);
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    let result = read_all(&mut r.dec);
    assert!(matches!(result, Err(MediaError::NotFound(_))), "{result:?}");
    let reloads = lock(&r.asked).iter().filter(|u| **u == url).count() - 1;
    assert!(
        reloads
            >= (1 + MediaLimits::DEFAULT.error_retries as usize)
                * (1 + MediaLimits::DEFAULT.recoveries as usize),
        "{reloads} reloads"
    );
}

/// Plays the `aac` fixture with `late` failing as it says; how often each segment was asked.
fn play_late(late: &[(u32, u32, bool)]) -> Result<Vec<usize>, MediaError> {
    let mut mem = Mem::of("aac", None);
    for &(seg, n, timeout) in late {
        mem.late.insert(format!("{ROOT}aac/seg0000{seg}.ts"), (n, timeout));
    }
    let mut r = start("aac", mem)?;
    let samples = read_all(&mut r.dec)?;
    assert!(samples.iter().any(|s| s.1 > ms(2900)), "plays to the end");
    let asked = lock(&r.asked);
    Ok((0..3).map(|i| asked.iter().filter(|u| u.ends_with(&format!("seg0000{i}.ts"))).count()).collect())
}

#[test]
fn a_late_segment_plays_on_once_it_appears() {
    let before = WAITED.get();
    assert_eq!(play_late(&[(1, 10, false)]).ok(), Some(vec![1, 11, 1]));
    // 250, 500, 1000 ms, then 250 after starting over, and so on: 4.75 s.
    let waited = WAITED.get() - before;
    assert!(waited >= ms(4750), "waited between tries: {waited:?}");
}

#[test]
fn a_stalled_fetch_is_started_again() {
    // Four timeouts in a row are retried without starting over; a fifth starts over.
    assert_eq!(play_late(&[(2, 4, true)]).ok(), Some(vec![1, 1, 5]));
    assert_eq!(play_late(&[(2, 30, true)]).ok(), Some(vec![1, 1, 31]));
    assert!(play_late(&[(2, 35, true)]).is_err(), "gives up after 7 rounds of 5");
}

#[test]
fn recoveries_are_earned_back_by_each_segment_read() {
    // Each spot nearly runs out (27 of 28 tries fail), but reading it earns them back.
    assert_eq!(play_late(&[(1, 27, false), (2, 27, false)]).ok(), Some(vec![1, 28, 28]));
    assert!(play_late(&[(1, 28, false)]).is_err());
}

#[test]
fn a_growing_playlist_waits_the_longest_pause_between_tries() {
    let mut mem = Mem::of("aac", None);
    growing(&mut mem);
    let mut r = start("aac", mem).unwrap_or_else(|e| unreachable!("opens: {e}"));
    r.dec.hls.failures = 1;
    assert_eq!(r.dec.hls.wanted().map(|w| w.after), Some(MediaLimits::DEFAULT.max_retry_pause), "growing");
    r.dec.hls.complete = true;
    assert_eq!(r.dec.hls.retry_pause(), MediaLimits::DEFAULT.retry_pause, "complete");
}

#[test]
fn what_is_held_grows_as_segments_arrive() {
    // The playlist and the first segment come at open; each next segment only when let through,
    // so every step is seen (a free fetch thread could bring two between two looks).
    let (permit, permits) = std::sync::mpsc::channel();
    let mut mem = Mem::of("aac", None);
    mem.gate = Some((2, permits));
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    let mut seen = vec![r.dec.buffered().unwrap_or_default()];
    assert_eq!(seen, [vec![ms(0)..ms(1000)]]);
    for _ in 1..3 {
        let _ = permit.send(());
        let asked = std::time::Instant::now();
        loop {
            let b = r.dec.buffered().unwrap_or_default();
            if seen.last() != Some(&b) {
                seen.push(b);
                break;
            }
            assert!(asked.elapsed() < Duration::from_secs(10), "the segment let through arrives");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert_eq!(seen, [vec![ms(0)..ms(1000)], vec![ms(0)..ms(2000)], vec![ms(0)..ms(3000)]]);
    drop(permit);
    assert!(read_all(&mut r.dec).is_ok_and(|s| !s.is_empty()), "and it all reads");
}

#[test]
fn a_discontinuity_starts_the_clock_again() {
    // The first segment of `aac` (clock from 1.46 s), then the first of `wrap` (clock near
    // the 33-bit wrap): the second plays on from 1 s.
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n../aac/seg00000.ts\n\
                #EXT-X-DISCONTINUITY\n#EXTINF:1,\n../wrap/seg00000.ts\n#EXT-X-ENDLIST\n";
    let mut mem = Mem::of("aac", Some(text));
    mem.files.insert(format!("{ROOT}wrap/seg00000.ts"), fixture("wrap/seg00000.ts"));
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    assert!(read_all(&mut r.dec).is_ok());
    let (video, audio) = times(&lock(&r.units));
    assert_eq!(video.len(), 50);
    assert!(ordered(&audio));
    assert!(audio.iter().any(|t| *t == ms(1000)), "the second starts at 1 s");
    assert!(audio.last().is_some_and(|t| *t < ms(2100)));
}

#[test]
fn a_master_playlist_leads_to_its_variant() {
    let mut hls = Hls::new("https://test.example/master.m3u8");
    let Some(w) = hls.wanted() else { unreachable!("the playlist is wanted") };
    let master =
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000,CODECS=\"avc1.64000d,mp4a.40.2\"\nv/index.m3u8\n";
    assert!(hls.deliver(&w, Ok(master.as_bytes().to_vec())).is_ok());
    let w = hls.wanted();
    assert_eq!(
        w.as_ref().map(|w| (w.url.as_str(), w.kind)),
        Some(("https://test.example/v/index.m3u8", WantKind::Playlist))
    );
    let Some(w) = w else { unreachable!() };
    assert!(hls.deliver(&w, Ok(fixture("aac/index.m3u8"))).is_ok());
    let w = hls.wanted();
    assert_eq!(
        w.map(|w| (w.url, w.kind)),
        Some(("https://test.example/v/seg00000.ts".into(), WantKind::Segment(0)))
    );
}

#[test]
fn master_playlists_that_point_at_each_other_fail_and_no_target_still_pauses() {
    let mut hls = Hls::new("https://test.example/master.m3u8");
    let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nmaster.m3u8\n";
    let results: Vec<Result<(), MediaError>> = (0..=MAX_MASTERS)
        .map(|_| {
            let Some(w) = hls.wanted() else { unreachable!("a playlist is wanted") };
            hls.deliver(&w, Ok(master.as_bytes().to_vec()))
        })
        .collect();
    assert!(matches!(results.last(), Some(Err(MediaError::Format(_)))), "{results:?}");
    assert!(results.iter().rev().skip(1).all(Result::is_ok), "{results:?}");
    // A growing playlist that states no target duration is still asked again with a pause.
    let mut hls = Hls::new("https://test.example/index.m3u8");
    let Some(w) = hls.wanted() else { unreachable!("the playlist is wanted") };
    assert!(hls.deliver(&w, Ok(b"#EXTM3U\n".to_vec())).is_ok());
    assert_eq!(hls.wanted().map(|w| (w.kind, w.after)), Some((WantKind::Playlist, MIN_RELOAD)));
}

#[test]
fn only_a_few_segments_are_fetched_ahead() {
    let text = format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:1\n{}#EXT-X-ENDLIST\n",
        (0..10).map(|_| "#EXTINF:1,\nseg00000.ts\n").collect::<String>()
    );
    let mut hls = Hls::new(&format!("{ROOT}aac/index.m3u8"));
    let seg = fixture("aac/seg00000.ts");
    let mut fetched = 0;
    while let Some(w) = hls.wanted() {
        let data = if w.kind == WantKind::Playlist { text.as_bytes().to_vec() } else { seg.clone() };
        fetched += usize::from(w.kind != WantKind::Playlist);
        assert!(hls.deliver(&w, Ok(data)).is_ok());
    }
    assert_eq!(fetched, MediaLimits::DEFAULT.hls_ahead + 1, "the one being read and those ahead");
    assert!(!hls.ended());
    // The app's own bound: more ahead.
    let mut hls = Hls::new(&format!("{ROOT}aac/index.m3u8"));
    hls.limits = MediaLimits::DEFAULT.hls_ahead(6);
    let mut fetched = 0;
    while let Some(w) = hls.wanted() {
        let data = if w.kind == WantKind::Playlist { text.as_bytes().to_vec() } else { seg.clone() };
        fetched += usize::from(w.kind != WantKind::Playlist);
        assert!(hls.deliver(&w, Ok(data)).is_ok());
    }
    assert_eq!(fetched, 7);
}

/// An event playlist of the `aac` fixture's segments that lists one more each version, the
/// last with its end; its URL.
fn growing(mem: &mut Mem) -> String {
    let head = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:EVENT\n";
    let seg = |i: u32| format!("#EXTINF:1.000000,\nseg0000{i}.ts\n");
    let v1 = format!("{head}{}", seg(0));
    let v2 = format!("{head}{}{}", seg(0), seg(1));
    let v3 = format!("{head}{}{}{}#EXT-X-ENDLIST\n", seg(0), seg(1), seg(2));
    let url = format!("{ROOT}aac/index.m3u8");
    mem.versions.insert(url.clone(), vec![v1, v2, v3]);
    url
}

#[test]
fn a_growing_playlist_is_asked_again_while_nothing_is_read() {
    let mut mem = Mem::of("aac", None);
    let url = growing(&mut mem);
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    let length = |r: &mut Run| {
        r.dec.buffered();
        assert!(r.dec.settle().is_ok());
        (r.dec.info().duration, r.dec.info().growing)
    };
    assert_eq!(length(&mut r), (Some(ms(1000)), true), "not due yet: not asked again");
    // Asked again every target duration (half of one near the end), nothing read meanwhile.
    pass(ms(1000));
    assert_eq!(length(&mut r), (Some(ms(2000)), true));
    pass(ms(1000));
    assert_eq!(length(&mut r), (Some(ms(3000)), false), "ended: the length is final");
    pass(ms(5000));
    length(&mut r);
    assert_eq!(lock(&r.asked).iter().filter(|u| **u == url).count(), 3, "not asked once ended");
}

#[test]
fn a_seek_past_the_end_so_far_waits_for_the_playlist_to_reach_it() {
    let mut mem = Mem::of("aac", None);
    growing(&mut mem);
    let Ok(mut r) = start("aac", mem) else { unreachable!("opens") };
    assert!(r.dec.seek(ms(2500)).is_ok());
    // Waiting for the playlist does not block: one slice, then the caller asks again.
    let polled = r.dec.poll_read(&mut Context::from_waker(std::task::Waker::noop()));
    assert!(polled.is_pending());
    lock(&r.units).clear();
    let Ok(samples) = read_all(&mut r.dec) else { unreachable!("plays on") };
    let first_video = samples.iter().find(|s| s.0).map(|s| s.1);
    assert!(first_video.is_some_and(|t| t >= ms(1990) && t <= ms(2100)), "from 2 s: {first_video:?}");
    assert_eq!(times(&lock(&r.units)).0.len(), 25, "only the third segment");
}

#[test]
fn a_seek_past_the_end_of_an_ended_playlist_starts_at_the_last_segment() {
    let mut r = run("aac");
    assert!(!r.dec.info().growing);
    assert!(r.dec.seek(ms(60_000)).is_ok());
    lock(&r.units).clear();
    assert!(read_all(&mut r.dec).is_ok());
    let (video, _) = times(&lock(&r.units));
    assert_eq!(video.len(), 25);
    // Decoding times run a little before the segment's start (pictures reordered).
    assert!(video[0].0 >= ms(1900) && video[0].1, "the third segment's key frame: {:?}", video[0]);
}

#[test]
fn a_growing_playlist_is_asked_again_far_from_its_end() {
    let text = format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n{}",
        (0..10).map(|_| "#EXTINF:4,\nseg00000.ts\n").collect::<String>()
    );
    let mut hls = Hls::new(&format!("{ROOT}aac/index.m3u8"));
    let seg = fixture("aac/seg00000.ts");
    while let Some(w) = hls.wanted().filter(|w| w.kind != WantKind::Playlist || !hls.loaded) {
        let data = if w.kind == WantKind::Playlist { text.as_bytes().to_vec() } else { seg.clone() };
        assert!(hls.deliver(&w, Ok(data)).is_ok());
    }
    // The segments ahead are fetched; the playlist is still asked for, a target duration on.
    let w = hls.wanted();
    assert_eq!(w.map(|w| (w.kind, w.after)), Some((WantKind::Playlist, Duration::from_secs(4))));
}

#[test]
fn hls_is_told_by_the_path() {
    assert!(is_hls(&MediaSource::url("https://h/live/index.m3u8")));
    assert!(is_hls(&MediaSource::url("https://h/live/INDEX.M3U8?token=a.mp4#t=3")));
    assert!(!is_hls(&MediaSource::url("https://h/video.mp4?list=a.m3u8")));
    assert!(is_hls(&MediaSource::file("C:/media/index.m3u8")));
    assert!(!is_hls(&MediaSource::file("clip.mp4")));
}

#[test]
fn file_urls_name_paths() {
    assert_eq!(file_path("https://a/b.ts"), None);
    let p = |u: &str| file_path(u).unwrap_or_default();
    if cfg!(windows) {
        assert_eq!(p("file:///C:/a%20b/seg ü 1.ts?x"), "C:/a b/seg ü 1.ts");
        assert_eq!(p("file://localhost/C:/a.ts"), "C:/a.ts");
        assert_eq!(p("file://server/share/a.ts"), "//server/share/a.ts");
    } else {
        assert_eq!(p("file:///tmp/a%20b/seg ü 1.ts?x"), "/tmp/a b/seg ü 1.ts");
        assert_eq!(p("file://localhost/tmp/a.ts"), "/tmp/a.ts");
    }
}

#[test]
fn a_file_is_read_whole_and_a_missing_one_is_not_found() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls/aac");
    let path = dir.join("index.m3u8").to_string_lossy().into_owned();
    assert_eq!(read_file(&path).ok(), Some(fixture("aac/index.m3u8")));
    let gone = dir.join("no-such.ts").to_string_lossy().into_owned();
    assert!(matches!(read_file(&gone), Err(MediaError::NotFound(_))));
}

/// How long reading a real segment takes: `REMEDY_HLS_BENCH=<a .ts file> cargo test --release
/// -p remedy-ui-media-playback --lib reading_a_big_segment -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: needs REMEDY_HLS_BENCH"]
fn reading_a_big_segment() {
    let Ok(path) = std::env::var("REMEDY_HLS_BENCH") else { return };
    let seg = std::fs::read(&path).unwrap_or_else(|e| unreachable!("{path}: {e}"));
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nseg.ts\n#EXT-X-ENDLIST\n";
    let rounds = 50;
    let t = std::time::Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(ts::demux(&seg));
    }
    let demux = t.elapsed() / rounds;
    let t = std::time::Instant::now();
    let mut units = 0;
    for _ in 0..rounds {
        let mut hls = Hls::new(&format!("{ROOT}index.m3u8"));
        let p = Want { url: format!("{ROOT}index.m3u8"), kind: WantKind::Playlist, after: Duration::ZERO };
        assert!(hls.deliver(&p, Ok(text.as_bytes().to_vec())).is_ok());
        let Some(w) = hls.wanted() else { unreachable!("the segment") };
        assert!(hls.deliver(&w, Ok(seg.clone())).is_ok());
        while hls.next_unit().is_some() {
            units += 1;
        }
    }
    let read = t.elapsed() / rounds;
    eprintln!("{} bytes: demux {demux:?}, read whole {read:?}, {} units", seg.len(), units / rounds);
}

/// A few thousand damaged segments and playlists (bytes flipped, cut, spliced, made up) are
/// read without panicking or overflowing (the tests are built with overflow checks).
#[test]
fn damaged_input_is_read_without_panicking() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rand = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };
    let segs: Vec<Vec<u8>> = ["aac", "mp3", "video", "wrap", "surround", "mono24"]
        .iter()
        .map(|d| fixture(&format!("{d}/seg00000.ts")))
        .collect();
    let index = Want { url: format!("{ROOT}index.m3u8"), kind: WantKind::Playlist, after: Duration::ZERO };
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\na.ts\n#EXTINF:1,\nb.ts\n#EXT-X-ENDLIST\n";
    let tags = [
        "#EXTM3U",
        "#EXTINF:",
        "#EXT-X-MEDIA-SEQUENCE:",
        "#EXT-X-TARGETDURATION:",
        "#EXT-X-ENDLIST",
        "#EXT-X-DISCONTINUITY",
        "#EXT-X-STREAM-INF:BANDWIDTH=",
        "a.ts",
        "//h/x.m3u8",
        "1e19,",
        "-1",
        "18446744073709551615",
        "NaN",
        "\u{feff}",
        "=\"",
        ",",
    ];
    for round in 0..3000 {
        let mut seg = segs[round % segs.len()].clone();
        match rand(4) {
            0 => seg.truncate(rand(seg.len())),
            1 => {
                for _ in 0..1 + rand(64) {
                    let i = rand(seg.len());
                    seg[i] = rand(256) as u8;
                }
            }
            2 => {
                let other = &segs[rand(segs.len())];
                let (a, b) = (rand(seg.len()), rand(other.len()));
                seg.truncate(a);
                seg.extend_from_slice(&other[b..]);
            }
            _ => seg = (0..rand(4000)).map(|i| if i % 188 == 0 { 0x47 } else { rand(256) as u8 }).collect(),
        }
        let mut hls = Hls::new(&index.url);
        assert!(hls.deliver(&index, Ok(text.as_bytes().to_vec())).is_ok());
        for _ in 0..2 {
            let Some(w) = hls.wanted() else { break };
            let _ = hls.deliver(&w, Ok(seg.clone()));
        }
        while hls.next_unit().is_some() {}
        assert_eq!(hls.queued, 0);
        // Playlists: tags and numbers, shuffled and run together.
        let mut p = String::from("#EXTM3U\n");
        for _ in 0..rand(40) {
            p.push_str(tags[rand(tags.len())]);
            if rand(2) == 0 {
                p.push('\n');
            }
        }
        let mut hls = Hls::new(&index.url);
        let _ = hls.deliver(&index, Ok(p.into_bytes()));
        let _ = (hls.wanted(), hls.info(), hls.buffered());
        hls.seek(Duration::from_secs(rand(1 << 20) as u64));
        let _ = hls.wanted();
    }
}

/// No more is fetched ahead once the units held reach `hls_max_held` bytes, but the segment to
/// read next is always fetched.
#[test]
fn what_is_fetched_ahead_is_bounded_in_bytes() {
    let text =
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n{}#EXT-X-ENDLIST\n", "#EXTINF:1,\na.ts\n".repeat(8));
    let mut hls = Hls::new(&format!("{ROOT}index.m3u8"));
    let index = Want { url: format!("{ROOT}index.m3u8"), kind: WantKind::Playlist, after: Duration::ZERO };
    assert!(hls.deliver(&index, Ok(text.into_bytes())).is_ok());
    let seg = fixture("aac/seg00000.ts");
    let Some(w) = hls.wanted() else { unreachable!("the first segment") };
    assert!(hls.deliver(&w, Ok(seg.clone())).is_ok());
    assert!(hls.queued > 0 && hls.queued < seg.len(), "{} bytes held", hls.queued);
    assert!(hls.wanted().is_some(), "a few segments ahead");
    // As if the segment were 60 MB.
    hls.queued += hls.limits.hls_max_held;
    assert_eq!(hls.wanted(), None, "enough bytes held");
    while hls.next_unit().is_some() {}
    assert!(hls.wanted().is_some(), "nothing held: the next is fetched");
}

/// A live playlist's window slides: what was played and is no longer listed is forgotten
/// (indices re-based, media times kept), what was never fetched and is gone is skipped, an
/// older copy changes nothing, and a server numbering from the start again is read on.
#[test]
fn a_live_playlist_slides_and_a_restarted_one_is_read_on() {
    let index = Want { url: format!("{ROOT}index.m3u8"), kind: WantKind::Playlist, after: Duration::ZERO };
    let live = |from: u64, count: u64, name: &str| {
        let segs: String = (from..from + count).map(|i| format!("#EXTINF:1,\n{name}{i}.ts\n")).collect();
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{from}\n{segs}").into_bytes()
    };
    let seqs = |h: &Hls| h.segments.iter().map(|s| s.sequence).collect::<Vec<_>>();
    let mut hls = Hls::new(&index.url);
    assert!(hls.deliver(&index, Ok(live(0, 4, "a"))).is_ok());
    // As if three segments were read and the third were playing.
    (hls.next, hls.current) = (3, 2);
    assert!(hls.deliver(&index, Ok(live(2, 4, "a"))).is_ok());
    assert_eq!(seqs(&hls), [2, 3, 4, 5]);
    assert_eq!((hls.next, hls.current), (1, 0));
    assert_eq!(hls.segments[0].start, ms(2000), "media times stay");
    assert_eq!(hls.info().duration, Some(ms(6000)));
    assert!(hls.deliver(&index, Ok(live(1, 4, "a"))).is_ok());
    assert_eq!(seqs(&hls), [2, 3, 4, 5], "an older copy of the playlist");
    // The reader fell behind the window: what it never fetched cannot be fetched any more.
    assert!(hls.deliver(&index, Ok(live(10, 3, "a"))).is_ok());
    assert_eq!(seqs(&hls), [2, 10, 11, 12]);
    assert_eq!(hls.wanted().map(|w| w.kind), Some(WantKind::Segment(10)));
    assert!(hls.deliver(&index, Ok(live(0, 2, "b"))).is_ok());
    assert_eq!(seqs(&hls), [2, 0, 1]);
    assert!(hls.segments[1].discontinuity && !hls.segments[2].discontinuity);
    assert_eq!(hls.wanted().map(|w| w.url), Some(format!("{ROOT}b0.ts")));
    // A day of a stream holds no more than its window and what plays.
    for from in 1..2000 {
        hls.next = hls.segments.len();
        hls.current = hls.next - 1;
        assert!(hls.deliver(&index, Ok(live(from, 3, "b"))).is_ok());
    }
    assert!(hls.segments.len() <= 4, "{} segments held", hls.segments.len());
}
