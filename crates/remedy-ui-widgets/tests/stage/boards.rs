//! Timelines and display boards on a real window: virtualization, scrubbing, flow, the
//! marquee and flaps, reduced motion and the audit.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::*;
use remedy_ui_core::input::Key;
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, RichText, Ui};
use remedy_ui_widgets::stage::{
    Board, DisplayOptions, Marquee, NewestAt, Timeline, TimelineOptions, display_text, timeline,
};
use remedy_ui_widgets::viz::Clock;

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(ui.el("holder").padding(20.0).child(el.clone()));
    let _ = frame(ui);
    let _ = frame(ui);
}

type Item = (f64, u64);

fn history(n: u64) -> Vec<Item> {
    (0..n).map(|i| (i as f64 * 10.0, i)).collect()
}

fn stone(ui: &Ui, it: &Item) -> El {
    ui.el("stone").width(24.0).height(24.0).text(RichText::plain(format!("m{}", it.1))).data_text()
}

fn track(ui: &Ui, items: Signal<Vec<Item>>, clock: &Clock, newest: NewestAt) -> Timeline<Item> {
    let ui2 = ui.clone();
    timeline(
        ui,
        "Messages",
        items,
        |it: &Item| it.0,
        |it: &Item| it.1,
        move |it: &Item| stone(&ui2, it),
        TimelineOptions::default().scale(1.0).lead(60.0).newest_at(newest).clock(clock.clone()),
    )
}

fn item_el(t: &Timeline<Item>, key: u64) -> Option<El> {
    let content = t.el().child_elements().into_iter().next()?;
    let pos = t.shown_keys().iter().position(|k| *k == key)?;
    content.child_elements().get(pos).and_then(|slot| slot.child_elements().into_iter().next())
}

