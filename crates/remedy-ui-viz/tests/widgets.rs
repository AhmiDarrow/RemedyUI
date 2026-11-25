//! The widgets on a real window: behavior over time, repainting, keyboard, pixels and the
//! accessibility audit.

mod common;

use common::*;
use remedy_ui_core::a11y::Role;
use remedy_ui_core::Point;
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_viz::{
    BarsOptions, Clock, History, MeterOptions, Orientation, Range, RingOptions, SparklineOptions, bars,
    level_meter, sparkline, speaking_ring,
};

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(el.clone());
    let _ = frame(ui);
}

fn avatar(ui: &Ui) -> El {
    ui.el("avatar").width(40.0).height(40.0)
}

/// A ring on a window with a manual clock.
fn ring(ui: &Ui, clock: &Clock) -> (Signal<f32>, Signal<bool>, El) {
    let level = Signal::new(0.0f32);
    let speaking = Signal::new(false);
    let el = speaking_ring(ui, "Alex", level, avatar(ui), {
        let mut v_ = RingOptions::default();
        v_.speaking = Some(speaking);
        v_.clock = clock.clone();
        v_
    });
    mount(ui, &el);
    (level, speaking, el)
}

/// The ring's outermost pixel, halfway down its left edge.
fn ring_pixel(shot: &Shot, el: &El) -> [u8; 3] {
    let b = el.bounds().unwrap_or_default();
    shot.at(b.min.x as u32 + 1, ((b.min.y + b.max.y) / 2.0) as u32)
}

fn reference(shot: &Shot) -> [u8; 3] {
    shot.at(299, 399)
}

// --- the speaking ring ---

#[test]
fn the_ring_follows_the_voice_and_hangs_through_a_pause() {
    let ui = ui();
    let clock = Clock::manual();
    let (level, speaking, el) = ring(&ui, &clock);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    assert_eq!(value_text(&el), "silent");
    assert!(near(ring_pixel(&shot, &el), reference(&shot), 3), "no ring while silent");

    level.set(0.8);
    pump(&ui, &clock, 120);
    shot.take(&ui);
    assert_eq!(speaking.get_untracked().ok(), Some(true));
    assert_eq!(value_text(&el), "speaking");
    assert!(!near(ring_pixel(&shot, &el), reference(&shot), 20), "the ring is drawn");

    // A pause shorter than the hang does not blink the ring.
    level.set(0.0);
    pump(&ui, &clock, 180);
    assert_eq!(speaking.get_untracked().ok(), Some(true));
    // Then it lets go and fades out.
    pump(&ui, &clock, 200);
    assert_eq!(speaking.get_untracked().ok(), Some(false));
    assert_eq!(value_text(&el), "silent");
    pump(&ui, &clock, 1500);
    shot.take(&ui);
    assert!(near(ring_pixel(&shot, &el), reference(&shot), 3), "faded out");
}

#[test]
fn a_muted_person_shows_muted_whatever_the_microphone_hears() {
    let ui = ui();
    let clock = Clock::manual();
    let level = Signal::new(0.0f32);
    let speaking = Signal::new(false);
    let muted = Signal::new(false);
    let el = speaking_ring(&ui, "Alex", level, avatar(&ui), {
        let mut v_ = RingOptions::default();
        v_.speaking = Some(speaking);
        v_.muted = Some(muted);
        v_.clock = clock.clone();
        v_
    });
    mount(&ui, &el);
    level.set(0.8);
    pump(&ui, &clock, 120);
    assert_eq!(speaking.get_untracked().ok(), Some(true));
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let speaking_color = ring_pixel(&shot, &el);
    muted.set(true);
    pump(&ui, &clock, 120);
    shot.take(&ui);
    assert_eq!(speaking.get_untracked().ok(), Some(false), "a muted person is not speaking");
    assert_eq!(value_text(&el), "muted");
    let muted_color = ring_pixel(&shot, &el);
    assert!(!near(muted_color, reference(&shot), 20), "the muted outline is drawn");
    assert!(!near(muted_color, speaking_color, 20), "in its own color");
    // Talking while muted changes nothing.
    level.set(0.9);
    pump(&ui, &clock, 120);
    assert_eq!(speaking.get_untracked().ok(), Some(false));
    assert_eq!(value_text(&el), "muted");
    // Unmuted, the voice shows again.
    muted.set(false);
    pump(&ui, &clock, 120);
    assert_eq!(speaking.get_untracked().ok(), Some(true));
    assert_eq!(value_text(&el), "speaking");
}

