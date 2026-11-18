//! The math behind the visuals: smoothing, peak hold, the speaking gate and series.

use std::time::Duration;

use remedy_ui_viz::{
    Gate, History, PeakHold, PlotBox, Range, Samples, Smoothing, Zone, Zones, extent, fraction, group_max,
    plot, smooth, unit_level,
};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn smoothing_rises_faster_than_it_falls() {
    let s = Smoothing::voice();
    let up = s.step(0.0, 1.0, ms(16));
    let down = 1.0 - s.step(1.0, 0.0, ms(16));
    assert!(up > 0.3, "attack is quick: {up}");
    assert!(down < up / 2.0, "release is slower: {down} against {up}");
}

#[test]
fn smoothing_always_arrives_and_lands_exactly() {
    let s = Smoothing::meter();
    let mut v = 0.0;
    for _ in 0..400 {
        v = s.step(v, 0.8, ms(16));
    }
    assert_eq!(v, 0.8, "smoothing finishes on the target, not a hair short");
    let mut v = 1.0;
    for _ in 0..400 {
        v = s.step(v, 0.0, ms(16));
    }
    assert_eq!(v, 0.0);
}

#[test]
fn smoothing_does_not_depend_on_the_frame_rate() {
    let s = Smoothing::voice();
    let one = s.step(0.0, 1.0, ms(32));
    let two = s.step(s.step(0.0, 1.0, ms(16)), 1.0, ms(16));
    assert!((one - two).abs() < 0.005, "{one} against {two}");
}

#[test]
fn zero_time_constants_jump_and_bad_numbers_are_silence() {
    assert_eq!(Smoothing::NONE.step(0.2, 0.9, ms(16)), 0.9);
    assert_eq!(smooth(f32::NAN, 0.5, ms(16), ms(10), ms(10)), 0.5);
    assert_eq!(smooth(0.5, f32::NAN, ms(16), ms(10), ms(10)), 0.0);
    assert_eq!(unit_level(f32::INFINITY), 0.0);
    assert_eq!(unit_level(7.0), 1.0);
    assert_eq!(unit_level(-1.0), 0.0);
}

#[test]
fn peak_holds_for_the_hold_time_then_falls_at_the_set_rate() {
    let mut p = PeakHold::new(ms(1000), 0.5);
    assert_eq!(p.feed(0.9, ms(0)), 0.9);
    // Lower levels do not pull it down during the hold.
    assert_eq!(p.feed(0.2, ms(500)), 0.9);
    assert_eq!(p.feed(0.2, ms(1000)), 0.9);
    // Half a second after the hold it has fallen by 0.25.
    let after = p.feed(0.2, ms(1500));
    assert!((after - 0.65).abs() < 1e-4, "{after}");
    // It never falls below the level.
    assert_eq!(p.feed(0.2, ms(10_000)), 0.2);
    assert!(!p.is_moving(0.2));
}

#[test]
fn a_louder_level_takes_the_peak_and_restarts_the_hold() {
    let mut p = PeakHold::new(ms(1000), 1.0);
    p.feed(0.5, ms(0));
    p.feed(0.1, ms(900));
    assert_eq!(p.feed(0.7, ms(950)), 0.7);
    assert_eq!(p.feed(0.1, ms(1900)), 0.7, "held 1000 ms from the new peak");
    assert!(p.feed(0.1, ms(2200)) < 0.7);
}

#[test]
fn a_peak_with_no_fall_rate_drops_at_once_after_the_hold() {
    let mut p = PeakHold::new(ms(300), 0.0);
    p.feed(0.8, ms(0));
    assert_eq!(p.feed(0.1, ms(200)), 0.8);
    assert_eq!(p.feed(0.1, ms(400)), 0.1);
}

#[test]
fn a_gate_opens_with_the_voice_and_hangs_through_pauses() {
    let mut g = Gate::new(0.2, ms(300));
    assert!(!g.feed(0.1, ms(16)));
    assert!(g.feed(0.5, ms(16)));
    // Between 60 percent of the threshold and the threshold: stays open without a timer.
    assert!(g.feed(0.15, ms(1000)));
    assert!(!g.is_hanging());
    // Well below: hangs, then closes.
    assert!(g.feed(0.0, ms(100)));
    assert!(g.is_hanging());
    assert!(g.feed(0.0, ms(150)));
    assert!(!g.feed(0.0, ms(100)));
    assert!(!g.is_hanging());
}

#[test]
fn a_word_inside_the_hang_keeps_the_gate_open() {
    let mut g = Gate::new(0.2, ms(300));
    g.feed(0.5, ms(16));
    g.feed(0.0, ms(250));
    assert!(g.feed(0.5, ms(16)));
    assert!(g.feed(0.0, ms(250)), "the hang started over");
}

#[test]
fn zones_split_the_scale_and_survive_odd_settings() {
    let z = Zones::default();
    assert_eq!(z.zone(0.3), Zone::Normal);
    assert_eq!(z.zone(0.7), Zone::Loud);
    assert_eq!(z.zone(0.95), Zone::Clipping);
    let odd = Zones { loud: 0.9, clipping: 0.5 };
    assert_eq!(odd.zone(0.6), Zone::Normal);
    assert_eq!(odd.zone(0.95), Zone::Clipping);
    assert_eq!(Zone::Clipping.name(), "clipping");
}

