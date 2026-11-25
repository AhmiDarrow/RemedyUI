//! Orbits and path layouts on a real window: where children land, turning that stops when it
//! should, reading order and the audit.

mod common;

use std::time::Duration;

use common::*;
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::style::ColorRole;
use remedy_ui_core::{Point, Size};
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_widgets::stage::{LineStyle, OrbitOptions, PathOptions, Route, Spread, orbit, path_layout};
use remedy_ui_widgets::viz::Clock;

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(ui.el("holder").padding(20.0).child(el.clone()));
    let _ = frame(ui);
    let _ = frame(ui);
}

fn dot(ui: &Ui, name: &str) -> El {
    let n = name.to_owned();
    ui.el("dot").width(24.0).height(24.0).focusable(true).on_click(|| {}).semantics(
        remedy_ui_core::a11y::Semantics::new(remedy_ui_core::a11y::Role::Button)
            .label(n)
            .action(remedy_ui_core::a11y::Action::Click),
    )
}

fn center(el: &El) -> Point {
    el.bounds().unwrap_or_default().center()
}

fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 0.75 && (a.y - b.y).abs() < 0.75
}

#[test]
fn an_orbit_spaces_children_on_nested_rings() {
    let ui = ui();
    let (a, b, c, d, hub) = (dot(&ui, "a"), dot(&ui, "b"), dot(&ui, "c"), dot(&ui, "d"), dot(&ui, "hub"));
    let o = orbit(&ui, "Call", OrbitOptions::default().inset(0.0))
        .center(hub.clone())
        .child_on(1, a.clone())
        .child_on(1, b.clone())
        .child_on(1, c.clone())
        .child_on(1, d.clone());
    let el: El = o.into();
    let el = el.width(200.0).height(200.0);
    mount(&ui, &el);
    let o0 = el.bounds().unwrap_or_default().min;
    let at = |x: f32, y: f32| Point::new(o0.x + x, o0.y + y);
    assert!(close(center(&hub), at(100.0, 100.0)));
    // Ring 1 of two is the outer one (fraction 1): top, right, bottom, left.
    assert!(close(center(&a), at(100.0, 0.0)), "{:?}", center(&a));
    assert!(close(center(&b), at(200.0, 100.0)));
    assert!(close(center(&c), at(100.0, 200.0)));
    assert!(close(center(&d), at(0.0, 100.0)));
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
    // Reading order is the order added, not the angle.
    let names: Vec<String> = ui.reader_view().iter().filter_map(|n| n.name.clone()).collect();
    let pos = |s: &str| names.iter().position(|n| n == s);
    assert!(pos("hub") < pos("a") && pos("a") < pos("b") && pos("c") < pos("d"), "{names:?}");
}

#[test]
fn a_turning_orbit_stops_under_reduced_motion() {
    let ui = ui();
    let clock = Clock::manual();
    let a = dot(&ui, "a");
    let o = orbit(
        &ui,
        "Moons",
        OrbitOptions::default().inset(0.0).period(Duration::from_secs(4)).clock(clock.clone()),
    )
    .child(a.clone());
    let el: El = o.clone().into();
    let el = el.width(100.0).height(100.0);
    mount(&ui, &el);
    let o0 = el.bounds().unwrap_or_default().min;
    pump(&ui, &clock, 1000);
    let _ = (frame(&ui), frame(&ui));
    // A quarter turn clockwise from the top: the right edge.
    assert!((o.turn() - 90.0).abs() < 2.0, "{}", o.turn());
    let p = center(&a);
    assert!((p.x - (o0.x + 100.0)).abs() < 2.0 && (p.y - (o0.y + 50.0)).abs() < 3.0, "{p:?}");
    ui.set_reduced_motion(true);
    let _ = frame(&ui);
    pump(&ui, &clock, 100);
    let turned = o.turn();
    assert_eq!(pending_timers(&ui, &clock), 0);
    pump(&ui, &clock, 500);
    assert_eq!(o.turn(), turned, "stands still");
    // And turns again when motion is allowed.
    ui.set_reduced_motion(false);
    let _ = frame(&ui);
    pump(&ui, &clock, 500);
    assert!(o.turn() > turned);
}