#[test]
fn the_ring_rises_faster_than_it_falls() {
    let ui = ui();
    let clock = Clock::manual();
    let (level, _, el) = ring(&ui, &clock);
    let mut shot = Shot::new(&ui);
    let bg = {
        shot.take(&ui);
        reference(&shot)
    };
    let strength = |shot: &Shot| -> u32 {
        let p = ring_pixel(shot, &el);
        p.iter().zip(bg).map(|(a, b)| u32::from(a.abs_diff(b))).sum()
    };
    level.set(1.0);
    pump(&ui, &clock, 48);
    shot.take(&ui);
    let risen = strength(&shot);
    pump(&ui, &clock, 400);
    shot.take(&ui);
    let full = strength(&shot);
    assert!(risen * 10 > full * 6, "in 48 ms the ring is most of the way up: {risen} of {full}");

    // Voice ends: hang first, then the fade starts. 48 ms into the fade it is still mostly there.
    level.set(0.0);
    pump(&ui, &clock, 250 + 48);
    shot.take(&ui);
    let fading = strength(&shot);
    assert!(fading * 10 > full * 5, "the release is slow: {fading} of {full}");
    pump(&ui, &clock, 1500);
    shot.take(&ui);
    assert_eq!(strength(&shot), 0);
}

#[test]
fn a_ring_at_rest_costs_nothing() {
    let ui = ui();
    let clock = Clock::manual();
    let (level, _, _) = ring(&ui, &clock);
    pump(&ui, &clock, 100);
    assert_eq!(pending_timers(&ui, &clock), 0, "a silent ring schedules no timers");
    assert!(!frame(&ui), "and draws nothing new");
    // Noise below the speaking level changes nothing on screen.
    level.set(0.03);
    assert!(!frame(&ui));
    assert_eq!(pending_timers(&ui, &clock), 0);

    level.set(0.9);
    pump(&ui, &clock, 600);
    assert_eq!(pending_timers(&ui, &clock), 0, "a steady voice settles and the timers stop");
    let _ = frame(&ui); // the last step of the motion draws once
    assert!(!frame(&ui));
    level.set(0.0);
    pump(&ui, &clock, 2500);
    assert_eq!(pending_timers(&ui, &clock), 0, "so does the fade");
    let _ = frame(&ui); // the last step of the motion draws once
    assert!(!frame(&ui));
}

#[test]
fn with_reduced_motion_the_ring_only_switches() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let (level, speaking, el) = ring(&ui, &clock);
    let mut shot = Shot::new(&ui);
    let accent = rgb(ui.theme().colors.accent);

    level.set(0.8);
    shot.take(&ui);
    assert_eq!(speaking.get_untracked().ok(), Some(true), "at once, with no frames of rising");
    assert!(near(ring_pixel(&shot, &el), accent, 6), "the full ring, not a fade-in");

    // During the hang it is still the full ring, then it is gone: no fade between.
    level.set(0.0);
    pump(&ui, &clock, 100);
    shot.take(&ui);
    assert!(near(ring_pixel(&shot, &el), accent, 6));
    pump(&ui, &clock, 300);
    shot.take(&ui);
    assert!(near(ring_pixel(&shot, &el), reference(&shot), 3));
    assert_eq!(pending_timers(&ui, &clock), 0);
}

