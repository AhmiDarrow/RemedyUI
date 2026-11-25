//! Chat lists on a real view: following the latest, reading higher up, older messages loaded
//! above, day headers, and what screen readers get.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_core::Vec2;
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_widgets::base as w;
use remedy_ui_widgets::data::*;

mod common;
use common::*;

#[derive(Clone, Debug, PartialEq)]
struct Msg {
    id: u32,
    day: &'static str,
    text: String,
}

fn msgs(range: std::ops::Range<u32>) -> Vec<Msg> {
    range
        .map(|i| Msg {
            id: i,
            day: if i < 1000 { "Monday" } else { "Today" },
            // Different lengths, so rows have different heights.
            text: format!("message {i} {}", "and more ".repeat((i % 5) as usize)),
        })
        .collect()
}

struct Chat {
    ui: Ui,
    list: El,
    items: Signal<Vec<Msg>>,
    state: ChatState,
    asked: Rc<Cell<u32>>,
}

fn chat(items: Vec<Msg>) -> Chat {
    let ui = ui();
    let items = Signal::new(items);
    let state = ChatState::new();
    let asked = Rc::new(Cell::new(0));
    let a2 = asked.clone();
    let list = chat_list(
        &ui,
        "Messages",
        items,
        |m: &Msg| m.id,
        |m: &Msg| m.day.to_owned(),
        40.0,
        |ui: &Ui, m: &Msg| w::text(ui, m.text.clone()).selectable(true),
        &state,
        move || a2.set(a2.get() + 1),
    );
    let holder = ui.el("col").column().height(600.0).fill_width().child(list.clone());
    ui.root().child(holder);
    for _ in 0..6 {
        frame(&ui);
    }
    Chat { ui, list, items, state, asked }
}

fn shown(c: &Chat) -> Vec<String> {
    texts(&c.list).into_iter().filter(|t| t.starts_with("message")).collect()
}

fn body(c: &Chat) -> El {
    find(&c.list, "scroll").into_iter().next().unwrap_or_else(|| unreachable!("the scroller"))
}

/// The message texts fully inside the view.
fn in_view(c: &Chat) -> Vec<String> {
    let view = body(c).bounds().unwrap_or_default();
    find(&c.list, "label")
        .into_iter()
        .filter(|l| l.bounds().is_some_and(|b| b.min.y >= view.min.y - 0.5 && b.max.y <= view.max.y + 0.5))
        .filter_map(|l| l.info().and_then(|i| i.text))
        .filter(|t| t.starts_with("message"))
        .collect()
}

#[test]
fn a_chat_starts_and_stays_at_the_latest_message() {
    let c = chat(msgs(1000..1300));
    assert!(c.state.at_end.get_untracked().unwrap_or(false));
    assert!(in_view(&c).iter().any(|t| t.starts_with("message 1299")), "{:?}", in_view(&c));
    assert!(shown(&c).len() < 120, "only rows near the view are built: {}", shown(&c).len());
    // A new message while at the end: it shows, and the view follows.
    c.items.update(|v| v.extend(msgs(1300..1301)));
    for _ in 0..4 {
        frame(&c.ui);
    }
    assert!(in_view(&c).iter().any(|t| t.starts_with("message 1300")), "{:?}", in_view(&c));
    assert_eq!(c.state.unread.get_untracked().ok(), Some(0));
}

