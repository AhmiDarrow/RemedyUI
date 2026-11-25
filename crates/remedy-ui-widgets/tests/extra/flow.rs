//! The carousel and the stepper flow on a real window: sliding, dragging, keyboard, autoplay,
//! reduced motion, step gating, focus and the accessibility audit.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use common::*;
use remedy_ui_core::a11y::{Live, Role};
use remedy_ui_core::Point;
use remedy_ui_core::input::{Buttons, Key, Modifiers, PointerPhase};
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::style::States;
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_widgets::viz::Clock;
use remedy_ui_widgets::base as w;
use remedy_ui_widgets::extra::{CarouselOptions, Step, StepperLabels, carousel, stepper_flow};

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(el.clone());
    let _ = frame(ui);
}

fn button_named(root: &El, name: &str) -> El {
    find(root, "button")
        .into_iter()
        .chain(find(root, "icon_button"))
        .find(|b| name_of(b) == name)
        .unwrap_or_else(|| unreachable!("no button {name}"))
}

fn disabled(el: &El) -> bool {
    el.info().is_some_and(|i| i.states.contains(States::DISABLED))
}

fn hidden(el: &El) -> bool {
    el.info().is_some_and(|i| i.hidden)
}

// --- carousel ---

struct Car {
    ui: Ui,
    root: El,
    pages: Vec<El>,
    index: Signal<usize>,
    clock: Clock,
}

fn car(n: usize, wrap: bool, autoplay: Option<Duration>) -> Car {
    let ui = ui();
    let clock = Clock::manual();
    let index = Signal::new(0usize);
    let pages: Vec<El> = (0..n).map(|i| w::text(&ui, format!("Slide number {}", i + 1))).collect();
    let root = carousel(&ui, "Highlights", pages.clone(), index, {
        let mut v_ = CarouselOptions::default();
        v_.wrap = wrap;
        v_.autoplay = autoplay;
        v_.clock = clock.clone();
        v_
    });
    mount(&ui, &root);
    Car { ui, root, pages, index, clock }
}

fn shown(c: &Car) -> Vec<bool> {
    // A page is shown when its wrapper is not hidden.
    c.pages.iter().map(|p| p.parent().is_some_and(|w| !hidden(&w))).collect()
}

fn left_of(c: &Car, i: usize) -> f32 {
    c.pages.get(i).and_then(El::bounds).map_or(f32::NAN, |b| b.min.x)
}

#[test]
fn only_the_current_page_is_shown_and_the_group_says_where_it_is() {
    let c = car(3, false, None);
    assert_eq!(shown(&c), [true, false, false]);
    assert_eq!(role_of(&c.root), Some(Role::Group));
    assert_eq!(name_of(&c.root), "Highlights");
    assert_eq!(value_text(&c.root), "Page 1 of 3");
    assert_eq!(c.root.info().and_then(|i| i.semantics).map(|s| s.live_state()), Some(Live::Polite));
    let wrapper = c.pages.first().and_then(El::parent).unwrap_or_else(|| unreachable!("wrapper"));
    assert_eq!(name_of(&wrapper), "Page 1 of 3");
    let viewport = first(&c.root, "carousel_viewport");
    let (vb, pb) =
        (viewport.bounds().unwrap_or_default(), c.pages.first().and_then(El::bounds).unwrap_or_default());
    assert!((vb.min.x - pb.min.x).abs() < 0.5, "the page sits in the viewport");
    assert!(audit(&c.ui).is_empty(), "{:?}", audit(&c.ui));
}

#[test]
fn changing_the_index_slides_to_the_page_and_settles() {
    let c = car(3, false, None);
    let viewport_left = first(&c.root, "carousel_viewport").bounds().unwrap_or_default().min.x;
    c.index.set(1);
    let _ = frame(&c.ui);
    assert_eq!(value_text(&c.root), "Page 2 of 3");
    pump_frame(&c.ui, &c.clock, 16);
    assert_eq!(shown(&c), [true, true, false], "both pages show while sliding");
    let start = left_of(&c, 1);
    assert!(start > viewport_left + 100.0, "the new page enters from the side: {start}");
    pump_frame(&c.ui, &c.clock, 48);
    let mid = left_of(&c, 1);
    assert!(mid < start && mid > viewport_left, "it is on its way: {mid}");
    pump_frame(&c.ui, &c.clock, 600);
    assert_eq!(shown(&c), [false, true, false], "settled");
    assert!((left_of(&c, 1) - viewport_left).abs() < 0.5);
    // At rest nothing is scheduled.
    assert_eq!(pending_timers(&c.ui, &c.clock), 0, "no work while still");
}