#[test]
fn a_ring_can_keep_a_faint_frame_while_silent() {
    let ui = ui();
    let level = Signal::new(0.0f32);
    let el = speaking_ring(&ui, "Alex", level, avatar(&ui), {
        let mut v_ = RingOptions::default();
        v_.show_when_silent = true;
        v_.clock = Clock::manual();
        v_
    });
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    let edge = shot.at(b.min.x as u32, ((b.min.y + b.max.y) / 2.0) as u32);
    assert!(!near(edge, reference(&shot), 3), "a one pixel hairline on the outer edge");
}

// --- the level meter ---

fn meter(ui: &Ui, clock: &Clock, options: MeterOptions) -> (Signal<f32>, El) {
    let level = Signal::new(0.0f32);
    let el = level_meter(ui, "Microphone", level, {
        let mut v_ = options;
        v_.clock = clock.clone();
        v_
    });
    mount(ui, &el);
    (level, el)
}

/// The pixel `f` of the way along the meter, in its middle.
fn along(shot: &Shot, el: &El, f: f32, dx: i32) -> [u8; 3] {
    let b = el.bounds().unwrap_or_default();
    let x = b.min.x + f * b.width() + dx as f32;
    shot.at(x as u32, ((b.min.y + b.max.y) / 2.0) as u32)
}

#[test]
fn the_meter_colors_its_zones_from_the_theme() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let (level, el) = meter(&ui, &clock, MeterOptions::default());
    let c = ui.theme().colors;
    let mut shot = Shot::new(&ui);

    level.set(0.5);
    shot.take(&ui);
    assert!(near(along(&shot, &el, 0.25, 0), rgb(c.ok), 4), "filled and normal");
    let empty = along(&shot, &el, 0.6, 0);
    assert!(!near(empty, rgb(c.ok), 30) && !near(empty, rgb(c.warn), 30), "empty is only a faint tint");

    level.set(0.85);
    shot.take(&ui);
    assert!(near(along(&shot, &el, 0.8, 0), rgb(c.warn), 4), "loud");
    level.set(1.0);
    shot.take(&ui);
    assert!(near(along(&shot, &el, 0.97, -3), rgb(c.danger), 4), "clipping");
}

#[test]
fn the_peak_marker_holds_then_falls_back() {
    let ui = ui();
    let clock = Clock::manual();
    let (level, el) = meter(&ui, &clock, MeterOptions::default());
    let c = ui.theme().colors;
    let mut shot = Shot::new(&ui);

    level.set(0.9);
    pump(&ui, &clock, 200);
    level.set(0.1);
    pump(&ui, &clock, 800);
    shot.take(&ui);
    assert!(near(along(&shot, &el, 0.9, -1), rgb(c.warn), 4), "the peak marker is still at 0.9");

    // After the hold it falls (0.7 a second), so it is no longer there but still showing.
    pump(&ui, &clock, 1000);
    shot.take(&ui);
    assert!(!near(along(&shot, &el, 0.9, -1), rgb(c.warn), 4), "it left its place");
    pump(&ui, &clock, 3000);
    assert_eq!(pending_timers(&ui, &clock), 0, "once it is back at the level, nothing is scheduled");
    let _ = frame(&ui); // the last step of the motion draws once
    assert!(!frame(&ui));
}

#[test]
fn with_reduced_motion_the_peak_drops_without_falling() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let (level, el) = meter(&ui, &clock, MeterOptions::default());
    let c = ui.theme().colors;
    let mut shot = Shot::new(&ui);
    level.set(1.0);
    level.set(0.2);
    shot.take(&ui);
    assert!(near(along(&shot, &el, 1.0, -1), rgb(c.danger), 4));
    pump(&ui, &clock, 1100);
    shot.take(&ui);
    assert!(near(along(&shot, &el, 1.0, -1), rgb(c.danger), 4), "held");
    pump(&ui, &clock, 300);
    shot.take(&ui);
    assert!(!near(along(&shot, &el, 1.0, -1), rgb(c.danger), 4), "gone at once");
}

