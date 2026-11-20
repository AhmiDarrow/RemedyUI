//! The animated image element: timing, loops, pausing, reduced motion, sharing, memory and
//! accessibility, with files made in the tests.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use remedy_ui_input::Key;
use remedy_ui_layout::{Axis, Display, Length};
use remedy_ui_media_anim::{Fit, Source, animated, cached, resident_frames, trim_memory};
use remedy_ui_view::El;

fn src(bytes: Vec<u8>) -> Source {
    Source::Bytes(Arc::from(bytes))
}

fn simple(s: &Screen, bytes: Vec<u8>, alt: &str) -> El {
    animated(&s.ui, src(bytes), alt, Fit::Contain).width(40.0).height(40.0)
}

/// Two frames, red then green, as each format.
fn two_frame_files(loops: u32) -> Vec<(&'static str, Vec<u8>)> {
    let gif_repeat = match loops {
        0 => Some(gif::Repeat::Infinite),
        1 => None,
        n => Some(gif::Repeat::Finite((n - 1) as u16)),
    };
    vec![
        ("gif", gif(4, 4, gif_repeat, &[GifFrame::solid(4, 4, RED, 10), GifFrame::solid(4, 4, GREEN, 10)])),
        ("apng", apng(4, 4, loops, None, &[ApngFrame::solid(4, 4, R, 100), ApngFrame::solid(4, 4, G, 100)])),
        ("webp", webp(4, 4, loops as u16, &[WebpFrame::solid(4, 4, R, 100), WebpFrame::solid(4, 4, G, 100)])),
    ]
}

#[test]
fn each_frame_has_a_timer_of_its_own_length_and_the_animation_loops() {
    for (name, bytes) in [
        (
            "gif",
            gif(
                4,
                4,
                Some(gif::Repeat::Infinite),
                &[
                    GifFrame::solid(4, 4, RED, 5),
                    GifFrame::solid(4, 4, GREEN, 10),
                    GifFrame::solid(4, 4, BLUE, 30),
                ],
            ),
        ),
        (
            "apng",
            apng(
                4,
                4,
                0,
                None,
                &[
                    ApngFrame::solid(4, 4, R, 50),
                    ApngFrame::solid(4, 4, G, 100),
                    ApngFrame::solid(4, 4, B, 300),
                ],
            ),
        ),
        (
            "webp",
            webp(
                4,
                4,
                0,
                &[
                    WebpFrame::solid(4, 4, R, 50),
                    WebpFrame::solid(4, 4, G, 100),
                    WebpFrame::solid(4, 4, B, 300),
                ],
            ),
        ),
    ] {
        let mut s = Screen::new();
        let el = simple(&s, bytes, "Colors");
        s.ui.root().child(el);
        s.frame();
        s.wait("the first frame and its timer", |s| s.color() == R && s.timers.len() == 1);
        assert!(about(s.delay_ms(), 50), "{name}: {}", s.delay_ms());
        let mut seen = vec![s.color()];
        let mut delays = vec![s.delay_ms()];
        for _ in 0..4 {
            let was = s.color();
            s.fire();
            s.wait("the next frame and its timer", |s| s.timers.len() == 1 && s.color() != was);
            seen.push(s.color());
            delays.push(s.delay_ms());
        }
        assert_eq!(seen, vec![R, G, B, R, G], "{name}");
        assert!(all_about(&delays, &[50, 100, 300, 50, 100]), "{name}: {delays:?}");
        // One timer at a time, and nothing else asked for in between.
        assert_eq!(s.asked, 5, "{name}");
    }
}

#[test]
fn gif_delays_under_20_ms_play_as_100_ms() {
    let mut s = Screen::new();
    let frames: Vec<GifFrame> = [0u16, 1, 2, 3].iter().map(|d| GifFrame::solid(4, 4, RED, *d)).collect();
    let el = simple(&s, gif(4, 4, Some(gif::Repeat::Infinite), &frames), "Fast");
    s.ui.root().child(el);
    s.frame();
    s.wait("the first timer", |s| s.timers.len() == 1);
    let mut got = vec![s.delay_ms()];
    for _ in 0..3 {
        s.fire();
        s.wait("the next timer", |s| s.timers.len() == 1);
        got.push(s.delay_ms());
    }
    // Frames of 0 and 10 ms play as 100; 20 ms is kept; 30 ms is 30.
    assert!(all_about(&got, &[100, 100, 20, 30]), "{got:?}");
}