#[test]
fn reduced_motion_changes_pages_at_once() {
    let c = car(3, false, None);
    c.ui.set_reduced_motion(true);
    c.index.set(2);
    let _ = frame(&c.ui);
    pump_frame(&c.ui, &c.clock, 16);
    assert_eq!(shown(&c), [false, false, true]);
    let viewport_left = first(&c.root, "carousel_viewport").bounds().unwrap_or_default().min.x;
    assert!((left_of(&c, 2) - viewport_left).abs() < 0.5);
}

#[test]
fn the_arrow_keys_turn_pages_and_stop_at_the_ends() {
    let c = car(3, false, None);
    c.root.focus();
    assert!(key(&c.ui, Key::ArrowRight));
    assert_eq!(c.index.get_untracked().ok(), Some(1));
    key(&c.ui, Key::End);
    assert_eq!(c.index.get_untracked().ok(), Some(2));
    key(&c.ui, Key::ArrowRight);
    assert_eq!(c.index.get_untracked().ok(), Some(2), "no page after the last");
    key(&c.ui, Key::ArrowLeft);
    key(&c.ui, Key::Home);
    assert_eq!(c.index.get_untracked().ok(), Some(0));
    key(&c.ui, Key::ArrowDown);
    assert_eq!(c.index.get_untracked().ok(), Some(1), "down is next, which is what Increment sends");
    key(&c.ui, Key::ArrowUp);
    assert_eq!(c.index.get_untracked().ok(), Some(0));
    assert!(!key(&c.ui, Key::char('q')));
    assert!(
        button_named(&c.root, "Previous page").info().is_some_and(|i| i.states.contains(States::DISABLED))
    );
}

#[test]
fn a_wrapping_carousel_goes_round() {
    let c = car(3, true, None);
    c.root.focus();
    key(&c.ui, Key::ArrowLeft);
    assert_eq!(c.index.get_untracked().ok(), Some(2), "back from the first goes to the last");
    key(&c.ui, Key::ArrowRight);
    assert_eq!(c.index.get_untracked().ok(), Some(0));
    assert!(!disabled(&button_named(&c.root, "Previous page")));
    // Sliding from the last to the first goes forward, not back across every page.
    c.index.set(2);
    pump_frame(&c.ui, &c.clock, 800);
    c.index.set(0);
    pump_frame(&c.ui, &c.clock, 16);
    let viewport_left = first(&c.root, "carousel_viewport").bounds().unwrap_or_default().min.x;
    assert!(left_of(&c, 0) > viewport_left, "the first page arrives from the end side");
    pump_frame(&c.ui, &c.clock, 800);
    assert_eq!(shown(&c), [true, false, false]);
}

#[test]
fn buttons_and_dots_turn_pages_and_are_named() {
    let c = car(4, false, None);
    click(&c.ui, center(&button_named(&c.root, "Next page")));
    assert_eq!(c.index.get_untracked().ok(), Some(1));
    click(&c.ui, center(&button_named(&c.root, "Previous page")));
    assert_eq!(c.index.get_untracked().ok(), Some(0));
    let dots = find(&c.root, "carousel_dot_target");
    assert_eq!(dots.len(), 4);
    assert_eq!(name_of(dots.get(2).unwrap_or_else(|| unreachable!("dot"))), "Go to page 3 of 4");
    click(&c.ui, center(dots.get(2).unwrap_or_else(|| unreachable!("dot"))));
    assert_eq!(c.index.get_untracked().ok(), Some(2));
    let selected = dots
        .iter()
        .map(|d| d.info().and_then(|i| i.semantics).and_then(|s| s.selected_state()))
        .collect::<Vec<_>>();
    assert_eq!(selected, [Some(false), Some(false), Some(true), Some(false)]);
    assert!(
        dots.iter().all(|d| d.bounds().is_some_and(|b| b.width() >= 24.0 && b.height() >= 24.0)),
        "full-size targets"
    );
    assert!(audit(&c.ui).is_empty(), "{:?}", audit(&c.ui));
}