#[test]
fn a_meter_without_peak_hold_shows_no_marker() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let (level, el) = meter(&ui, &clock, {
        let mut v_ = MeterOptions::default();
        v_.peak_hold = None;
        v_
    });
    let mut shot = Shot::new(&ui);
    level.set(0.9);
    level.set(0.1);
    shot.take(&ui);
    assert_eq!(pending_timers(&ui, &clock), 0);
    let c = ui.theme().colors;
    assert!(!near(along(&shot, &el, 0.9, -1), rgb(c.warn), 4));
}

#[test]
fn a_meter_at_rest_paints_and_schedules_nothing() {
    let ui = ui();
    let clock = Clock::manual();
    let (level, _) = meter(&ui, &clock, MeterOptions::default());
    let _ = frame(&ui); // the last step of the motion draws once
    assert!(!frame(&ui));
    level.set(0.4);
    pump(&ui, &clock, 3000);
    assert_eq!(pending_timers(&ui, &clock), 0);
    let _ = frame(&ui); // the last step of the motion draws once
    assert!(!frame(&ui));
    level.set(0.4);
    assert!(!frame(&ui), "the same level again changes nothing");
    level.set(0.41);
    pump(&ui, &clock, 3000);
    assert_eq!(pending_timers(&ui, &clock), 0);
}

#[test]
fn screen_readers_get_the_level_and_its_zone() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let (level, el) = meter(&ui, &clock, MeterOptions::default());
    let role = |el: &El| el.info().and_then(|i| i.semantics).map(|s| s.role());
    assert_eq!(role(&el), Some(Role::Progress));
    level.set(0.5);
    let _ = frame(&ui);
    assert_eq!(value_text(&el), "50 percent, normal");
    level.set(0.96);
    assert_eq!(value_text(&el), "95 percent, clipping");
    // Small wobbles do not churn the accessibility tree.
    level.set(0.97);
    assert_eq!(value_text(&el), "95 percent, clipping");
    level.set(0.8);
    assert_eq!(value_text(&el), "80 percent, loud");
}

fn handle(el: &El) -> El {
    find(el, "meter_threshold").into_iter().next().unwrap_or_else(|| unreachable!("a threshold handle"))
}

#[test]
fn the_threshold_is_a_slider_the_keyboard_can_set() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let threshold = Signal::new(0.2f32);
    let (_, el) = meter(&ui, &clock, {
        let mut v_ = MeterOptions::default();
        v_.threshold = Some(threshold);
        v_
    });
    let h = handle(&el);
    let info = h.info().unwrap_or_else(|| unreachable!("info"));
    assert!(info.focusable);
    let sem = info.semantics.unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!(sem.role(), Role::Slider);
    assert_eq!(sem.name().map(plain).as_deref(), Some("Microphone threshold"));
    assert_eq!(value_text(&h), "20 percent");

    let t = || threshold.get_untracked().unwrap_or(-1.0);
    h.focus();
    assert!(key(&ui, Key::ArrowRight));
    assert!((t() - 0.21).abs() < 1e-4, "{}", t());
    key(&ui, Key::ArrowLeft);
    key(&ui, Key::ArrowDown);
    assert!((t() - 0.19).abs() < 1e-4);
    key(&ui, Key::PageUp);
    assert!((t() - 0.29).abs() < 1e-4);
    key(&ui, Key::PageDown);
    key(&ui, Key::PageDown);
    assert!((t() - 0.09).abs() < 1e-4);
    key(&ui, Key::Home);
    assert_eq!(t(), 0.0);
    key(&ui, Key::ArrowLeft);
    assert_eq!(t(), 0.0, "stops at the bottom");
    key(&ui, Key::End);
    assert_eq!(t(), 1.0);
    key(&ui, Key::ArrowUp);
    assert_eq!(t(), 1.0, "stops at the top");
    assert_eq!(value_text(&h), "100 percent", "screen readers hear the change");
    assert!(!key(&ui, Key::Escape), "other keys are left alone");
}