#[test]
fn files_play_as_many_times_as_they_say() {
    for (loops, plays) in [(1u32, 1usize), (2, 2), (3, 3)] {
        for (name, bytes) in two_frame_files(loops) {
            let mut s = Screen::new();
            let el = simple(&s, bytes, "Twice");
            s.ui.root().child(el);
            s.frame();
            s.wait("the first frame", |s| s.color() == R);
            let mut seen = vec![s.color()];
            // Fire until nothing is pending: the last frame of the last play stays up. No timer
            // is pending while the next frame decodes, so the end is no timer for a while, not
            // one moment without one (a loaded machine decodes slowly).
            let quiet = std::cell::Cell::new(None::<std::time::Instant>);
            let ended = |s: &Screen| {
                if !s.timers.is_empty() {
                    quiet.set(None);
                    return false;
                }
                let since = quiet.get().unwrap_or_else(std::time::Instant::now);
                quiet.set(Some(since));
                since.elapsed() >= Duration::from_millis(300)
            };
            loop {
                s.wait("the next timer or the end", |s| !s.timers.is_empty() || ended(s));
                if s.timers.is_empty() {
                    break;
                }
                assert!(seen.len() < 20, "{name} never stops");
                let before = s.color();
                s.fire();
                s.wait("a frame change or the end", |s| s.color() != before || ended(s));
                if s.color() != before {
                    seen.push(s.color());
                }
            }
            let want: Vec<[u8; 4]> = (0..plays).flat_map(|_| [R, G]).collect();
            assert_eq!(seen, want, "{name} with {loops} plays");
            // Later nothing moves.
            s.idle(30);
            assert!(s.timers.is_empty(), "{name}");
            assert_eq!(s.color(), G, "{name}");
        }
    }
}

#[test]
fn a_gif_without_a_loop_count_plays_once_and_one_with_repeat_plays_one_more() {
    // GIF counts repeats after the first play: Finite(1) is two plays; none is one.
    let once = gif(4, 4, None, &[GifFrame::solid(4, 4, RED, 10), GifFrame::solid(4, 4, GREEN, 10)]);
    let twice = gif(
        4,
        4,
        Some(gif::Repeat::Finite(1)),
        &[GifFrame::solid(4, 4, RED, 10), GifFrame::solid(4, 4, GREEN, 10)],
    );
    let mut counts = Vec::new();
    for bytes in [once, twice] {
        let mut s = Screen::new();
        let el = simple(&s, bytes, "Loops");
        s.ui.root().child(el);
        s.frame();
        s.wait("the first frame", |s| s.color() == R);
        let mut shown = 1;
        while !s.timers.is_empty() {
            let before = s.color();
            s.fire();
            s.wait("a change", |s| s.color() != before || s.timers.is_empty());
            shown += 1;
        }
        counts.push(shown);
    }
    assert_eq!(counts, vec![2, 4]);
}