#[test]
fn dragging_turns_the_page_past_a_quarter_and_springs_back_short_of_it() {
    let c = car(3, false, None);
    let view = first(&c.root, "carousel_viewport");
    let b = view.bounds().unwrap_or_default();
    let y = b.min.y + 5.0;
    let from = Point::new(b.min.x + b.width() * 0.8, y);
    // A short drag springs back.
    pointer(&c.ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x - b.width() * 0.1, y), PointerPhase::Move, Buttons::PRIMARY);
    assert!(left_of(&c, 1) < b.max.x, "the next page follows the pointer");
    pointer(&c.ui, Point::new(from.x - b.width() * 0.1, y), PointerPhase::Up, Buttons(0));
    pump_frame(&c.ui, &c.clock, 800);
    assert_eq!(c.index.get_untracked().ok(), Some(0));
    assert_eq!(shown(&c), [true, false, false]);
    // A long one turns it.
    pointer(&c.ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x - b.width() * 0.6, y), PointerPhase::Move, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x - b.width() * 0.6, y), PointerPhase::Up, Buttons(0));
    pump_frame(&c.ui, &c.clock, 800);
    assert_eq!(c.index.get_untracked().ok(), Some(1));
    // Dragging right goes back.
    let from = Point::new(b.min.x + b.width() * 0.2, y);
    pointer(&c.ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x + b.width() * 0.6, y), PointerPhase::Move, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x + b.width() * 0.6, y), PointerPhase::Up, Buttons(0));
    pump_frame(&c.ui, &c.clock, 800);
    assert_eq!(c.index.get_untracked().ok(), Some(0));
    // Dragging past the first page goes nowhere.
    pointer(&c.ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x + b.width() * 0.9, y), PointerPhase::Move, Buttons::PRIMARY);
    pointer(&c.ui, Point::new(from.x + b.width() * 0.9, y), PointerPhase::Up, Buttons(0));
    pump_frame(&c.ui, &c.clock, 800);
    assert_eq!(c.index.get_untracked().ok(), Some(0));
}

#[test]
fn right_to_left_pages_run_from_the_right() {
    let ui = ui();
    let _ = ui.root().layout(|l| l.direction = remedy_ui_core::layout::Direction::Rtl);
    let clock = Clock::manual();
    let index = Signal::new(0usize);
    let pages: Vec<El> = (0..3).map(|i| w::text(&ui, format!("Slide {i}"))).collect();
    let root = carousel(&ui, "Highlights", pages.clone(), index, {
        let mut v_ = CarouselOptions::default();
        v_.clock = clock.clone();
        v_
    });
    mount(&ui, &root);
    root.focus();
    // Left is on in a right to left window.
    key(&ui, Key::ArrowLeft);
    assert_eq!(index.get_untracked().ok(), Some(1));
    let _ = frame(&ui);
    let viewport = first(&root, "carousel_viewport").bounds().unwrap_or_default();
    assert!(
        pages
            .get(1)
            .and_then(El::bounds)
            .is_some_and(|b| b.max.x < viewport.max.x + 0.5 && b.min.x < viewport.min.x + viewport.width()),
        "the next page enters from the left"
    );
    pump_frame(&ui, &clock, 800);
    // Dragging to the right moves on.
    let b = viewport;
    let from = Point::new(b.min.x + b.width() * 0.2, b.min.y + 5.0);
    pointer(&ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&ui, Point::new(from.x + b.width() * 0.6, from.y), PointerPhase::Move, Buttons::PRIMARY);
    pointer(&ui, Point::new(from.x + b.width() * 0.6, from.y), PointerPhase::Up, Buttons(0));
    pump_frame(&ui, &clock, 800);
    assert_eq!(index.get_untracked().ok(), Some(2));
}

#[test]
fn autoplay_moves_on_by_itself_and_can_be_paused_and_held() {
    let c = car(3, true, Some(Duration::from_secs(4)));
    let every = Duration::from_secs(4);
    let t = timers_after(&c.ui, every);
    assert_eq!(t.len(), 1, "one timer is waiting");
    for id in t {
        c.ui.fire_timer(id);
    }
    assert_eq!(c.index.get_untracked().ok(), Some(1));
    let t = timers_after(&c.ui, every);
    assert_eq!(t.len(), 1, "and one for the next page");
    // The pointer over the carousel holds it.
    pointer(&c.ui, center(&c.root), PointerPhase::Move, Buttons(0));
    let again = timers_after(&c.ui, every);
    for id in &again {
        c.ui.fire_timer(*id);
    }
    assert_eq!(c.index.get_untracked().ok(), Some(1), "held while hovered");
    // Pause button.
    let pause = button_named(&c.root, "Pause automatic page changes");
    click(&c.ui, center(&pause));
    assert_eq!(name_of(&button_named(&c.root, "Play automatic page changes")), "Play automatic page changes");
    pointer(&c.ui, Point::new(470.0, 630.0), PointerPhase::Move, Buttons(0));
    for id in timers_after(&c.ui, every) {
        c.ui.fire_timer(id);
    }
    assert_eq!(c.index.get_untracked().ok(), Some(1), "paused");
    assert!(audit(&c.ui).is_empty(), "{:?}", audit(&c.ui));
}