#[test]
fn history_keeps_the_newest_values() {
    let mut h = History::new(3);
    for v in [1.0, 2.0, 3.0, 4.0] {
        h.push(v);
    }
    assert_eq!(h.iter().collect::<Vec<_>>(), vec![2.0, 3.0, 4.0]);
    assert_eq!(h.latest(), Some(4.0));
    assert_eq!(Samples::capacity(&h), Some(3));
    h.set_capacity(2);
    assert_eq!(h.iter().collect::<Vec<_>>(), vec![3.0, 4.0]);
    h.set_capacity(100);
    assert_eq!(h.len(), 2, "growing keeps what there is");
    h.clear();
    assert!(Samples::is_empty(&h));
    assert_eq!(History::new(0).capacity(), 1, "a history keeps at least one value");
}

#[test]
fn ranges_fit_or_stay_fixed_and_never_collapse() {
    assert_eq!(Range::Auto.resolve(2.0, 6.0), (2.0, 6.0));
    assert_eq!(Range::Fixed(0.0, 1.0).resolve(5.0, 9.0), (0.0, 1.0));
    assert_eq!(Range::Fixed(1.0, 0.0).resolve(5.0, 9.0), (0.0, 1.0), "reversed bounds are put in order");
    let (lo, hi) = Range::Auto.resolve(3.0, 3.0);
    assert!(hi > lo && (lo + hi) / 2.0 == 3.0, "a flat series is centered: {lo}..{hi}");
    assert_eq!(extent(&vec![f32::NAN, 2.0, -1.0, f32::INFINITY]), Some((-1.0, 2.0)));
    assert_eq!(extent(&Vec::<f32>::new()), None);
    assert_eq!(fraction(5.0, 0.0, 10.0), 0.5);
    assert_eq!(fraction(50.0, 0.0, 10.0), 1.0);
}

const AREA: PlotBox = PlotBox { width: 100.0, height: 40.0, margin: 2.0 };

#[test]
fn the_newest_value_sits_on_the_right_edge_and_high_values_are_up() {
    let runs = plot(&vec![0.0, 1.0], 2, (0.0, 1.0), AREA);
    assert_eq!(runs.len(), 1);
    let run = runs.first().cloned().unwrap_or_default();
    assert_eq!(run.len(), 2);
    let (a, b) = (run.first().copied().unwrap_or_default(), run.last().copied().unwrap_or_default());
    assert_eq!((a.x, b.x), (0.0, 100.0));
    assert_eq!(a.y, 38.0, "the lowest value is at the bottom margin");
    assert_eq!(b.y, 2.0, "the highest is at the top margin");
}

#[test]
fn a_filling_history_leaves_the_left_empty() {
    let mut h = History::new(11);
    for v in [0.1, 0.2, 0.3] {
        h.push(v);
    }
    let runs = plot(&h, h.capacity(), (0.0, 1.0), AREA);
    let run = runs.first().cloned().unwrap_or_default();
    assert_eq!(run.len(), 3);
    assert_eq!(run.last().map(|p| p.x), Some(100.0));
    assert_eq!(run.first().map(|p| p.x), Some(80.0), "three of eleven slots: the last two tenths");
}

#[test]
fn values_that_are_not_numbers_break_the_line() {
    let runs = plot(&vec![0.1, 0.2, f32::NAN, 0.4, 0.5], 5, (0.0, 1.0), AREA);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 2]);
}

#[test]
fn many_values_per_pixel_keep_their_spikes_with_work_that_follows_the_width() {
    let mut v = vec![0.5f32; 100_000];
    if let Some(x) = v.get_mut(31_337) {
        *x = 1.0;
    }
    if let Some(x) = v.get_mut(70_001) {
        *x = 0.0;
    }
    let runs = plot(&v, v.len(), (0.0, 1.0), AREA);
    let points: Vec<_> = runs.iter().flatten().collect();
    assert!(points.len() <= 2 * 100 + 2, "about two points per pixel column, got {}", points.len());
    assert!(points.iter().any(|p| p.y == 2.0), "the spike up survives");
    assert!(points.iter().any(|p| p.y == 38.0), "the spike down survives");
    let xs_ordered = points.windows(2).all(|w| w.first().zip(w.last()).is_some_and(|(a, b)| a.x <= b.x));
    assert!(xs_ordered, "points stay in time order");
}

#[test]
fn grouping_bars_keeps_the_highest_of_each_group() {
    let mut v = vec![0.0f32; 1000];
    if let Some(x) = v.get_mut(503) {
        *x = 0.9;
    }
    let g = group_max(&v, 10);
    assert_eq!(g.len(), 10);
    assert_eq!(<[f32]>::get(&g, 5).copied(), Some(0.9));
    assert_eq!(g.iter().filter(|x| **x > 0.0).count(), 1);
    assert_eq!(
        group_max(&vec![0.1, f32::NAN, 0.3], 8),
        vec![0.1, 0.0, 0.3],
        "fewer values than columns pass through"
    );
    assert!(group_max(&Vec::<f32>::new(), 4).is_empty());
}