#[test]
fn the_threshold_follows_the_pointer_only_while_pressed() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let threshold = Signal::new(0.2f32);
    let (_, el) = meter(&ui, &clock, {
        let mut v_ = MeterOptions::default();
        v_.threshold = Some(threshold);
        v_
    });
    let b = handle(&el).bounds().unwrap_or_default();
    let at = |f: f32| Point::new(b.min.x + f * b.width(), (b.min.y + b.max.y) / 2.0);
    let t = || threshold.get_untracked().unwrap_or(-1.0);

    pointer(&ui, at(0.3), PointerPhase::Down, Buttons::PRIMARY);
    assert!((t() - 0.3).abs() < 0.011, "{}", t());
    pointer(&ui, at(0.6), PointerPhase::Move, Buttons::PRIMARY);
    assert!((t() - 0.6).abs() < 0.011, "{}", t());
    pointer(&ui, at(0.6), PointerPhase::Up, Buttons::default());
    pointer(&ui, at(0.9), PointerPhase::Move, Buttons::default());
    assert!((t() - 0.6).abs() < 0.011, "moving without a button does nothing");
    pointer(&ui, at(0.5), PointerPhase::Down, Buttons::PRIMARY);
    pointer(&ui, Point::new(b.min.x - 40.0, at(0.5).y), PointerPhase::Move, Buttons::PRIMARY);
    assert_eq!(t(), 0.0, "dragging past the end stops at the end");
}

#[test]
fn a_vertical_meter_rises_and_its_threshold_goes_up() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let threshold = Signal::new(0.5f32);
    let (level, el) = meter(&ui, &clock, {
        let mut v_ = MeterOptions::default();
        v_.orientation = Orientation::Vertical;
        v_.threshold = Some(threshold);
        v_
    });
    let b = el.bounds().unwrap_or_default();
    assert!(b.height() > b.width() * 4.0, "tall and thin: {b:?}");
    let c = ui.theme().colors;
    let mut shot = Shot::new(&ui);
    level.set(0.5);
    shot.take(&ui);
    let x = ((b.min.x + b.max.x) / 2.0) as u32;
    let y_at = |f: f32| (b.max.y - f * b.height()) as u32;
    assert!(near(shot.at(x, y_at(0.2)), rgb(c.ok), 4), "filled from the bottom");
    assert!(!near(shot.at(x, y_at(0.8)), rgb(c.ok), 30), "empty at the top");

    handle(&el).focus();
    key(&ui, Key::ArrowUp);
    assert!(threshold.get_untracked().is_ok_and(|t| (t - 0.51).abs() < 1e-4));
    pointer(&ui, Point::new(b.min.x + 2.0, b.min.y), PointerPhase::Down, Buttons::PRIMARY);
    assert_eq!(threshold.get_untracked().ok(), Some(1.0), "the top is the loudest");
}

// --- sparkline and bars ---

fn plain_sparkline(ui: &Ui, h: Signal<History>, options: SparklineOptions) -> El {
    let el = sparkline(ui, "Ping", h, options);
    mount(ui, &el);
    el
}

#[test]
fn a_sparkline_repaints_only_when_its_values_change() {
    let ui = ui();
    let h = Signal::new(History::new(50));
    let el = plain_sparkline(&ui, h, SparklineOptions::default());
    assert_eq!(value_text(&el), "no data");
    assert!(!frame(&ui), "nothing changed since the first frame");
    h.update(|h| h.push(0.4));
    assert!(frame(&ui), "a new value draws");
    assert!(!frame(&ui), "and then it is still");
    assert_eq!(value_text(&el), "latest 0.40");
    h.update(|h| h.push(0.5));
    assert!(frame(&ui));
    assert_eq!(pending_timers(&ui, &Clock::manual()), 0, "a sparkline never runs a timer");
}

