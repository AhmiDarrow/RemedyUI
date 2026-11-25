//! Motion behavior.

use std::time::Duration;

use remedy_ui_core::{Color, Point};
use remedy_ui_core::motion::*;

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

#[test]
fn easing_curves_match_css() {
    assert_eq!(Easing::EASE_IN_OUT.apply(0.0), 0.0);
    assert_eq!(Easing::EASE_IN_OUT.apply(1.0), 1.0);
    assert!((Easing::EASE_IN_OUT.apply(0.5) - 0.5).abs() < 1e-3, "symmetric curve");
    // CSS ease-in-out at x = 0.25 is about 0.129.
    assert!((Easing::EASE_IN_OUT.apply(0.25) - 0.129).abs() < 0.005);
    let mut last = 0.0;
    for i in 0..=100 {
        let y = Easing::STANDARD.apply(i as f32 / 100.0);
        assert!(y >= last - 1e-5, "monotonic");
        last = y;
    }
}

#[test]
fn tweens_reach_their_target_on_time() {
    let mut a = Animation::new(0.0f32);
    a.animate_to(10.0, Motion::Tween { duration: ms(200), easing: Easing::Linear }, ms(1000));
    assert_eq!(a.sample(ms(1000)), 0.0);
    assert!((a.sample(ms(1100)) - 5.0).abs() < 1e-4);
    assert_eq!(a.sample(ms(1200)), 10.0);
    assert!(!a.is_settled(ms(1199)));
    assert!(a.is_settled(ms(1200)));
}

#[test]
fn a_smooth_spring_never_overshoots() {
    let mut a = Animation::new(0.0f32);
    a.animate_to(100.0, Motion::Spring(Spring::smooth(ms(300))), Duration::ZERO);
    let mut max: f32 = 0.0;
    for t in 0..2000 {
        max = max.max(a.sample(ms(t)));
    }
    assert!(max <= 100.0 + 1e-3, "{max}");
    assert!((a.sample(ms(1500)) - 100.0).abs() < 0.01);
}

#[test]
fn a_bouncy_spring_overshoots_then_settles() {
    let mut a = Animation::new(0.0f32);
    a.animate_to(100.0, Motion::Spring(Spring::bouncy(ms(400))), Duration::ZERO);
    let max = (0..3000).map(|t| a.sample(ms(t))).fold(0.0f32, f32::max);
    assert!(max > 101.0, "{max}");
    assert!(a.is_settled(ms(5000)));
}

#[test]
fn retargeting_keeps_position_and_velocity() {
    let mut a = Animation::new(0.0f32);
    let spring = Motion::Spring(Spring::smooth(ms(400)));
    a.animate_to(100.0, spring, Duration::ZERO);
    let at = ms(120);
    let (x, v) = (a.sample(at), a.velocity(at).first().copied().unwrap_or(0.0));
    a.animate_to(-50.0, spring, at);
    assert!((a.sample(at) - x).abs() < 1e-3, "no jump in position");
    let v2 = a.velocity(at).first().copied().unwrap_or(0.0);
    assert!((v2 - v).abs() < 1e-2, "no jump in velocity: {v} then {v2}");
    assert!((a.sample(ms(3000)) + 50.0).abs() < 0.01);
}

#[test]
fn springs_agree_with_simulation() {
    // Integrate the same spring with tiny steps; the closed form must match.
    for spring in
        [Spring::smooth(ms(300)), Spring::bouncy(ms(300)), Spring { stiffness: 100.0, damping: 40.0 }]
    {
        let (mut x, mut v) = (1.0f32, 0.0f32);
        let dt = 1e-5;
        for _ in 0..50_000 {
            let acc = -spring.stiffness * x - spring.damping * v;
            v += acc * dt;
            x += v * dt;
        }
        let (cx, _) = spring.displacement(1.0, 0.0, 0.5);
        assert!((cx - x).abs() < 2e-3, "{spring:?}: {cx} vs {x}");
    }
}

#[test]
fn reduced_motion_makes_moves_instant() {
    let mut a = Animation::new(Point::new(0.0, 0.0));
    a.set_reduced_motion(true);
    a.animate_to(Point::new(10.0, 20.0), Motion::Spring(Spring::smooth(ms(300))), Duration::ZERO);
    assert_eq!(a.sample(Duration::ZERO), Point::new(10.0, 20.0));
    assert!(a.is_settled(Duration::ZERO));
}

#[test]
fn colors_fade_through_oklab() {
    let red = Color::rgb8(255, 0, 0);
    let blue = Color::rgb8(0, 0, 255);
    let mut a = Animation::new(red);
    a.animate_to(blue, Motion::Tween { duration: ms(100), easing: Easing::Linear }, Duration::ZERO);
    let mid = a.sample(ms(50));
    assert_eq!(mid.to_rgba8(), red.mix_oklab(blue, 0.5).to_rgba8());
    assert_eq!(a.sample(ms(100)).to_rgba8(), blue.to_rgba8());
}

#[test]
fn the_frame_clock_predicts_and_sleeps() {
    let mut c = FrameClock::new(ms(16));
    c.vblank(ms(1000), ms(8)); // a 120 Hz display
    assert_eq!(c.presentation_time(), ms(1008));
    assert!(c.is_idle());
    c.request(ms(1350));
    c.request(ms(1100));
    assert_eq!(c.next_deadline(), Some(ms(1100)));
    assert!(c.take_due(ms(1200)));
    assert_eq!(c.next_deadline(), Some(ms(1350)));
    assert!(!c.take_due(ms(1300)));
}

#[test]
fn colors_arrive_exactly_at_their_target() {
    use remedy_ui_core::Color;
    use remedy_ui_core::motion::{Animation, Easing, Motion};
    use std::time::Duration;
    let red = Color::rgb8(255, 0, 0);
    let mut a = Animation::new(Color::rgb8(10, 120, 200));
    a.animate_to(red, Motion::Instant, Duration::ZERO);
    assert_eq!(a.sample(Duration::ZERO), red, "an instant move lands on the target itself");
    let mut b = Animation::new(Color::WHITE);
    b.animate_to(
        red,
        Motion::Tween { duration: Duration::from_millis(100), easing: Easing::Linear },
        Duration::ZERO,
    );
    assert_ne!(b.sample(Duration::from_millis(50)), red);
    assert_eq!(b.sample(Duration::from_millis(100)), red, "a finished tween is exactly its target");
}