#[test]
fn a_still_orbit_schedules_nothing() {
    let ui = ui();
    let clock = Clock::manual();
    let o = orbit(&ui, "Still", OrbitOptions::default().clock(clock.clone())).child(dot(&ui, "a"));
    let el: El = o.into();
    mount(&ui, &el.width(100.0).height(100.0));
    assert_eq!(pending_timers(&ui, &clock), 0);
    assert!(!frame(&ui), "nothing to draw");
}

#[test]
fn routes_measure_and_flatten() {
    let r = Route::new([Point::new(0.0, 0.0), Point::new(10.0, 0.0)]);
    assert_eq!(r.point_at(0.5), Point::new(5.0, 0.0));
    assert_eq!(r.point_at(f32::NAN), Point::new(0.0, 0.0));
    assert_eq!(Route::new([]).point_at(0.5), Point::ZERO);
    let curve = Route::new([Point::new(0.0, 0.0), Point::new(5.0, 5.0), Point::new(10.0, 0.0)]).smooth();
    let flat = curve.flatten();
    assert!(flat.len() > 10);
    // A smooth route still passes through its points.
    assert!(flat.iter().any(|p| (p.x - 5.0).abs() < 1e-3 && (p.y - 5.0).abs() < 1e-3));
    assert_eq!(flat.last().copied(), Some(Point::new(10.0, 0.0)));
    let square =
        Route::new([Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)])
            .closed();
    assert_eq!(square.length(), 4.0);
}

#[test]
fn a_path_layout_places_stations_and_travellers() {
    let ui = ui();
    let route = Route::new([Point::new(0.0, 50.0), Point::new(100.0, 50.0), Point::new(100.0, 150.0)]);
    let at = Signal::new(0.0f32);
    let (s1, s2, s3, train) = (dot(&ui, "s1"), dot(&ui, "s2"), dot(&ui, "s3"), dot(&ui, "train"));
    let p = path_layout(
        &ui,
        "Line 1",
        route,
        PathOptions::default()
            .design(Size::new(100.0, 200.0))
            .line(LineStyle::dashed(ColorRole::Accent).width(4.0)),
    )
    .child(s1.clone())
    .child(s2.clone())
    .child(s3.clone())
    .child_at(at, train.clone());
    let el: El = p.into();
    let el = el.width(200.0).height(200.0);
    mount(&ui, &el);
    let o = el.bounds().unwrap_or_default().min;
    let pt = |x: f32, y: f32| Point::new(o.x + x, o.y + y);
    // Stretched by 2 horizontally: the route runs (0,50) to (200,50) to (200,150).
    assert!(close(center(&s1), pt(0.0, 50.0)));
    assert!(close(center(&s2), pt(150.0, 50.0)), "{:?}", center(&s2));
    assert!(close(center(&s3), pt(200.0, 150.0)));
    assert!(close(center(&train), pt(0.0, 50.0)));
    at.set(1.0);
    let _ = (frame(&ui), frame(&ui));
    assert!(close(center(&train), pt(200.0, 150.0)));

    // The line is drawn in the accent.
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let hits = (20..130).filter(|x| near(shot.at(o.x as u32 + x, o.y as u32 + 50), accent, 40)).count();
    assert!(hits > 30 && hits < 120, "dashed: {hits}");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn centered_spread_leaves_the_ends_free() {
    let ui = ui();
    let (a, b) = (dot(&ui, "a"), dot(&ui, "b"));
    let p = path_layout(
        &ui,
        "River",
        Route::new([Point::new(0.0, 0.5), Point::new(1.0, 0.5)]),
        PathOptions::default().spread(Spread::Centered),
    )
    .child(a.clone())
    .child(b.clone());
    let el: El = p.into();
    let el = el.width(200.0).height(100.0);
    mount(&ui, &el);
    let o = el.bounds().unwrap_or_default().min;
    assert!(close(center(&a), Point::new(o.x + 50.0, o.y + 50.0)));
    assert!(close(center(&b), Point::new(o.x + 150.0, o.y + 50.0)));
}