#[test]
fn a_filling_sparkline_draws_from_the_right_with_the_newest_dot() {
    let ui = ui();
    let h = Signal::new(History::new(100));
    for i in 0..6 {
        h.update(|h| h.push(i as f32 / 5.0));
    }
    let el = plain_sparkline(&ui, h, {
        let mut v_ = SparklineOptions::default();
        v_.range = Range::Fixed(0.0, 1.0);
        v_
    });
    let b = el.bounds().unwrap_or_default();
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let bg = reference(&shot);
    // The newest value (the top of the range) is a dot at the top right.
    assert!(near(shot.at(b.max.x as u32 - 3, b.min.y as u32 + 2), accent, 10));
    // The left half has no data yet.
    for y in b.min.y as u32..b.max.y as u32 {
        assert!(near(shot.at(b.min.x as u32 + 10, y), bg, 3), "row {y}");
    }
}

#[test]
fn a_sparkline_takes_a_plain_vec_and_a_huge_one() {
    let ui = ui();
    let v = Signal::new((0..200_000).map(|i| (i as f32 * 0.01).sin()).collect::<Vec<f32>>());
    let el = sparkline(&ui, "Wave", v, SparklineOptions::default());
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    let accent = rgb(ui.theme().colors.accent);
    let inked = (b.min.x as u32..b.max.x as u32)
        .filter(|x| (b.min.y as u32..b.max.y as u32).any(|y| near(shot.at(*x, y), accent, 12)))
        .count();
    assert!(inked > 250, "the line crosses the width: {inked} columns");
}

fn plain_bars(ui: &Ui, v: Signal<Vec<f32>>, clock: &Clock, options: BarsOptions) -> El {
    let el = bars(ui, "Spectrum", v, {
        let mut v_ = options;
        v_.clock = clock.clone();
        v_
    });
    mount(ui, &el);
    el
}

#[test]
fn bars_stand_as_tall_as_their_values() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let v = Signal::new(vec![1.0f32, 0.0, 0.5]);
    let el = plain_bars(&ui, v, &clock, {
        let mut v_ = BarsOptions::default();
        v_.gap = Some(0.0);
        v_
    });
    let b = el.bounds().unwrap_or_default();
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let third = b.width() / 3.0;
    let px = |bar: f32, up: f32| {
        shot.at((b.min.x + third * (bar + 0.5)) as u32, (b.max.y - up * b.height()) as u32)
    };
    assert!(near(px(0.0, 0.9), accent, 4), "full");
    assert!(near(px(1.0, 0.1), reference(&shot), 3), "empty");
    assert!(near(px(2.0, 0.4), accent, 4), "half, below its top");
    assert!(near(px(2.0, 0.6), reference(&shot), 3), "half, above its top");
    assert_eq!(value_text(&el), "3 bars, highest 100 percent");
}

#[test]
fn bars_fall_gently_and_then_everything_stops() {
    let ui = ui();
    let clock = Clock::manual();
    let v = Signal::new(vec![1.0f32, 1.0]);
    let el = plain_bars(&ui, v, &clock, {
        let mut v_ = BarsOptions::default();
        v_.gap = Some(0.0);
        v_
    });
    let b = el.bounds().unwrap_or_default();
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let mid = |shot: &Shot| shot.at((b.min.x + 5.0) as u32, (b.max.y - 0.5 * b.height()) as u32);
    assert!(near(mid(&shot), accent, 4), "starts full, not rising from zero");
    v.set(vec![0.0, 0.0]);
    pump(&ui, &clock, 32);
    shot.take(&ui);
    assert!(near(mid(&shot), accent, 4), "32 ms later the bar has barely moved");
    pump(&ui, &clock, 3000);
    shot.take(&ui);
    assert!(near(mid(&shot), reference(&shot), 3), "then it is down");
    assert_eq!(pending_timers(&ui, &clock), 0);
    let _ = frame(&ui); // the last step of the motion draws once
    assert!(!frame(&ui));
}

#[test]
fn more_bins_than_pixels_share_bars_and_keep_the_peak() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let mut bins = vec![0.0f32; 10_000];
    if let Some(x) = bins.get_mut(5000) {
        *x = 1.0;
    }
    let v = Signal::new(bins);
    let el = plain_bars(&ui, v, &clock, BarsOptions::default());
    let b = el.bounds().unwrap_or_default();
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let inked = (b.min.x as u32..b.max.x as u32)
        .filter(|x| near(shot.at(*x, b.max.y as u32 - 2), accent, 40))
        .count();
    assert!((1..=3).contains(&inked), "one bar for the one loud bin: {inked}");
}