#[test]
fn autoplay_never_runs_under_reduced_motion() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let index = Signal::new(0usize);
    let pages: Vec<El> = (0..3).map(|i| w::text(&ui, format!("Slide {i}"))).collect();
    let root = carousel(&ui, "Highlights", pages, index, {
        let mut v_ = CarouselOptions::default();
        v_.autoplay = Some(Duration::from_secs(2));
        v_
    });
    mount(&ui, &root);
    assert!(timers(&ui).is_empty());
}

#[test]
fn a_slide_between_pages_of_different_heights_grows_and_shrinks_smoothly() {
    let ui = ui();
    let clock = Clock::manual();
    let index = Signal::new(0usize);
    let pages: Vec<El> = [40.0f32, 160.0, 90.0].iter().map(|h| ui.el("block").height(*h)).collect();
    let root = carousel(&ui, "Tall and short", pages, index, {
        let mut v_ = CarouselOptions::default();
        v_.clock = clock.clone();
        v_
    });
    mount(&ui, &root);
    let _ = frame(&ui);
    let view = first(&root, "carousel_viewport");
    let height = || view.bounds().map_or(0.0, |b| b.height());
    assert!((height() - 40.0).abs() < 0.5, "as tall as its page at rest: {}", height());
    // Growing: every step is at least as tall as the one before, never past the tall page,
    // and it takes several steps.
    index.set(1);
    let mut seen = vec![height()];
    for _ in 0..60 {
        pump_frame(&ui, &clock, 16);
        seen.push(height());
    }
    assert!(seen.windows(2).all(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| b + 0.5 >= *a)), "{seen:?}");
    assert!(seen.iter().all(|h| *h <= 160.5), "{seen:?}");
    let between = seen.iter().filter(|h| **h > 45.0 && **h < 155.0).count();
    assert!(between >= 3, "in between for several frames: {between} {seen:?}");
    assert!((height() - 160.0).abs() < 0.5, "settles at the tall page: {}", height());
    // Shrinking the same way.
    index.set(0);
    let mut seen = vec![height()];
    for _ in 0..60 {
        pump_frame(&ui, &clock, 16);
        seen.push(height());
    }
    assert!(
        seen.windows(2).all(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| *b <= *a + 0.5)),
        "{seen:?}"
    );
    assert!(seen.iter().filter(|h| **h > 45.0 && **h < 155.0).count() >= 3, "{seen:?}");
    assert!((height() - 40.0).abs() < 0.5);
    // Under reduced motion the height changes at once.
    ui.set_reduced_motion(true);
    index.set(1);
    pump_frame(&ui, &clock, 16);
    assert!((height() - 160.0).abs() < 0.5, "{}", height());
    index.set(2);
    pump_frame(&ui, &clock, 16);
    assert!((height() - 90.0).abs() < 0.5, "{}", height());
}

// --- stepper_flow ---

struct Flow {
    ui: Ui,
    root: El,
    current: Signal<usize>,
    name_ok: Signal<bool>,
    agreed: Signal<bool>,
    finished: Rc<Cell<u32>>,
}

fn flow() -> Flow {
    let ui = ui();
    let current = Signal::new(0usize);
    let name_ok = Signal::new(false);
    let agreed = Signal::new(false);
    let finished = Rc::new(Cell::new(0u32));
    let f2 = finished.clone();
    let steps = vec![
        Step::new("Welcome", w::text(&ui, "Welcome to setup")),
        Step::new("Name", w::text_input(&ui, "Device name").el()).description("Pick a name").valid(name_ok),
        Step::new("Extras", w::text(&ui, "Optional things")).optional(true).valid(Signal::new(false)),
        Step::new("Terms", w::check(&ui, "I agree", agreed)).valid(agreed),
    ];
    let root =
        stepper_flow(&ui, "Setup", steps, current, StepperLabels::default(), move || f2.set(f2.get() + 1));
    mount(&ui, &root);
    Flow { ui, root, current, name_ok, agreed, finished }
}

fn cur(f: &Flow) -> usize {
    f.current.get_untracked().unwrap_or(99)
}