#[test]
fn reading_higher_up_counts_what_arrives_and_jumps_back() {
    let c = chat(msgs(1000..1300));
    let b = body(&c);
    let y = c.ui.scroll_offset(b.id()).y;
    c.ui.set_scroll_offset(b.id(), Vec2::new(0.0, (y - 2000.0).max(0.0)));
    for _ in 0..4 {
        frame(&c.ui);
    }
    assert_eq!(c.state.at_end.get_untracked().ok(), Some(false));
    let before = in_view(&c);
    c.items.update(|v| v.extend(msgs(1300..1303)));
    for _ in 0..4 {
        frame(&c.ui);
    }
    assert_eq!(c.state.unread.get_untracked().ok(), Some(3));
    assert_eq!(in_view(&c), before, "the view stays on what was being read");
    let jump = find(&c.list, "button").into_iter().next().unwrap_or_else(|| unreachable!("the jump button"));
    assert!(jump.info().is_some_and(|i| !i.hidden), "the jump button shows");
    let label = find(&jump, "button_label").first().and_then(|l| l.info()).and_then(|i| i.text);
    // Numbers in messages come wrapped in isolation marks (for right-to-left text).
    let plain = |s: Option<String>| s.map(|s| s.replace(['\u{2068}', '\u{2069}'], ""));
    assert_eq!(plain(label).as_deref(), Some("3 new"));
    let spoken = jump.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
    assert_eq!(plain(spoken.clone()).as_deref(), Some("3 new messages, jump to latest"));
    c.ui.set_locale("de");
    frame(&c.ui);
    let spoken = jump.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
    assert_eq!(plain(spoken).as_deref(), Some("3 neue Nachrichten, zu den neuesten springen"));
    c.ui.set_locale("en");
    frame(&c.ui);
    c.state.jump_to_latest();
    for _ in 0..4 {
        frame(&c.ui);
    }
    assert!(in_view(&c).iter().any(|t| t.starts_with("message 1302")), "{:?}", in_view(&c));
    assert_eq!(c.state.unread.get_untracked().ok(), Some(0));
}

#[test]
fn older_messages_load_above_without_moving_the_view() {
    let c = chat(msgs(1000..1040));
    // Read from the top: the list asks for older messages.
    let b = body(&c);
    c.ui.set_scroll_offset(b.id(), Vec2::new(0.0, 0.0));
    for _ in 0..4 {
        frame(&c.ui);
    }
    assert!(c.asked.get() >= 1, "it asked for older messages");
    let before = in_view(&c);
    let mut older = msgs(900..1000);
    older.extend(c.items.get_untracked().unwrap_or_default());
    c.items.set(older);
    for _ in 0..6 {
        frame(&c.ui);
    }
    assert_eq!(in_view(&c).first(), before.first(), "the same message stays at the top of the view");
    // The days get their headers; the earlier day is above.
    assert!(texts(&c.list).iter().any(|t| t == "Today"));
}

#[test]
fn messages_read_well_and_select_across_rows() {
    let c = chat(msgs(1000..1030));
    let found = audit(&c.ui);
    assert!(found.is_empty(), "{found:#?}");
    let rows = find(&c.list, "chat_row");
    let headers: Vec<String> = rows
        .iter()
        .flat_map(|r| find(r, "label"))
        .filter_map(|l| l.info().and_then(|i| i.semantics))
        .filter(|s| matches!(s.role(), remedy_ui_core::a11y::Role::Heading(_)))
        .filter_map(|s| s.name().map(str::to_owned))
        .collect();
    assert_eq!(headers, ["Today"], "one header for the day");
}

#[test]
fn a_chat_follows_a_signal_vec_as_the_other_lists_do() {
    let ui = ui();
    let items = remedy_ui_core::reactive::SignalVec::new(msgs(1000..1100));
    let state = ChatState::new();
    let list = chat_list(
        &ui,
        "Messages",
        items,
        |m: &Msg| m.id,
        |m: &Msg| m.day.to_owned(),
        40.0,
        |ui: &Ui, m: &Msg| w::text(ui, m.text.clone()).selectable(true),
        &state,
        || {},
    );
    ui.root().child(ui.el("col").column().height(600.0).fill_width().child(list.clone()));
    for _ in 0..6 {
        frame(&ui);
    }
    let c =
        Chat { ui: ui.clone(), list, items: Signal::new(Vec::new()), state, asked: Rc::new(Cell::new(0)) };
    assert!(in_view(&c).iter().any(|t| t.starts_with("message 1099")), "{:?}", in_view(&c));
    // A pushed message shows, and the view follows it.
    let Some(next) = msgs(1100..1101).pop() else { unreachable!("one message") };
    assert!(items.push(next).is_ok());
    for _ in 0..4 {
        frame(&ui);
    }
    assert!(in_view(&c).iter().any(|t| t.starts_with("message 1100")), "{:?}", in_view(&c));
    assert_eq!(c.state.unread.get_untracked().ok(), Some(0));
}