#[test]
fn animations_pause_off_screen_in_hidden_parts_and_in_covered_windows() {
    let mut s = Screen::new();
    let bytes = gif(
        4,
        4,
        Some(gif::Repeat::Infinite),
        &[GifFrame::solid(4, 4, RED, 10), GifFrame::solid(4, 4, GREEN, 10)],
    );
    let list = s.ui.el("list").scroll().layout(|l| {
        l.height = Length::Px(100.0);
        l.display = Display::Flex;
        l.axis = Axis::Column;
    });
    let spacer = s.ui.el("spacer").layout(|l| l.height = Length::Px(400.0));
    let cat = simple(&s, bytes, "Below the fold");
    let list = list.child(spacer).child(cat);
    s.ui.root().child(list.clone());
    s.frame();
    s.wait("the picture to be ready", |s| !s.drawn.is_empty());
    // Below the viewport: shows its first frame and runs nothing.
    s.idle(80);
    assert!(s.timers.is_empty() && s.asked == 0, "off screen: {:?} asked {}", s.timers, s.asked);
    assert_eq!(s.color(), R);

    // Scrolled into view it plays.
    s.ui.set_scroll_offset(list.id(), remedy_ui_core::Vec2::new(0.0, 380.0));
    s.wait("a timer once seen", |s| s.timers.len() == 1);
    s.fire();
    s.wait("the second frame", |s| s.color() == G && s.timers.len() == 1);

    // Scrolled away its timer is cancelled.
    s.ui.set_scroll_offset(list.id(), remedy_ui_core::Vec2::new(0.0, 0.0));
    s.wait("the timer to be cancelled", |s| s.timers.is_empty());
    let asked = s.asked;
    s.idle(60);
    assert_eq!(s.asked, asked, "nothing asked for while away");

    // Back, then a minimized window.
    s.ui.set_scroll_offset(list.id(), remedy_ui_core::Vec2::new(0.0, 380.0));
    s.wait("playing again", |s| s.timers.len() == 1);
    s.ui.set_occluded(true);
    s.wait("the timer to be cancelled by the window", |s| s.timers.is_empty());
    s.ui.set_occluded(false);
    s.wait("playing again", |s| s.timers.len() == 1);

    // Hidden by the app.
    let _ = list.clone().hidden(true);
    s.wait("the timer to be cancelled by hiding", |s| s.timers.is_empty());
    let _ = list.hidden(false);
    s.wait("playing again", |s| s.timers.len() == 1);
}

#[test]
fn under_reduced_motion_the_first_frame_shows_with_a_play_button_that_keyboards_can_press() {
    let mut s = Screen::new();
    s.ui.set_reduced_motion(true);
    let bytes = gif(
        4,
        4,
        Some(gif::Repeat::Infinite),
        &[GifFrame::solid(4, 4, RED, 10), GifFrame::solid(4, 4, GREEN, 10), GifFrame::solid(4, 4, BLUE, 10)],
    );
    let el = simple(&s, bytes, "Dancing cat");
    s.ui.root().child(el.clone());
    s.frame();
    s.wait("the first frame", |s| s.color() == R);
    s.idle(60);
    assert!(s.timers.is_empty() && s.asked == 0, "no autoplay");
    let button = find(&el, "icon_button").into_iter().next().unwrap_or_else(|| unreachable!("a play button"));
    assert_eq!(button.info().map(|i| i.hidden), Some(false));
    let sem = semantics(&button);
    assert!(sem.contains("Play animation: Dancing cat") && sem.contains("Button"), "{sem}");
    assert!(button.info().is_some_and(|i| i.focusable), "reachable by keyboard");
    let problems = audit(&mut s);
    assert!(problems.is_empty(), "{problems:?}");

    // Enter on the focused button plays; the same again pauses and goes back to the first frame.
    button.focus();
    assert!(press(&s.ui, Key::Enter));
    s.wait("playing", |s| s.timers.len() == 1);
    assert!(semantics(&button).contains("Pause animation: Dancing cat"), "{}", semantics(&button));
    s.fire();
    s.wait("the second frame", |s| s.color() == G && s.timers.len() == 1);
    assert!(press(&s.ui, Key::Space));
    s.wait("paused", |s| s.timers.is_empty() && s.color() == R);
    assert!(semantics(&button).contains("Play animation"), "{}", semantics(&button));
    // Playing again starts from the first frame.
    assert!(press(&s.ui, Key::Enter));
    s.wait("playing", |s| s.timers.len() == 1 && s.color() == R);
    let problems = audit(&mut s);
    assert!(problems.is_empty(), "{problems:?}");

    // Motion allowed again: it plays by itself and the button goes away.
    s.ui.set_reduced_motion(false);
    s.wait("hidden button", |_| button.info().is_some_and(|i| i.hidden));
    s.wait("playing", |s| s.timers.len() == 1);

    // Reduced again: the earlier choice to play does not carry over.
    s.ui.set_reduced_motion(true);
    s.wait("stopped on the first frame", |s| s.timers.is_empty() && s.color() == R);
    s.idle(40);
    assert!(s.timers.is_empty());
    assert!(semantics(&button).contains("Play animation"), "{}", semantics(&button));
}

