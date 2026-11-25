//! Knobs, tuners, faders and big buttons on a real window: pointer, wheel, keyboard,
//! assistive technology and the audit.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::*;
use remedy_ui_core::a11y::{Action, Role};
use remedy_ui_core::{Point, Vec2};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, PointerPhase, WheelEvent};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_viz::Clock;
use remedy_ui_widgets_stage::{
    BigButtonOptions, ButtonLook, FaderOptions, KnobOptions, Station, TunerOptions, TunerShape, big_button,
    big_toggle, fader, knob, tuner,
};

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(ui.el("holder").padding(20.0).child(el.clone()));
    let _ = frame(ui);
    let _ = frame(ui);
}

fn semantics(el: &El) -> remedy_ui_core::a11y::Semantics {
    el.info().and_then(|i| i.semantics).unwrap_or_else(|| remedy_ui_core::a11y::Semantics::new(Role::Group))
}

fn act(ui: &Ui, el: &El, a: Action) -> bool {
    let name = semantics(el).name().map(str::to_owned);
    let id = ui.reader_view().iter().find(|n| n.name == name).map(|n| n.id);
    let r = id.is_some_and(|id| ui.accessibility_action(id, a));
    let _ = frame(ui);
    r
}

fn wheel(ui: &Ui, at: Point, dy: f32) {
    let _ = ui.wheel(&WheelEvent::new(at, Vec2::new(0.0, dy)));
    let _ = frame(ui);
}