#[test]
fn bars_can_fit_their_values() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let v = Signal::new(vec![5.0f32, 10.0]);
    let el = plain_bars(&ui, v, &clock, {
        let mut v_ = BarsOptions::default();
        v_.range = Some(Range::Auto);
        v_.gap = Some(0.0);
        v_
    });
    let b = el.bounds().unwrap_or_default();
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let y = (b.max.y - 0.9 * b.height()) as u32;
    assert!(
        near(shot.at((b.min.x + b.width() * 0.75) as u32, y), accent, 4),
        "the highest value fills the height"
    );
    assert!(
        near(shot.at((b.min.x + b.width() * 0.25) as u32, y), reference(&shot), 3),
        "half of it does not"
    );
}

// --- accessibility ---

#[test]
fn every_widget_passes_the_accessibility_audit() {
    let ui = ui();
    let clock = Clock::manual();
    let col = ui.el("column").column().fill_width();
    let level = Signal::new(0.3f32);
    let threshold = Signal::new(0.2f32);
    let history = Signal::new(History::new(30));
    let spectrum = Signal::new(vec![0.2f32, 0.6, 0.4]);
    let col = col
        .child(speaking_ring(&ui, "Alex", level, avatar(&ui), {
            let mut v_ = RingOptions::default();
            v_.clock = clock.clone();
            v_.show_when_silent = true;
            v_
        }))
        .child(level_meter(&ui, "Microphone", level, {
            let mut v_ = MeterOptions::default();
            v_.threshold = Some(threshold);
            v_.clock = clock.clone();
            v_
        }))
        .child(level_meter(&ui, "Output", level, {
            let mut v_ = MeterOptions::default();
            v_.orientation = Orientation::Vertical;
            v_.clock = clock.clone();
            v_
        }))
        .child(sparkline(&ui, "Ping", history, SparklineOptions::default()))
        .child(bars(&ui, "Spectrum", spectrum, {
            let mut v_ = BarsOptions::default();
            v_.clock = clock.clone();
            v_
        }));
    mount(&ui, &col);
    history.update(|h| h.push(0.5));
    pump(&ui, &clock, 200);
    let problems = audit(&ui);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn every_widget_is_named_for_screen_readers() {
    let ui = ui();
    let clock = Clock::manual();
    let level = Signal::new(0.0f32);
    let history = Signal::new(History::new(3));
    let spectrum = Signal::new(vec![0.5f32]);
    let els = [
        speaking_ring(&ui, "Alex", level, avatar(&ui), {
            let mut v_ = RingOptions::default();
            v_.clock = clock.clone();
            v_
        }),
        level_meter(&ui, "Mic", level, {
            let mut v_ = MeterOptions::default();
            v_.clock = clock.clone();
            v_
        }),
        sparkline(&ui, "Ping", history, SparklineOptions::default()),
        bars(&ui, "Spectrum", spectrum, {
            let mut v_ = BarsOptions::default();
            v_.clock = clock;
            v_
        }),
    ];
    for (el, name) in els.iter().zip(["Alex", "Mic", "Ping", "Spectrum"]) {
        let sem = el.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
        assert_eq!(sem.name(), Some(name));
    }
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&remedy_ui_viz::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn screen_readers_hear_the_window_language() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let clock = Clock::manual();
    let (level, el) = meter(&ui, &clock, MeterOptions::default());
    level.set(0.5);
    let _ = frame(&ui);
    ui.set_locale("de");
    let _ = frame(&ui);
    assert_eq!(value_text(&el), "50 %, normal");

    let (_, _, ring) = ring(&ui, &clock);
    let _ = frame(&ui);
    assert_eq!(value_text(&ring), "still");
    ui.set_locale("fr");
    let _ = frame(&ui);
    assert_eq!(value_text(&ring), "silencieux");
}