#[test]
fn reduced_motion_turned_on_while_playing_goes_back_to_the_first_frame() {
    let mut s = Screen::new();
    let el = simple(&s, two_frame_files(0).remove(0).1, "Pulse");
    s.ui.root().child(el);
    s.frame();
    s.wait("playing", |s| s.timers.len() == 1);
    s.fire();
    s.wait("second frame", |s| s.color() == G && s.timers.len() == 1);
    s.ui.set_reduced_motion(true);
    s.wait("stopped on the first frame", |s| s.timers.is_empty() && s.color() == R);
}

#[test]
fn elements_showing_the_same_source_share_one_animation_and_free_it_together() {
    let mut s = Screen::new();
    let bytes: Arc<[u8]> = Arc::from(gif(
        4,
        4,
        Some(gif::Repeat::Infinite),
        &[GifFrame::solid(4, 4, RED, 10), GifFrame::solid(4, 4, GREEN, 10), GifFrame::solid(4, 4, BLUE, 10)],
    ));
    let a = animated(&s.ui, Source::Bytes(bytes.clone()), "One", Fit::Contain).width(40.0).height(40.0);
    let b = animated(&s.ui, Source::Bytes(bytes.clone()), "Two", Fit::Contain).width(40.0).height(40.0);
    s.ui.root().child(s.ui.el("row").row().child(a.clone()).child(b.clone()));
    s.frame();
    s.wait("both playing", |s| s.timers.len() == 2 && s.colors() == vec![R, R]);
    assert_eq!(cached(), 1, "one decoder for both");
    // Decoding runs on the workers: once all three frames are in, they are held once, not
    // once per element.
    s.wait("all three decoded", |_| resident_frames() >= 3);
    assert_eq!(resident_frames(), 3, "the three frames once");

    // The first to run out of time moves the shared playhead; the second finds it moved.
    s.fire_nth(0);
    s.wait("one moved", |s| s.colors().contains(&G));
    s.fire_nth(0);
    s.wait("both in step", |s| s.colors() == vec![G, G] && s.timers.len() == 2);
    // Both are still waiting for their next frame, none ran ahead.
    for (_, d) in &s.timers {
        assert!(d.as_millis() <= 100);
    }

    // The same source at another size is another animation.
    let big = animated(&s.ui, Source::Bytes(bytes.clone()), "Big", Fit::Contain).width(300.0).height(300.0);
    s.ui.root().child(big.clone());
    s.wait("a second animation", |_| cached() == 2);

    // Removed, they are freed.
    big.remove();
    a.remove();
    s.wait("one left", |_| cached() == 1);
    b.remove();
    s.frame();
    s.wait("none left", |_| cached() == 0 && resident_frames() == 0);
}

fn digits(i: usize) -> (u8, u8) {
    ((i % 4) as u8 + 1, ((i / 4) % 4) as u8 + 1)
}

/// A frame whose first two pixels spell `i` in base four.
fn numbered(i: usize) -> GifFrame {
    let (a, b) = digits(i);
    let mut f = GifFrame::solid(4, 4, a, 5);
    if let Some(p) = f.px.get_mut(1) {
        *p = b;
    }
    f
}

#[test]
fn a_long_animation_streams_with_a_window_of_frames_and_loops_back_to_the_start() {
    let n = 300;
    let frames: Vec<GifFrame> = (0..n).map(numbered).collect();
    let mut s = Screen::new();
    let el = simple(&s, gif(4, 4, Some(gif::Repeat::Infinite), &frames), "Long");
    s.ui.root().child(el);
    s.frame();
    s.wait("playing", |s| s.timers.len() == 1);
    let mut most = resident_frames();
    for i in 1..=(n + 3) {
        let was = s.color();
        s.fire();
        let (a, _) = digits(i % n);
        let rgba = rgba(a);
        s.wait("the next frame", |s| s.color() != was && s.timers.len() == 1);
        // Frames come in order, none skipped and none repeated.
        assert_eq!(s.color(), rgba, "frame {i}");
        most = most.max(resident_frames());
    }
    // 50 ms frames: the window is the next five, the one shown, and the poster.
    assert!(most <= 10, "kept {most} of {n} frames");
    assert!(most >= 3, "{most}");
}