#[test]
fn next_waits_for_the_step_to_be_valid() {
    let f = flow();
    assert_eq!(cur(&f), 0);
    assert_eq!(value_text(&f.root), "Step 1 of 4: Welcome");
    let next = button_named(&f.root, "Next");
    let back = button_named(&f.root, "Back");
    assert!(!disabled(&next) && disabled(&back));
    click(&f.ui, center(&next));
    assert_eq!(cur(&f), 1);
    assert_eq!(value_text(&f.root), "Step 2 of 4: Name");
    // The step is not valid yet: Next is off and a click does nothing.
    assert!(disabled(&button_named(&f.root, "Next")));
    click(&f.ui, center(&button_named(&f.root, "Next")));
    assert_eq!(cur(&f), 1);
    f.name_ok.set(true);
    let _ = frame(&f.ui);
    assert!(!disabled(&button_named(&f.root, "Next")));
    click(&f.ui, center(&button_named(&f.root, "Next")));
    assert_eq!(cur(&f), 2);
    // Back always works.
    click(&f.ui, center(&button_named(&f.root, "Back")));
    assert_eq!(cur(&f), 1);
    // Invalidating a step after the fact blocks leaving it again.
    f.name_ok.set(false);
    let _ = frame(&f.ui);
    assert!(disabled(&button_named(&f.root, "Next")));
}

#[test]
fn only_the_current_step_is_shown() {
    let f = flow();
    let titles = |f: &Flow| -> Vec<bool> {
        find(&f.root, "heading").iter().map(|h| h.parent().is_some_and(|p| !hidden(&p))).collect()
    };
    assert_eq!(titles(&f), [true, false, false, false]);
    click(&f.ui, center(&button_named(&f.root, "Next")));
    assert_eq!(titles(&f), [false, true, false, false]);
    assert!(
        find(&f.root, "label")
            .iter()
            .any(|l| l.info().and_then(|i| i.text).as_deref() == Some("Pick a name"))
    );
}

#[test]
fn markers_show_progress_and_gate_jumping_ahead() {
    let f = flow();
    let markers = find(&f.root, "step_marker_cell");
    let names: Vec<String> = markers.iter().map(name_of).collect();
    assert_eq!(names.first().map(String::as_str), Some("Step 1: Welcome, current"));
    assert_eq!(names.get(1).map(String::as_str), Some("Step 2: Name, not started"));
    assert_eq!(
        names.get(2).map(String::as_str),
        Some("Step 3: Extras, locked"),
        "the name step is not valid, so nothing past it opens"
    );
    f.name_ok.set(true);
    let _ = frame(&f.ui);
    // Jump ahead through valid steps by pressing a marker.
    click(&f.ui, center(markers.get(1).unwrap_or_else(|| unreachable!("marker"))));
    assert_eq!(cur(&f), 1);
    let names: Vec<String> = find(&f.root, "step_marker_cell").iter().map(name_of).collect();
    assert_eq!(names.first().map(String::as_str), Some("Step 1: Welcome, completed"));
    assert_eq!(names.get(1).map(String::as_str), Some("Step 2: Name, current"));
    // A locked step does not open.
    click(&f.ui, center(markers.get(3).unwrap_or_else(|| unreachable!("marker"))));
    assert_eq!(cur(&f), 1, "step 3 is not valid, so step 4 is locked");
    // A completed step does.
    click(&f.ui, center(markers.first().unwrap_or_else(|| unreachable!("marker"))));
    assert_eq!(cur(&f), 0);
    assert!(markers.iter().all(|m| m.bounds().is_some_and(|b| b.height() >= 24.0)));
}

#[test]
fn the_progress_bar_follows_the_step() {
    let f = flow();
    let bar = || find(&f.root, "progress").into_iter().next().unwrap_or_else(|| unreachable!("progress"));
    let numeric = |e: &El| e.info().and_then(|i| i.semantics).map(|s| format!("{s:?}")).unwrap_or_default();
    let before = numeric(&bar());
    click(&f.ui, center(&button_named(&f.root, "Next")));
    assert_ne!(before, numeric(&bar()), "the bar moved");
    assert_eq!(name_of(&bar()), "Setup progress");
}