fn drag(ui: &Ui, from: Point, to: Point) {
    pointer(ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(ui, to, PointerPhase::Move, Buttons::PRIMARY);
    pointer(ui, to, PointerPhase::Up, Buttons::default());
}

// --- knob ---

#[test]
fn a_knob_turns_with_keys_wheel_and_drag() {
    let ui = ui();
    let v = Signal::new(50.0f64);
    let el = knob(
        &ui,
        "Gain",
        v,
        KnobOptions::default().range(0.0, 100.0).step(1.0).value_text(|v| format!("{v:.0} percent")),
    );
    mount(&ui, &el);
    let s = semantics(&el);
    assert_eq!(s.role(), Role::Slider);
    assert_eq!(s.value_text(), Some("50 percent"));
    el.focus();
    assert!(key(&ui, Key::ArrowUp));
    assert_eq!(v.get_untracked().ok(), Some(51.0));
    assert!(key(&ui, Key::PageDown));
    assert_eq!(v.get_untracked().ok(), Some(41.0));
    assert!(key(&ui, Key::End));
    assert_eq!(v.get_untracked().ok(), Some(100.0));
    assert!(act(&ui, &el, Action::Decrement));
    assert_eq!(v.get_untracked().ok(), Some(99.0));
    let c = el.bounds().unwrap_or_default().center();
    wheel(&ui, c, 10.0);
    assert_eq!(v.get_untracked().ok(), Some(98.0), "wheel down turns down");
    // Dragging up turns it up; the range clamps.
    drag(&ui, c, Point::new(c.x, c.y - 1000.0));
    assert_eq!(v.get_untracked().ok(), Some(100.0));
    drag(&ui, c, Point::new(c.x, c.y + 100.0));
    assert_eq!(v.get_untracked().ok(), Some(50.0), "two hundred pixels is the whole range");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

// --- tuner ---

fn stations() -> Vec<Station> {
    vec![
        Station::new("General").detail("01.0"),
        Station::new("Lounge").detail("02.4"),
        Station::new("Stage").detail("07.7"),
        Station::new("Off-topic").detail("09.1"),
    ]
}

#[test]
fn a_tuner_moves_between_stations_and_reads_their_names() {
    let ui = ui();
    let sel = Signal::new(0usize);
    let clock = Clock::manual();
    let el = tuner(&ui, "Room", stations(), sel, TunerOptions::default().clock(clock.clone()));
    mount(&ui, &el);
    let s = semantics(&el);
    assert_eq!((s.role(), s.value_text()), (Role::Slider, Some("General")));
    el.focus();
    assert!(key(&ui, Key::ArrowRight));
    assert_eq!(sel.get_untracked().ok(), Some(1));
    assert_eq!(semantics(&el).value_text(), Some("Lounge"));
    assert!(key(&ui, Key::End));
    assert_eq!(sel.get_untracked().ok(), Some(3));
    assert!(key(&ui, Key::ArrowRight), "stays on the last");
    assert_eq!(sel.get_untracked().ok(), Some(3));
    assert!(act(&ui, &el, Action::Decrement));
    assert_eq!(sel.get_untracked().ok(), Some(2));
    // The needle glides, then rests.
    assert!(pump(&ui, &clock, 1000) > 0);
    assert_eq!(pending_timers(&ui, &clock), 0, "at rest");

    // Dragging the needle and letting go between stations snaps to the nearest.
    let b = el.bounds().unwrap_or_default();
    let pad = ui.theme().spacing.unit * 4.0;
    let x_of = |t: f32| b.min.x + pad + (b.width() - pad * 2.0) * t;
    drag(&ui, Point::new(x_of(0.9), b.min.y + 10.0), Point::new(x_of(0.3), b.min.y + 10.0));
    assert_eq!(sel.get_untracked().ok(), Some(1), "0.3 is nearest Lounge at 0.375");
    wheel(&ui, b.center(), -10.0);
    assert_eq!(sel.get_untracked().ok(), Some(2), "wheel up tunes up");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn an_arc_tuner_picks_by_angle_and_jumps_under_reduced_motion() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let sel = Signal::new(0usize);
    let clock = Clock::manual();
    let el = tuner(
        &ui,
        "Band",
        stations(),
        sel,
        TunerOptions::default().shape(TunerShape::Arc).clock(clock.clone()),
    );
    mount(&ui, &el);
    let b = el.bounds().unwrap_or_default();
    // Straight up from the arc's center is the middle of the dial; right of it is past the
    // middle (Stage, at 0.625).
    let c = Point::new(b.center().x + b.width() * 0.12, b.min.y + 10.0);
    drag(&ui, c, c);
    assert_eq!(sel.get_untracked().ok(), Some(2));
    sel.set(0);
    let _ = frame(&ui);
    assert_eq!(pending_timers(&ui, &clock), 0, "no glide under reduced motion");
}

// --- fader ---

#[test]
fn a_fader_drags_by_its_cap_and_steps_with_keys() {
    let ui = ui();
    let v = Signal::new(0.0f64);
    let level = Signal::new(0.5f32);
    let el =
        fader(&ui, "Maple", v, FaderOptions::default().range(0.0, 10.0).step(0.5).length(200.0).meter(level));
    mount(&ui, &el);
    // With a meter the result is a strip: the fader, then the meter.
    let kids = el.child_elements();
    assert_eq!(kids.len(), 2);
    let f = kids.first().cloned().unwrap_or_else(|| el.clone());
    let m = kids.last().cloned().unwrap_or_else(|| el.clone());
    assert_eq!(semantics(&f).role(), Role::Slider);
    let plain = |s: &str| s.replace(['\u{2068}', '\u{2069}'], "");
    assert_eq!(semantics(&m).name().map(plain).as_deref(), Some("Maple level"));
    ui.set_locale("de");
    let _ = frame(&ui);
    assert_eq!(semantics(&m).name().map(plain).as_deref(), Some("Pegel von Maple"));
    ui.set_locale("en");
    let _ = frame(&ui);
    let b = f.bounds().unwrap_or_default();
    let cap = ui.theme().spacing.unit * 4.0;
    // The cap sits at the bottom (value 0). Grabbing it a little above its center does not
    // make it jump.
    let cap_y = b.max.y - cap / 2.0;
    let x = b.center().x;
    pointer(&ui, Point::new(x, cap_y - 3.0), PointerPhase::Down, Buttons::PRIMARY);
    assert_eq!(v.get_untracked().ok(), Some(0.0));
    let travel = b.height() - cap;
    pointer(&ui, Point::new(x, cap_y - 3.0 - travel / 2.0), PointerPhase::Move, Buttons::PRIMARY);
    pointer(&ui, Point::new(x, cap_y - 3.0 - travel / 2.0), PointerPhase::Up, Buttons::default());
    assert_eq!(v.get_untracked().ok(), Some(5.0));
    // Pressing the groove away from the cap moves it there.
    pointer(&ui, Point::new(x, b.min.y + cap / 2.0), PointerPhase::Down, Buttons::PRIMARY);
    pointer(&ui, Point::new(x, b.min.y + cap / 2.0), PointerPhase::Up, Buttons::default());
    assert_eq!(v.get_untracked().ok(), Some(10.0));
    f.focus();
    assert!(key(&ui, Key::ArrowDown));
    assert_eq!(v.get_untracked().ok(), Some(9.5));
    assert!(key(&ui, Key::Home));
    assert_eq!(v.get_untracked().ok(), Some(0.0));
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

// --- big buttons ---

#[test]
fn a_big_button_presses_on_release_and_reports_holding() {
    let ui = ui();
    let presses = Rc::new(Cell::new(0));
    let p2 = presses.clone();
    let held = Signal::new(false);
    let el =
        big_button(&ui, "Push to talk", move || p2.set(p2.get() + 1), BigButtonOptions::default().held(held));
    mount(&ui, &el);
    let c = el.bounds().unwrap_or_default().center();
    pointer(&ui, c, PointerPhase::Down, Buttons::PRIMARY);
    assert_eq!(held.get_untracked().ok(), Some(true));
    assert_eq!(presses.get(), 0);
    pointer(&ui, c, PointerPhase::Up, Buttons::default());
    assert_eq!((held.get_untracked().ok(), presses.get()), (Some(false), 1));
    // Let go outside: no press.
    pointer(&ui, c, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&ui, Point::new(c.x + 500.0, c.y), PointerPhase::Up, Buttons::default());
    assert_eq!(presses.get(), 1);
    // Space holds it too.
    el.focus();
    let _ = ui.key(&KeyEvent::press(Key::Space).with_text(None));
    assert_eq!(held.get_untracked().ok(), Some(true));
    let mut up = KeyEvent::press(Key::Space).with_text(None);
    up.pressed = false;
    let _ = ui.key(&up);
    assert_eq!((held.get_untracked().ok(), presses.get()), (Some(false), 2));
    assert!(act(&ui, &el, Action::Click));
    assert_eq!(presses.get(), 3);
    let s = semantics(&el);
    assert_eq!((s.role(), s.name(), s.checked_state()), (Role::Button, Some("Push to talk"), None));
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn big_toggles_latch_and_switches_are_switches() {
    let ui = ui();
    let mic = Signal::new(false);
    let deafen = Signal::new(true);
    let a = big_toggle(&ui, "Mic", mic, BigButtonOptions::default().look(ButtonLook::Plate));
    let b = big_toggle(
        &ui,
        "Deafen",
        deafen,
        BigButtonOptions::default().look(ButtonLook::Switch).variant("danger"),
    );
    let row = ui.el("row").row().gap(20.0).child(a.clone()).child(b.clone());
    mount(&ui, &row);
    let c = a.bounds().unwrap_or_default().center();
    pointer(&ui, c, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&ui, c, PointerPhase::Up, Buttons::default());
    assert_eq!(mic.get_untracked().ok(), Some(true));
    assert_eq!(semantics(&a).checked_state(), Some(true));
    assert_eq!(semantics(&a).role(), Role::Button);
    // The switch is the first child of its row (the label is text beside it).
    let sw = b.child_elements().first().cloned().unwrap_or_else(|| b.clone());
    assert_eq!((semantics(&sw).role(), semantics(&sw).checked_state()), (Role::Switch, Some(true)));
    assert!(act(&ui, &sw, Action::Click));
    assert_eq!(deafen.get_untracked().ok(), Some(false));
    assert_eq!(semantics(&sw).checked_state(), Some(false));
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn a_lit_button_draws_brighter() {
    let ui = ui();
    let lit = Signal::new(false);
    let el = big_button(&ui, "Record", || {}, BigButtonOptions::default().lit(lit).hide_label(true));
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    let at = |s: &Shot| s.at(b.center().x as u32, (b.center().y + b.height() * 0.25) as u32);
    let dark = at(&shot);
    lit.set(true);
    shot.take(&ui);
    let bright = at(&shot);
    let sum = |p: [u8; 3]| p.iter().map(|v| u32::from(*v)).sum::<u32>();
    assert!(sum(bright) > sum(dark) + 30, "{dark:?} {bright:?}");
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_widgets_stage::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}
