//! Ambient motion under its budget: painters animate at the policy's rate, only while seen,
//! and cost nothing while paused, hidden, scrolled away or minimized.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::{Point, Size, Vec2};
use remedy_ui_layout::Length;
use remedy_ui_style::{StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{AmbientLoop, AmbientMode, AmbientPolicy, AmbientTick, El, Request, TickKind, Ui};

const SIZE: Size = Size { width: 200.0, height: 120.0 };

fn ui() -> Ui {
    Ui::new(Theme::dark_forest(), StyleSheet::new(), FontCollection::new())
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// A frame at `t` ms on the frame clock; the requests it left.
fn frame_at(ui: &Ui, t: u64) -> Vec<Request> {
    ui.set_frame_time(ms(t));
    let _ = ui.frame(SIZE, 1.0);
    ui.take_requests()
}

fn timers(r: &[Request]) -> Vec<(u64, Duration)> {
    r.iter()
        .filter_map(|r| match r {
            Request::Timer { id, delay } => Some((*id, *delay)),
            _ => None,
        })
        .collect()
}

type Ticks = Rc<RefCell<Vec<Option<AmbientTick>>>>;

fn firefly(ui: &Ui) -> (El, Ticks) {
    let ticks: Ticks = Rc::default();
    let t2 = ticks.clone();
    let el = ui
        .el("firefly")
        .layout(|l| {
            l.height = Length::Px(20.0);
            l.width = Length::Px(20.0);
            l.shrink = 0.0;
        })
        .paint(move |cx| t2.borrow_mut().push(cx.ambient()));
    (el, ticks)
}

#[test]
fn an_ambient_painter_repaints_at_the_policy_rate_not_every_frame() {
    let ui = ui();
    let (fly, ticks) = firefly(&ui);
    ui.root().child(fly);
    let r = frame_at(&ui, 0);
    assert_eq!(ticks.borrow().len(), 1);
    let t = timers(&r);
    assert_eq!(t.len(), 1, "one timer for the next ambient frame: {r:?}");
    assert_eq!(t.first().map(|t| t.1), Some(Duration::from_secs(1) / 30));
    // Frames for other reasons before it is due do not draw it again.
    let _ = frame_at(&ui, 16);
    assert_eq!(ticks.borrow().len(), 1, "throttled");
    for (id, _) in t {
        ui.fire_timer(id);
    }
    let _ = frame_at(&ui, 33);
    let got: Vec<_> = ticks.borrow().clone();
    assert_eq!(got.len(), 2);
    let tick = got.last().copied().flatten();
    assert!(tick.is_some_and(|k| k.t == ms(33) && k.dt == ms(33) && !k.light), "{tick:?}");
    assert_eq!(ui.ambient_waiting().0, 1);
}

#[test]
fn ambient_painters_get_a_layer_of_their_own() {
    let ui = ui();
    let (fly, _) = firefly(&ui);
    ui.root().child(ui.el("backdrop").layout(|l| l.height = Length::Px(40.0))).child(fly.clone());
    ui.set_frame_time(ms(0));
    let layers = ui.frame(SIZE, 1.0).layers.unwrap_or_default();
    let _ = ui.frame(SIZE, 1.0);
    let mut n = 0;
    layers.for_each(|_| n += 1);
    // Root, the clip and content layers of the firefly, and the segment after it.
    assert!(n >= 3, "{n} layers");
    fly.repaint();
    let f = ui.frame(SIZE, 1.0);
    assert_eq!(ui.last_frame_stats().repainted, vec![fly.id()], "only the firefly repaints");
    assert!(f.layers.is_some());
}

#[test]
fn paused_ambient_draws_still_schedules_nothing_and_resumes() {
    let ui = ui();
    let (fly, ticks) = firefly(&ui);
    ui.root().child(fly);
    ui.set_ambient_policy(AmbientPolicy::off());
    let _ = frame_at(&ui, 0);
    let r = frame_at(&ui, 1);
    assert_eq!(ticks.borrow().as_slice(), &[None], "a still frame");
    assert!(timers(&r).is_empty() && !r.contains(&Request::Frame), "{r:?}");
    assert_eq!(ui.ambient(), None);
    // Turned on again: drawn again at once, moving.
    ui.set_ambient_policy(AmbientPolicy::default());
    let r = ui.take_requests();
    assert!(r.contains(&Request::Frame));
    let _ = frame_at(&ui, 50);
    assert!(ticks.borrow().last().copied().flatten().is_some());
}

#[test]
fn reduced_motion_wins_and_light_mode_says_so() {
    let ui = ui();
    let (fly, ticks) = firefly(&ui);
    ui.root().child(fly);
    ui.set_reduced_motion(true);
    let _ = frame_at(&ui, 0);
    assert_eq!(ticks.borrow().last().copied().flatten(), None);
    ui.set_reduced_motion(false);
    let mut light = AmbientPolicy::default();
    light.mode = AmbientMode::Light;
    ui.set_ambient_policy(light);
    let r = frame_at(&ui, 10);
    assert!(ticks.borrow().last().copied().flatten().is_some_and(|t| t.light));
    assert_eq!(timers(&r).first().map(|t| t.1), Some(Duration::from_secs(1) / 15));
}

#[test]
fn background_and_power_saving_slow_it_down() {
    let ui = ui();
    ui.set_window_focused(false);
    assert_eq!(ui.ambient().map(|r| r.interval), Some(Duration::from_secs(1) / 10));
    ui.set_window_focused(true);
    ui.set_power_saving(true);
    assert_eq!(ui.ambient().map(|r| r.interval), Some(Duration::from_secs(1) / 15));
    let mut still = AmbientPolicy::default();
    still.power_saving_fps = Some(0);
    ui.set_ambient_policy(still);
    assert_eq!(ui.ambient(), None, "still on battery");
}

#[test]
fn a_minimized_window_costs_nothing_until_it_shows() {
    let ui = ui();
    let (fly, ticks) = firefly(&ui);
    ui.root().child(fly);
    let r = frame_at(&ui, 0);
    ui.set_occluded(true);
    let _ = ui.take_requests();
    for (id, _) in timers(&r) {
        ui.fire_timer(id);
    }
    let _ = frame_at(&ui, 40);
    let r = frame_at(&ui, 41);
    assert_eq!(ticks.borrow().len(), 2, "the due repaint draws a still frame");
    assert_eq!(ticks.borrow().last().copied().flatten(), None);
    assert!(timers(&r).is_empty() && !r.contains(&Request::Frame), "nothing more: {r:?}");
    assert_eq!(ui.ambient_waiting(), (0, 1));
    ui.set_occluded(false);
    assert!(ui.take_requests().contains(&Request::Frame));
    let _ = frame_at(&ui, 5000);
    assert!(ticks.borrow().last().copied().flatten().is_some_and(|t| t.dt == ms(250)), "dt is capped");
}

#[test]
fn scrolled_out_of_sight_it_stops_and_comes_back() {
    let ui = ui();
    let (fly, ticks) = firefly(&ui);
    let list = ui
        .el("list")
        .scroll()
        .layout(|l| {
            l.height = Length::Px(100.0);
            l.display = remedy_ui_layout::Display::Flex;
            l.axis = remedy_ui_layout::Axis::Column;
        })
        .child(fly.clone())
        .child(ui.el("rest").layout(|l| {
            l.height = Length::Px(400.0);
            l.shrink = 0.0;
        }));
    ui.root().child(list.clone());
    let _ = frame_at(&ui, 0);
    assert!(fly.is_seen());
    ui.set_scroll_offset(list.id(), Vec2::new(0.0, 200.0));
    let _ = frame_at(&ui, 10);
    fly.repaint();
    let _ = frame_at(&ui, 20);
    let r = frame_at(&ui, 21);
    assert!(!fly.is_seen());
    let n = ticks.borrow().len();
    assert!(timers(&r).is_empty() && !r.contains(&Request::Frame), "nothing while unseen: {r:?}");
    ui.set_scroll_offset(list.id(), Vec2::ZERO);
    let _ = frame_at(&ui, 30);
    let _ = frame_at(&ui, 40);
    assert!(ticks.borrow().len() > n, "drawn again when it shows");
}

#[test]
fn unseen_painters_asking_every_frame_stop_asking() {
    let ui = ui();
    let painted = Rc::new(Cell::new(0u32));
    let p2 = painted.clone();
    let spinner = ui.el("spinner").layout(|l| l.height = Length::Px(10.0)).paint(move |cx| {
        p2.set(p2.get() + 1);
        cx.request_next_frame();
    });
    ui.root().child(spinner.clone());
    let r = frame_at(&ui, 0);
    assert!(r.contains(&Request::Frame));
    let _ = spinner.clone().hidden(true);
    let _ = frame_at(&ui, 16);
    let _ = spinner.clone().hidden(false);
    ui.set_occluded(true);
    let _ = ui.take_requests();
    let r = frame_at(&ui, 32);
    let r2 = frame_at(&ui, 48);
    assert!(!r.contains(&Request::Frame) && !r2.contains(&Request::Frame), "{r:?} {r2:?}");
    let n = painted.get();
    ui.set_occluded(false);
    let _ = frame_at(&ui, 64);
    assert!(painted.get() > n);
}

#[test]
fn a_painter_can_ask_for_a_frame_later() {
    let ui = ui();
    ui.root().child(ui.el("blink").layout(|l| l.height = Length::Px(10.0)).paint(|cx| {
        cx.request_frame_in(Duration::from_millis(500));
    }));
    let r = frame_at(&ui, 0);
    assert_eq!(timers(&r).first().map(|t| t.1), Some(ms(500)));
    assert!(!frame_at(&ui, 1).contains(&Request::Frame));
}

#[test]
fn a_declared_rate_caps_the_painter() {
    let ui = ui();
    ui.root().child(ui.el("flame").layout(|l| l.height = Length::Px(10.0)).paint(|cx| {
        let _ = cx.ambient_at(12);
    }));
    let r = frame_at(&ui, 0);
    assert_eq!(timers(&r).first().map(|t| t.1), Some(Duration::from_secs(1) / 12));
}

#[test]
fn compositor_loops_reach_the_layer_without_repainting() {
    let ui = ui();
    let beam = ui
        .el("beam")
        .layout(|l| l.height = Length::Px(50.0))
        .ambient_loop(AmbientLoop::spin(Point::new(10.0, 10.0), Duration::from_secs(8)));
    ui.root().child(beam.clone());
    let layers = ui.frame(SIZE, 1.0).layers.unwrap_or_default();
    let mut found = 0;
    layers.for_each(|l| found += l.ambient.len());
    assert_eq!(found, 1);
    beam.set_ambient_loops(Vec::new());
    let layers = ui.frame(SIZE, 1.0).layers.unwrap_or_default();
    let mut found = 0;
    layers.for_each(|l| found += l.ambient.len());
    assert_eq!(found, 0);
}

#[test]
fn timer_loops_park_until_they_may_run() {
    let ui = ui();
    let el = ui.el("orbit").layout(|l| l.height = Length::Px(10.0));
    ui.root().child(el.clone());
    let _ = frame_at(&ui, 0);
    ui.set_ambient_policy(AmbientPolicy::off());
    assert_eq!(ui.tick_interval(TickKind::Ambient), None);
    assert!(ui.tick_interval(TickKind::Live).is_some(), "live data still runs");
    let ran = Rc::new(Cell::new(false));
    let r2 = ran.clone();
    el.resume_when(TickKind::Ambient, move || r2.set(true));
    assert!(!ran.get());
    ui.set_ambient_policy(AmbientPolicy::default());
    assert!(ran.get(), "resumed with the policy");
}