#[test]
fn an_optional_step_can_be_skipped_and_the_last_step_finishes() {
    let f = flow();
    f.name_ok.set(true);
    click(&f.ui, center(&button_named(&f.root, "Next")));
    click(&f.ui, center(&button_named(&f.root, "Next")));
    assert_eq!(cur(&f), 2);
    // The extras step is invalid forever, but optional.
    assert!(disabled(&button_named(&f.root, "Next")));
    let skip = button_named(&f.root, "Skip");
    assert!(!hidden(&skip));
    click(&f.ui, center(&skip));
    assert_eq!(cur(&f), 3);
    // The last step: Finish instead of Next, off until the terms are agreed.
    assert!(hidden(&button_named(&f.root, "Next")) && hidden(&button_named(&f.root, "Skip")));
    let finish = button_named(&f.root, "Finish");
    assert!(!hidden(&finish) && disabled(&finish));
    click(&f.ui, center(&finish));
    assert_eq!(f.finished.get(), 0, "not valid yet");
    f.agreed.set(true);
    let _ = frame(&f.ui);
    click(&f.ui, center(&button_named(&f.root, "Finish")));
    assert_eq!(f.finished.get(), 1);
    // The pointer away, so no hover color is audited.
    pointer(&f.ui, Point::new(470.0, 630.0), PointerPhase::Move, Buttons(0));
    assert!(audit(&f.ui).is_empty(), "{:?}", audit(&f.ui));
}

#[test]
fn focus_moves_into_the_new_step() {
    let f = flow();
    click(&f.ui, center(&button_named(&f.root, "Next")));
    let input = first(&f.root, "text_input");
    let focused = input.is_focused() || find(&input, "text_input").iter().any(El::is_focused);
    assert!(focused, "the name field has focus on the Name step");
}

#[test]
fn alt_arrows_go_back_and_on_when_allowed() {
    let f = flow();
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    button_named(&f.root, "Next").focus();
    assert!(key_mod(&f.ui, Key::ArrowRight, alt));
    assert_eq!(cur(&f), 1);
    key_mod(&f.ui, Key::ArrowRight, alt);
    assert_eq!(cur(&f), 1, "not valid yet");
    f.name_ok.set(true);
    key_mod(&f.ui, Key::ArrowRight, alt);
    assert_eq!(cur(&f), 2);
    key_mod(&f.ui, Key::ArrowLeft, alt);
    assert_eq!(cur(&f), 1);
    key(&f.ui, Key::ArrowRight);
    assert_eq!(cur(&f), 1, "plain arrows are left to the step's content");
}

#[test]
fn the_flow_reads_its_step_and_announces_changes() {
    let f = flow();
    assert_eq!(role_of(&f.root), Some(Role::Group));
    assert_eq!(f.root.info().and_then(|i| i.semantics).map(|s| s.live_state()), Some(Live::Polite));
    let status = find(&f.root, "label")
        .into_iter()
        .find(|l| l.info().and_then(|i| i.text).is_some_and(|t| plain(t).starts_with("Step 1 of")));
    assert!(status.is_some(), "the step number is on screen too");
}

#[test]
fn custom_button_words_are_used() {
    let ui = ui();
    let current = Signal::new(0usize);
    let steps = vec![Step::new("Eins", w::text(&ui, "a")), Step::new("Zwei", w::text(&ui, "b"))];
    let labels = {
        let mut v_ = StepperLabels::default();
        v_.back = "Zur\u{fc}ck".into();
        v_.next = "Weiter".into();
        v_.skip = "\u{dc}berspringen".into();
        v_.finish = "Fertig".into();
        v_
    };
    let root = stepper_flow(&ui, "Einrichtung", steps, current, labels, || {});
    mount(&ui, &root);
    assert!(!hidden(&button_named(&root, "Weiter")));
    assert_eq!(name_of(&button_named(&root, "Zur\u{fc}ck")), "Zur\u{fc}ck");
    click(&ui, center(&button_named(&root, "Weiter")));
    assert!(!hidden(&button_named(&root, "Fertig")));
}

#[test]
fn the_carousel_and_the_flow_follow_the_window_language() {
    let c = car(3, false, Some(Duration::from_secs(5)));
    c.ui.set_locale("de");
    let _ = frame(&c.ui);
    assert_eq!(value_text(&c.root), "Seite 1 von 3");
    let _ = button_named(&c.root, "Vorherige Seite");
    let _ = button_named(&c.root, "Automatischen Seitenwechsel anhalten");

    let f = flow();
    f.ui.set_locale("de");
    let _ = frame(&f.ui);
    assert_eq!(value_text(&f.root), "Schritt 1 von 4: Welcome");
    let _ = button_named(&f.root, "Weiter");
    assert_eq!(StepperLabels::localized("fr-CA").next, "Suivant");
}
