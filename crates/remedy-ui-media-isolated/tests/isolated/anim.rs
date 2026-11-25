//! Animations in the helper: the same frames as in the app, and hostile animated decoders
//! (registered as an animated format before `helper_main`) that panic, loop or allocate
//! without end part of the way through.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_core::Size;
use remedy_ui_media_anim::route::{self, Animation, FrameSource, Loops, Opened, open_here, opener};
use remedy_ui_media_image::{DecodeError, Fit, Source, fitted_scalable};
use remedy_ui_media_isolated::stats;
use remedy_ui_paint::Image;
use remedy_ui_view::Request;

use super::{semantics, ui};

/// `HOSTANIM:<what>:<frame>`: four solid frames of 8×8, 20 ms each, looping; on frame
/// `<frame>` it does `<what>` (`panic`, `loop`, `alloc`; anything else plays normally).
pub(crate) struct HostileAnimation;

const MAGIC: &[u8] = b"HOSTANIM:";

pub(crate) fn hostile(what: &str, at: u64) -> Vec<u8> {
    [MAGIC, format!("{what}:{at}").as_bytes()].concat()
}

struct HostileFrames {
    what: String,
    at: u64,
    n: u64,
    animation: Animation,
}

impl route::Format for HostileAnimation {
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(MAGIC)
    }

    fn dimensions(&self, _bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
        Ok((8, 8))
    }

    fn open(
        &self,
        bytes: &Arc<[u8]>,
        box_px: (u32, u32),
        cover: bool,
    ) -> Result<Box<dyn FrameSource>, DecodeError> {
        let text = String::from_utf8_lossy(bytes.get(MAGIC.len()..).unwrap_or_default()).into_owned();
        let (what, at) = text.split_once(':').unwrap_or((&text, "0"));
        Ok(Box::new(HostileFrames {
            what: what.to_owned(),
            at: at.parse().unwrap_or(0),
            n: 0,
            animation: Animation {
                size: (8, 8),
                show_at: fitted_scalable((8, 8), box_px, cover, false),
                frames: Some(4),
                loops: Loops::Forever,
            },
        }))
    }
}

impl FrameSource for HostileFrames {
    fn animation(&self) -> &Animation {
        &self.animation
    }

    #[allow(clippy::panic)]
    fn next(&mut self) -> Result<Option<(Image, Duration)>, DecodeError> {
        if self.n == self.at {
            match self.what.as_str() {
                "panic" => panic!("a hostile animation panics on frame {}", self.n),
                "loop" => loop {
                    black_box(&self.n);
                },
                "alloc" => {
                    let mut hoard: Vec<Vec<u8>> = Vec::new();
                    loop {
                        hoard.push(vec![1u8; 16 << 20]);
                        black_box(&hoard);
                    }
                }
                _ => {}
            }
        }
        if self.n >= 4 {
            return Ok(None);
        }
        let (w, h) = self.animation.show_at;
        let shade = 60 * self.n as u8;
        let px: Vec<u8> = (0..w * h).flat_map(|_| [shade, 0, 0, 255]).collect();
        self.n += 1;
        Ok(Some((Image::new(w, h, Arc::from(px)), Duration::from_millis(20))))
    }

    fn restart(&mut self) -> Result<(), DecodeError> {
        self.n = 0;
        Ok(())
    }
}

/// A looping GIF of `colors.len()` solid 8×8 frames, `centis` hundredths of a second each.
pub(crate) fn gif(colors: &[[u8; 3]], centis: u16) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let Ok(mut e) = gif::Encoder::new(&mut out, 8, 8, &[]) else { return Vec::new() };
        let _ = e.set_repeat(gif::Repeat::Infinite);
        for c in colors {
            let mut px: Vec<u8> = (0..64).flat_map(|_| [c[0], c[1], c[2], 255]).collect();
            let mut f = gif::Frame::from_rgba_speed(8, 8, &mut px, 10);
            f.delay = centis;
            let _ = e.write_frame(&f);
        }
    }
    out
}

const COLORS: [[u8; 3]; 3] = [[200, 10, 10], [10, 200, 10], [10, 10, 200]];

fn open_remote(bytes: &[u8]) -> Box<dyn FrameSource> {
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let Some(o) = opener() else { unreachable!("isolation sets an opener") };
    match o.open(&bytes, (8, 8), false) {
        Ok(Opened::Animated(s)) => s,
        other => unreachable!("{other:?}"),
    }
}

fn frame(s: &mut Box<dyn FrameSource>) -> Result<Option<(Vec<u8>, Duration)>, DecodeError> {
    s.next().map(|f| f.map(|(img, d)| (img.pixels.to_vec(), d)))
}

/// A good animation opened in a helper.
pub(crate) fn playing() -> Box<dyn FrameSource> {
    open_remote(&gif(&COLORS, 3))
}

/// The pixels of its next frame.
pub(crate) fn next_pixels(s: &mut Box<dyn FrameSource>) -> Option<Vec<u8>> {
    frame(s).ok().flatten().map(|f| f.0)
}

fn stopped<T: std::fmt::Debug>(r: Result<T, DecodeError>) -> String {
    match r {
        Err(DecodeError::Stopped(why)) => why,
        other => unreachable!("expected the helper to be stopped, got {other:?}"),
    }
}