#[test]
fn a_timeline_builds_only_what_is_near_the_view() {
    let ui = ui();
    let clock = Clock::manual();
    let items = Signal::new(history(1000));
    let t = track(&ui, items, &clock, NewestAt::Right);
    let el: El = t.clone().into();
    mount(&ui, &el.height(60.0));
    let keys = t.shown_keys();
    assert!(keys.len() < 60, "virtualized: {}", keys.len());
    assert_eq!(keys.last(), Some(&999));
    assert!(keys.windows(2).all(|w| w.first() < w.last()), "time order");
    // The newest ends at the live edge: the track's width less the lead.
    let b = t.el().bounds().unwrap_or_default();
    let newest = item_el(&t, 999).and_then(|e| e.bounds()).unwrap_or_default();
    assert!((newest.max.x - (b.max.x - 60.0)).abs() < 0.75, "{newest:?} {b:?}");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn scrubbing_moves_back_through_history_and_asks_for_more() {
    let ui = ui();
    let clock = Clock::manual();
    let items = Signal::new(history(100));
    let asked = Rc::new(Cell::new(0));
    let a2 = asked.clone();
    let t = track(&ui, items, &clock, NewestAt::Right).on_near_oldest(move || a2.set(a2.get() + 1));
    let el: El = t.clone().into();
    mount(&ui, &el.height(60.0));
    let x0 = item_el(&t, 99).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    t.el().focus();
    assert!(key(&ui, Key::ArrowLeft));
    let quarter = f64::from(t.el().bounds().unwrap_or_default().width()) / 4.0;
    assert!((t.back().get_untracked().unwrap_or(0.0) - quarter).abs() < 1e-3);
    let _ = frame(&ui);
    let x1 = item_el(&t, 99).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    assert!((x1 - x0 - quarter as f32).abs() < 0.75, "older is to the left, so the view moves right");
    assert_eq!(asked.get(), 0);
    assert!(key(&ui, Key::Home));
    assert_eq!(t.back().get_untracked().ok(), Some(990.0));
    assert!(t.shown_keys().contains(&0));
    pump(&ui, &clock, 16);
    assert_eq!(asked.get(), 1, "near the oldest: ask for history once");
    let _ = frame(&ui);
    pump(&ui, &clock, 16);
    assert_eq!(asked.get(), 1);
    assert!(key(&ui, Key::End));
    assert_eq!(t.back().get_untracked().ok(), Some(0.0));
}

#[test]
fn new_items_flow_in_live_and_leave_a_scrubbed_view_alone() {
    let ui = ui();
    let clock = Clock::manual();
    let items = Signal::new(history(10));
    let t = track(&ui, items, &clock, NewestAt::Right);
    let el: El = t.clone().into();
    mount(&ui, &el.height(60.0));
    let edge = t.el().bounds().unwrap_or_default().max.x - 60.0;
    items.update(|v| v.push((100.0, 10)));
    let _ = (frame(&ui), frame(&ui));
    // It glides in from where the old edge was, then rests at the edge.
    let first = item_el(&t, 10).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    assert!(first > edge + 1.0, "starts beyond the edge: {first} {edge}");
    assert!(pump(&ui, &clock, 1500) > 0);
    let _ = frame(&ui);
    let rest = item_el(&t, 10).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    assert!((rest - edge).abs() < 0.75, "{rest} {edge}");
    assert_eq!(pending_timers(&ui, &clock), 0, "at rest");

    // Scrubbed back, the view stays put as items arrive.
    t.back().set(50.0);
    let _ = frame(&ui);
    let x = item_el(&t, 5).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    items.update(|v| v.push((130.0, 11)));
    let _ = (frame(&ui), frame(&ui));
    assert_eq!(t.back().get_untracked().ok(), Some(80.0));
    let x2 = item_el(&t, 5).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    assert!((x - x2).abs() < 0.75);

    // Under reduced motion, live items jump in.
    t.go_live();
    ui.set_reduced_motion(true);
    let _ = frame(&ui);
    items.update(|v| v.push((160.0, 12)));
    let _ = (frame(&ui), frame(&ui));
    let at = item_el(&t, 12).and_then(|e| e.bounds()).unwrap_or_default().max.x;
    assert!((at - edge).abs() < 0.75);
    assert_eq!(pending_timers(&ui, &clock), 0);
}

#[test]
fn newest_at_the_left_mirrors_time() {
    let ui = ui();
    let clock = Clock::manual();
    let items = Signal::new(history(5));
    let t = track(&ui, items, &clock, NewestAt::Left);
    let el: El = t.clone().into();
    mount(&ui, &el.height(60.0));
    let b = t.el().bounds().unwrap_or_default();
    let newest = item_el(&t, 4).and_then(|e| e.bounds()).unwrap_or_default();
    let older = item_el(&t, 3).and_then(|e| e.bounds()).unwrap_or_default();
    assert!((newest.min.x - (b.min.x + 60.0)).abs() < 0.75);
    assert!(older.min.x > newest.min.x + 9.0);
}

// --- display boards ---

fn line(el: &El) -> El {
    el.child_elements().into_iter().next().unwrap_or_else(|| el.clone())
}

#[test]
fn board_text_is_real_text_padded_to_its_cells() {
    let ui = ui();
    let text = Signal::new("GATE 4".to_owned());
    let el = display_text(&ui, text, DisplayOptions::default().board(Board::SplitFlap).cells(8));
    mount(&ui, &el);
    let names: Vec<String> = ui.reader_view().iter().filter_map(|n| n.name.clone()).collect();
    assert!(names.iter().any(|n| n.starts_with("GATE 4")), "{names:?}");
    let w = line(&el).bounds().unwrap_or_default().width();
    text.set("GATE 12".to_owned());
    let _ = (frame(&ui), frame(&ui));
    assert!(
        (line(&el).bounds().unwrap_or_default().width() - w).abs() < 0.5,
        "cells keep the width {w} {:?}",
        line(&el).bounds()
    );
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn flaps_flip_then_rest() {
    let ui = ui();
    let clock = Clock::manual();
    let text = Signal::new("ON TIME".to_owned());
    let el = display_text(&ui, text, DisplayOptions::default().board(Board::SplitFlap).clock(clock.clone()));
    mount(&ui, &el);
    assert_eq!(pending_timers(&ui, &clock), 0, "a still board does nothing");
    text.set("DELAYED".to_owned());
    let _ = frame(&ui);
    assert!(pump(&ui, &clock, 100) > 0, "flipping");
    pump(&ui, &clock, 1000);
    assert_eq!(pending_timers(&ui, &clock), 0, "at rest");
}

#[test]
fn long_text_scrolls_as_a_marquee_but_not_under_reduced_motion() {
    let ui = ui();
    let clock = Clock::manual();
    let el = display_text(
        &ui,
        "Now boarding: General, Lounge, Stage and Off-topic".to_owned(),
        DisplayOptions::default().clock(clock.clone()),
    )
    .width(120.0);
    mount(&ui, &el);
    let x0 = line(&el).bounds().unwrap_or_default().min.x;
    pump(&ui, &clock, 1500 + 500);
    let _ = frame(&ui);
    let x1 = line(&el).bounds().unwrap_or_default().min.x;
    assert!(x1 < x0 - 5.0, "scrolled: {x0} {x1}");
    ui.set_reduced_motion(true);
    let _ = frame(&ui);
    pump(&ui, &clock, 50);
    let _ = frame(&ui);
    assert_eq!(line(&el).bounds().unwrap_or_default().min.x, x0, "back at the start");
    assert_eq!(pending_timers(&ui, &clock), 0);

    // Short text, or the marquee off: nothing moves.
    let ui = common::ui();
    let clock = Clock::manual();
    let el = display_text(&ui, "On time".to_owned(), DisplayOptions::default().clock(clock.clone()));
    mount(&ui, &el);
    assert_eq!(pending_timers(&ui, &clock), 0);
    let ui = common::ui();
    let clock = Clock::manual();
    let el = display_text(
        &ui,
        "A long line that does not fit".to_owned(),
        DisplayOptions::default().marquee(Marquee::Off).clock(clock.clone()),
    )
    .width(60.0);
    mount(&ui, &el);
    assert_eq!(pending_timers(&ui, &clock), 0);
}