#[test]
fn long_apngs_and_webps_stream_too() {
    let n = 100u32;
    let shade = |i: u32| [(i * 2) as u8, 255 - i as u8, 10, 255];
    let png = apng(4, 4, 0, None, &(0..n).map(|i| ApngFrame::solid(4, 4, shade(i), 50)).collect::<Vec<_>>());
    let wp = webp(4, 4, 0, &(0..n).map(|i| WebpFrame::solid(4, 4, shade(i), 50)).collect::<Vec<_>>());
    for (name, bytes) in [("apng", png), ("webp", wp)] {
        let mut s = Screen::new();
        let el = simple(&s, bytes, "Long");
        s.ui.root().child(el);
        s.frame();
        s.wait("playing", |s| s.timers.len() == 1 && s.color() == shade(0));
        let mut most = resident_frames();
        for i in 1..=(n + 2) {
            let was = s.color();
            s.fire();
            s.wait("the next frame", |s| s.color() != was && s.timers.len() == 1);
            assert_eq!(s.color(), shade(i % n), "{name} frame {i}");
            most = most.max(resident_frames());
        }
        assert!(most <= 10, "{name} kept {most} of {n} frames");
        // The window closing: nothing is left behind for the next file.
        s.ui.root().clear();
        s.frame();
        assert_eq!(resident_frames(), 0, "{name}");
    }
}

#[test]
fn a_short_animation_is_kept_whole_and_loops_without_decoding() {
    let mut s = Screen::new();
    let frames: Vec<GifFrame> = (0..12).map(numbered).collect();
    let el = simple(&s, gif(4, 4, Some(gif::Repeat::Infinite), &frames), "Short");
    s.ui.root().child(el);
    s.frame();
    s.wait("all twelve decoded", |_| resident_frames() == 12);
    for i in 1..30 {
        let was = s.color();
        s.fire();
        s.wait("the next frame", |s| s.color() != was && s.timers.len() == 1);
        assert_eq!(s.color(), rgba(digits(i % 12).0));
        assert_eq!(resident_frames(), 12);
    }
}