pub(crate) fn animations_decode_in_the_helper_as_in_the_app() {
    let bytes = gif(&COLORS, 3);
    let shared: Arc<[u8]> = Arc::from(&bytes[..]);
    let Ok(Opened::Animated(mut here)) = open_here(&shared, (8, 8), false) else { unreachable!() };
    let before = stats().requests;
    let mut there = open_remote(&bytes);
    assert_eq!(there.animation(), here.animation());
    assert_eq!(there.animation().loops, Loops::Forever);
    // Two plays: the frames, the end of the play, and starting over.
    for _ in 0..2 {
        for _ in 0..COLORS.len() {
            let (a, b) = (frame(&mut here), frame(&mut there));
            assert!(matches!(&a, Ok(Some(_))), "{a:?}");
            assert_eq!(a.ok(), b.ok(), "the same pixels and delay");
        }
        assert!(matches!(frame(&mut there), Ok(None)));
        assert!(matches!(frame(&mut here), Ok(None)));
        assert!(there.restart().is_ok() && here.restart().is_ok());
    }
    assert!(stats().requests > before + 8, "frames came from the helper");
    // A still opens as a still.
    let Some(o) = opener() else { unreachable!() };
    assert!(matches!(o.open(&Arc::from(super::png(4, 4)), (8, 8), false), Ok(Opened::Still)));
}

/// Opens two good animations and a hostile one doing `what` on its second frame. All three
/// share a helper (nothing else is decoding, so the first helper is free each time). The
/// hostile one fails and stays failed; the good ones open again on a fresh helper and carry on
/// from where they were.
fn a_hostile_animation_fails_alone(what: &str) -> String {
    let s = stats();
    let good = gif(&COLORS, 3);
    let mut a = open_remote(&good);
    let mut b = open_remote(&good);
    let mut bad = open_remote(&hostile(what, 1));
    let first = frame(&mut a).ok().flatten().map(|f| f.0);
    assert!(first.is_some());
    assert!(frame(&mut b).is_ok_and(|f| f.is_some()));
    assert!(frame(&mut bad).is_ok_and(|f| f.is_some()), "the hostile animation's first frame is fine");
    let t = Instant::now();
    let why = stopped(frame(&mut bad));
    println!("    {why} (after {:.2?})", t.elapsed());
    // It stays failed, and is not given another helper.
    assert!(matches!(bad.restart(), Err(DecodeError::Stopped(_))));
    assert!(matches!(bad.next(), Err(DecodeError::Stopped(_))));
    // The good ones go on from their second frame, on a fresh helper.
    let Ok(Opened::Animated(mut here)) = open_here(&Arc::from(&good[..]), (8, 8), false) else {
        unreachable!()
    };
    let _ = frame(&mut here);
    let want = frame(&mut here).ok().flatten();
    assert!(want.is_some());
    assert_eq!(frame(&mut a).ok().flatten(), want, "it carried on where it was");
    assert_eq!(frame(&mut b).ok().flatten(), want);
    assert!(stats().reopened >= s.reopened + 2, "both reopened: {:?}", stats());
    drop((a, b, bad));
    why
}

pub(crate) fn a_panicking_animation_fails_alone_and_its_neighbours_reopen() {
    let crashed = stats().crashed;
    let why = a_hostile_animation_fails_alone("panic");
    assert!(why.contains("crashed") && why.contains("panicked"), "{why}");
    assert_eq!(stats().crashed, crashed + 1);
}

pub(crate) fn a_looping_animation_is_stopped_after_its_frames_time() {
    let timed_out = stats().timed_out;
    let why = a_hostile_animation_fails_alone("loop");
    assert!(why.contains("worked past") && why.contains("frame of a 8×8"), "{why}");
    assert_eq!(stats().timed_out, timed_out + 1);
}

pub(crate) fn an_animation_allocating_without_end_is_stopped_at_its_frames_memory() {
    let over = stats().over_memory;
    let why = a_hostile_animation_fails_alone("alloc");
    assert!(why.contains("memory"), "{why}");
    assert_eq!(stats().over_memory, over + 1);
}

pub(crate) fn a_hostile_animation_shows_its_error_state_while_a_good_one_plays() {
    let requests = stats().requests;
    let ui = ui();
    let bad = remedy_ui_media_anim::animated(
        &ui,
        Source::Bytes(Arc::from(hostile("panic", 0))),
        "A trap",
        Fit::Contain,
    )
    .width(40.0)
    .height(40.0);
    let good = remedy_ui_media_anim::animated(
        &ui,
        Source::Bytes(Arc::from(gif(&COLORS, 3))),
        "Lights",
        Fit::Contain,
    )
    .width(40.0)
    .height(40.0);
    ui.root().child(ui.el("row").row().child(bad.clone()).child(good.clone()));

    let mut timers: Vec<u64> = Vec::new();
    let pump = |ui: &remedy_ui_view::Ui, timers: &mut Vec<u64>| {
        let _ = remedy_ui_core::reactive::run_posted();
        let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
        for r in ui.take_requests() {
            match r {
                Request::Timer { id, .. } => timers.push(id),
                Request::CancelTimer(id) => timers.retain(|t| *t != id),
                _ => {}
            }
        }
    };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30)
        && !(semantics(&bad).contains("Not shown") && !timers.is_empty())
    {
        pump(&ui, &mut timers);
        std::thread::sleep(Duration::from_millis(5));
    }
    let s = semantics(&bad);
    println!("    {s}");
    assert!(s.contains("Not shown") && s.contains("decoder was stopped"), "{s}");
    assert!(!semantics(&good).contains("Not shown"), "{}", semantics(&good));
    // The good one plays: each timer it asks for shows the next frame and asks for another
    // (a short animation is kept whole, so its later loops need no decoding).
    for _ in 0..4 {
        let Some(id) = timers.pop() else { unreachable!("the good animation asked for no timer") };
        ui.fire_timer(id);
        let t = Instant::now();
        while timers.is_empty() && t.elapsed() < Duration::from_secs(30) {
            pump(&ui, &mut timers);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert!(!timers.is_empty(), "it went on playing");
    assert!(stats().requests > requests + 4, "its frames came from a helper");
    ui.root().clear();
    pump(&ui, &mut timers);
}