#[test]
fn stills_show_as_stills_and_a_one_frame_animation_never_runs_a_timer() {
    let mut s = Screen::new();
    s.ui.set_reduced_motion(true);
    let still = animated(&s.ui, src(png_still(8, 8, [10, 20, 30, 255])), "A still", Fit::Contain)
        .width(40.0)
        .height(40.0);
    let one = simple(&s, gif(4, 4, None, &[GifFrame::solid(4, 4, GREEN, 10)]), "One frame");
    s.ui.root().child(s.ui.el("row").row().child(still.clone()).child(one.clone()));
    s.frame();
    s.wait("both drawn", |s| s.colors() == vec![[10, 20, 30, 255], G]);
    s.idle(60);
    assert_eq!(s.asked, 0);
    // No play button where nothing moves, even under reduced motion.
    for e in [&still, &one] {
        assert!(find(e, "icon_button").iter().all(|b| b.info().is_some_and(|i| i.hidden)));
    }
    s.ui.set_reduced_motion(false);
    s.idle(60);
    assert_eq!(s.asked, 0);
    let problems = audit(&mut s);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn files_that_cannot_be_shown_say_why_and_damaged_tails_play_what_there_is() {
    let mut s = Screen::new();
    let bad = animated(&s.ui, src(b"not an image".to_vec()), "Broken", Fit::Contain).width(40.0).height(40.0);
    let frames: Vec<GifFrame> = (0..4).map(numbered).collect();
    let mut cut = gif(4, 4, Some(gif::Repeat::Infinite), &frames);
    cut.pop();
    let tail = simple(&s, cut, "No trailer");
    let mut head = gif(4, 4, Some(gif::Repeat::Infinite), &frames);
    head.truncate(30);
    let early = simple(&s, head, "Cut early");
    s.ui.root().child(s.ui.el("row").row().child(bad.clone()).child(tail.clone()).child(early.clone()));
    s.frame();
    s.wait("verdicts", |_| {
        semantics(&bad).contains("Not shown")
            && semantics(&early).contains("Not shown")
            && !semantics(&tail).contains("Not shown")
    });
    s.wait("the damaged-tail animation plays", |s| s.timers.len() == 1);
    // All four frames before the missing trailer play.
    let mut seen = Vec::new();
    for _ in 0..5 {
        seen.push(s.color());
        let was = s.color();
        s.fire();
        s.wait("next", |s| s.color() != was && s.timers.len() == 1);
    }
    let want: Vec<[u8; 4]> = (0..5).map(|i| rgba(digits(i % 4).0)).collect();
    assert_eq!(seen, want);
}

#[test]
fn sizes_and_accessibility_are_those_of_the_image_element() {
    let mut s = Screen::new();
    let el =
        animated(&s.ui, src(gif(30, 20, None, &[GifFrame::solid(30, 20, RED, 10)])), "Red box", Fit::Contain);
    let decor = animated(&s.ui, src(gif(10, 10, None, &[GifFrame::solid(10, 10, RED, 10)])), "", Fit::Cover)
        .width(20.0)
        .height(20.0);
    s.ui.root().child(s.ui.el("col").column().child(el.clone()).child(decor.clone()));
    s.frame();
    s.wait("drawn", |s| s.drawn.len() == 2);
    let b = el.bounds().unwrap_or_default();
    assert_eq!((b.width(), b.height()), (30.0, 20.0), "takes the picture's own size");
    assert!(semantics(&el).contains("Red box") && semantics(&el).contains("Image"), "{}", semantics(&el));
    assert!(semantics(&decor).contains("hidden"), "{}", semantics(&decor));
    let problems = audit(&mut s);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_changed_file_is_played_again_by_every_element_showing_it() {
    let dir = std::env::temp_dir().join(format!("remedy-anim-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("loop.gif");
    let two = |a, b| {
        gif(4, 4, Some(gif::Repeat::Infinite), &[GifFrame::solid(4, 4, a, 10), GifFrame::solid(4, 4, b, 10)])
    };
    let _ = std::fs::write(&path, two(RED, GREEN));
    let mut s = Screen::new();
    let first = animated(&s.ui, Source::File(path.clone()), "Loop", Fit::Contain).width(40.0).height(40.0);
    s.ui.root().child(first);
    s.frame();
    s.wait("red", |s| s.color() == R);
    std::thread::sleep(Duration::from_millis(30));
    let _ = std::fs::write(&path, two(BLUE, YELLOW));
    let second = animated(&s.ui, Source::File(path.clone()), "Loop", Fit::Contain).width(40.0).height(40.0);
    s.ui.root().child(second);
    s.wait("both on the new file", |s| s.colors() == vec![B, B]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trimming_memory_lets_go_of_idle_frames_and_they_come_back_when_played() {
    let mut s = Screen::new();
    let frames: Vec<GifFrame> = (0..12).map(numbered).collect();
    let list = s.ui.el("list").scroll().layout(|l| {
        l.height = Length::Px(100.0);
        l.display = Display::Flex;
        l.axis = Axis::Column;
    });
    let spacer = s.ui.el("spacer").layout(|l| l.height = Length::Px(400.0));
    let el = simple(&s, gif(4, 4, Some(gif::Repeat::Infinite), &frames), "Idle");
    let list = list.child(spacer).child(el);
    s.ui.root().child(list.clone());
    s.frame();
    s.wait("the poster", |s| !s.drawn.is_empty());
    s.ui.set_scroll_offset(list.id(), remedy_ui_core::Vec2::new(0.0, 380.0));
    s.wait("all decoded", |_| resident_frames() == 12);
    s.ui.set_scroll_offset(list.id(), remedy_ui_core::Vec2::new(0.0, 0.0));
    s.wait("paused", |s| s.timers.is_empty());
    // The system saying memory is short trims, as the runner passes it on.
    remedy_ui_view::device::memory_pressure(remedy_ui_view::device::MemoryPressure::Moderate);
    assert_eq!(resident_frames(), 1, "only the poster stays");
    trim_memory();
    assert_eq!(resident_frames(), 1);
    s.ui.set_scroll_offset(list.id(), remedy_ui_core::Vec2::new(0.0, 380.0));
    s.wait("playing again", |s| s.timers.len() == 1);
    let was = s.color();
    s.fire();
    s.wait("moved on", |s| s.color() != was && s.timers.len() == 1);
    assert!(resident_frames() >= 3);
}
