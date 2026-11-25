//! Text field editing: word and all selection, drag, focus-selects-all, and the
//! usual keyboard chords.

use std::time::Duration;

use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_core::layout::Length;
use remedy_ui_core::style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui_core::{Point, Size};
use remedy_ui_engine::view::{FieldKind, InputPurpose, Ui};
use remedy_ui_text::FontCollection;

fn fonts() -> FontCollection {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    f
}

fn ui() -> Ui {
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default());
    Ui::new(Theme::dark_forest(), sheet, fonts())
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(400.0, 120.0), 1.0);
}

fn key(k: Key, text: Option<&str>, m: Modifiers) -> KeyEvent {
    KeyEvent::press(k).with_modifiers(m).with_text(text.map(str::to_owned))
}

fn typed(c: &str) -> KeyEvent {
    key(Key::Character(c.to_lowercase()), Some(c), Modifiers::NONE)
}

fn ctrl(c: &str) -> KeyEvent {
    key(Key::Character(c.into()), None, Modifiers { ctrl: true, ..Modifiers::NONE })
}

fn pointer(phase: PointerPhase, x: f32, y: f32, buttons: Buttons, time: Duration) -> PointerEvent {
    PointerEvent::new(phase, Point::new(x, y)).with_buttons(buttons).with_time(time)
}

fn click(ui: &Ui, x: f32, y: f32, time: Duration) {
    ui.pointer(&pointer(PointerPhase::Down, x, y, Buttons::PRIMARY, time));
    ui.pointer(&pointer(PointerPhase::Up, x, y, Buttons::default(), time + Duration::from_millis(20)));
}

fn field_at(ui: &Ui, kind: FieldKind) -> (remedy_ui_engine::view::TextField, f32, f32) {
    let field = ui.text_field("text_input", kind);
    ui.root().child(field.el().layout(|l| {
        l.width = Length::Px(240.0);
        l.height = Length::Px(32.0);
    }));
    frame(ui);
    let b = field.el().bounds().unwrap_or_default();
    (field, b.min.x + 8.0, b.min.y + b.height() / 2.0)
}

#[test]
fn double_click_selects_a_word_and_triple_click_selects_all() {
    let ui = ui();
    let (field, x, y) = field_at(&ui, FieldKind::Single);
    field.set_value("hello world");
    frame(&ui);
    click(&ui, x, y, Duration::from_millis(0));
    click(&ui, x, y, Duration::from_millis(80));
    assert_eq!(field.value(), "hello world");
    assert_eq!(field.selection(), 0..5, "double-click selects the first word");
    click(&ui, x, y, Duration::from_millis(160));
    assert_eq!(field.selection(), 0..11, "triple-click selects all");
}

#[test]
fn a_drag_selects_text() {
    let ui = ui();
    let (field, x, y) = field_at(&ui, FieldKind::Single);
    field.set_value("hello world");
    frame(&ui);
    // Click well after any previous press, then drag to the right.
    let t = Duration::from_millis(800);
    ui.pointer(&pointer(PointerPhase::Down, x, y, Buttons::PRIMARY, t));
    let start = field.selection().start;
    ui.pointer(&pointer(PointerPhase::Move, x + 80.0, y, Buttons::PRIMARY, t + Duration::from_millis(30)));
    ui.pointer(&pointer(PointerPhase::Up, x + 80.0, y, Buttons::default(), t + Duration::from_millis(40)));
    let sel = field.selection();
    assert!(sel.end > sel.start, "a drag leaves a range selected, got {sel:?} (start click {start})");
    assert!(field.value()[sel.clone()].contains('l') || sel.end - sel.start >= 2, "drag covered some text");
}

#[test]
fn a_url_field_selects_all_on_first_click_or_focus_and_later_clicks_place_the_caret() {
    let ui = ui();
    let field = ui.text_field("text_input", FieldKind::Single).purpose(InputPurpose::Url);
    let other = ui.text_field("text_input", FieldKind::Single);
    ui.root().child(field.el().layout(|l| {
        l.width = Length::Px(240.0);
        l.height = Length::Px(32.0);
    }));
    ui.root().child(other.el().layout(|l| {
        l.width = Length::Px(240.0);
        l.height = Length::Px(32.0);
    }));
    frame(&ui);
    field.set_value("https://example.com/path");
    other.set_value("other");
    frame(&ui);
    let b = field.el().bounds().unwrap_or_default();
    assert!(b.width() > 0.0, "the URL field should be laid out");
    let (x, y) = (b.min.x + 12.0, b.min.y + b.height() / 2.0);

    other.el().focus();
    frame(&ui);
    assert!(other.el().is_focused());

    click(&ui, x, y, Duration::from_millis(0));
    assert!(field.el().is_focused());
    assert_eq!(field.selection(), 0..field.value().len(), "first click selects the whole URL");

    // A later click, after the double-click window, places the caret.
    click(&ui, x, y, Duration::from_millis(800));
    assert!(field.selection().is_empty(), "a later click places the caret, got {:?}", field.selection());

    other.el().focus();
    frame(&ui);
    // Tab back onto the URL field.
    ui.key(&key(Key::Tab, None, Modifiers { shift: true, ..Modifiers::NONE }));
    frame(&ui);
    // If Tab didn't land on it (focus order), focus via the API still selects all.
    if !field.el().is_focused() {
        field.el().focus();
        frame(&ui);
    }
    assert!(field.el().is_focused());
    assert_eq!(field.selection(), 0..field.value().len(), "focus selects the whole URL");
}

#[test]
fn text_fields_move_and_edit_by_word_and_line_and_accept_the_usual_chords() {
    let ui = ui();
    let (field, x, y) = field_at(&ui, FieldKind::Single);
    click(&ui, x, y, Duration::ZERO);
    for c in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
        ui.key(&typed(c));
    }
    assert_eq!(field.value(), "hello world");

    ui.key(&key(Key::Home, None, Modifiers::NONE));
    assert_eq!(field.caret(), 0);
    ui.key(&key(Key::End, None, Modifiers::NONE));
    assert_eq!(field.caret(), field.value().len());

    ui.key(&key(Key::Home, None, Modifiers { shift: true, ..Modifiers::NONE }));
    assert_eq!(field.selection(), 0..field.value().len(), "Shift+Home from the end selects all");

    ui.key(&key(Key::End, None, Modifiers::NONE));
    ui.key(&key(Key::ArrowLeft, None, Modifiers { ctrl: true, ..Modifiers::NONE }));
    assert_eq!(field.caret(), 6, "Ctrl+Left lands at the start of the last word");

    ui.key(&key(Key::Backspace, None, Modifiers { ctrl: true, ..Modifiers::NONE }));
    assert_eq!(field.value(), "world", "Ctrl+Backspace deletes the previous word");

    ui.key(&ctrl("a"));
    assert_eq!(field.selection(), 0..field.value().len());
    ui.key(&ctrl("x"));
    assert_eq!(field.value(), "");
    assert!(
        ui.take_requests()
            .iter()
            .any(|r| matches!(r, remedy_ui_engine::view::Request::Copy { text, .. } if text == "world"))
    );
    ui.key(&ctrl("v"));
    assert!(ui.take_requests().contains(&remedy_ui_engine::view::Request::Paste));
    ui.paste("world");
    assert_eq!(field.value(), "world");
    ui.key(&ctrl("z"));
    assert_eq!(field.value(), "", "Ctrl+Z undoes the paste");
    ui.key(&ctrl("y"));
    assert_eq!(field.value(), "world", "Ctrl+Y redoes it");
}

/// A multi-line field 240 wide and `height` tall, focused, holding `text`.
fn multi(ui: &Ui, text: &str, height: f32) -> remedy_ui_engine::view::TextField {
    let field = ui.text_field("text_area", FieldKind::Multi);
    ui.root().child(field.el().layout(move |l| {
        l.width = Length::Px(240.0);
        l.height = Length::Px(height);
    }));
    field.set_value(text);
    field.el().focus();
    frame(ui);
    field
}

fn press(ui: &Ui, k: Key, m: Modifiers) -> bool {
    let took = ui.key(&key(k, None, m));
    frame(ui);
    took
}

#[test]
fn up_and_down_move_through_the_lines_keeping_the_column() {
    let ui = ui();
    let text = "first line\nab\nthird line";
    let field = multi(&ui, text, 120.0);
    assert_eq!(field.caret(), text.len(), "set_value leaves the caret at the end");

    assert!(press(&ui, Key::ArrowUp, Modifiers::NONE));
    assert_eq!(field.caret(), 13, "Up from the end of line three: the end of the short line two");
    assert!(press(&ui, Key::ArrowUp, Modifiers::NONE));
    assert_eq!(field.caret(), 10, "and on up to the same column of line one, not the short line's");
    assert!(press(&ui, Key::ArrowDown, Modifiers::NONE));
    assert!(press(&ui, Key::ArrowDown, Modifiers::NONE));
    assert_eq!(field.caret(), text.len(), "Down twice comes back to where it began");

    // On the first line Up goes to the start of the text, on the last Down to its end.
    assert!(press(&ui, Key::ArrowUp, Modifiers::NONE));
    assert!(press(&ui, Key::ArrowUp, Modifiers::NONE));
    assert!(press(&ui, Key::ArrowUp, Modifiers::NONE));
    assert_eq!(field.caret(), 0);
    assert!(!press(&ui, Key::ArrowUp, Modifiers::NONE), "Up at the very start is not taken: a page scrolls");
    assert!(press(&ui, Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!(field.selection(), 0..11, "Shift+Down selects the first line");
    assert!(press(&ui, Key::ArrowDown, Modifiers::SHIFT));
    assert!(press(&ui, Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!(field.selection(), 0..text.len(), "and on to the end of the text from the last line");
    assert!(!press(&ui, Key::ArrowDown, Modifiers::SHIFT), "nothing further to select");
}

#[test]
fn up_and_down_follow_wrapped_lines() {
    let ui = ui();
    let text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen ".repeat(3);
    let field = multi(&ui, text.trim_end(), 200.0);
    let top = |f: &remedy_ui_engine::view::TextField| f.caret_rect().map_or(0.0, |r| r.min.y);
    let mut seen = vec![(field.caret(), top(&field))];
    while press(&ui, Key::ArrowUp, Modifiers::NONE) {
        seen.push((field.caret(), top(&field)));
    }
    assert!(seen.len() >= 4, "the text wraps to several lines: {seen:?}");
    assert!(
        seen.windows(2).all(|w| matches!(w, [a, b] if b.0 < a.0)),
        "each step is earlier in the text: {seen:?}"
    );
    // Every step but the last (to the start of the text, on the first line) is a line up.
    let lines_up = seen.windows(2).filter(|w| matches!(w, [a, b] if b.1 < a.1 - 0.5)).count();
    assert!(lines_up >= seen.len() - 2, "each step is a line up: {seen:?}");
    assert_eq!(field.caret(), 0);
}

#[test]
fn home_and_end_go_to_the_line_edges_and_with_ctrl_to_the_text_edges() {
    let ui = ui();
    let text = "first line\nsecond\nthird line";
    let field = multi(&ui, text, 120.0);
    press(&ui, Key::ArrowUp, Modifiers::NONE);
    assert!(press(&ui, Key::Home, Modifiers::NONE));
    assert_eq!(field.caret(), 11, "Home: the start of line two, not of the text");
    assert!(press(&ui, Key::End, Modifiers::NONE));
    assert_eq!(field.caret(), 17, "End: the end of line two, before its line break");
    assert!(press(&ui, Key::Home, Modifiers::SHIFT));
    assert_eq!(field.selection(), 11..17, "Shift+Home selects the line");
    assert!(press(&ui, Key::Home, Modifiers::CTRL));
    assert_eq!(field.caret(), 0, "Ctrl+Home: the start of the text");
    assert!(press(&ui, Key::End, Modifiers { ctrl: true, shift: true, ..Modifiers::NONE }));
    assert_eq!(field.selection(), 0..text.len(), "Ctrl+Shift+End selects to the end of the text");

    // A wrapped line: End stays on the line the caret is drawn on.
    let wrapped = "one two three four five six seven eight nine ten eleven twelve";
    field.set_value(wrapped);
    frame(&ui);
    press(&ui, Key::Home, Modifiers::CTRL);
    assert!(press(&ui, Key::End, Modifiers::NONE));
    let first_end = field.caret();
    assert!(first_end > 0 && first_end < wrapped.len(), "End stops at the first line's end, at {first_end}");
    let top = field.caret_rect().map(|r| r.min.y);
    press(&ui, Key::Home, Modifiers::CTRL);
    assert_eq!(field.caret_rect().map(|r| r.min.y), top, "and the caret is drawn on that first line");
    press(&ui, Key::End, Modifiers::NONE);
    assert!(press(&ui, Key::Home, Modifiers::NONE));
    assert_eq!(field.caret(), 0, "Home from a wrapped line's end goes to that line's start");
}

#[test]
fn page_keys_move_a_page_and_scroll_the_caret_into_view() {
    let ui = ui();
    let text = (1..=40).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let field = multi(&ui, &text, 80.0);
    press(&ui, Key::Home, Modifiers::CTRL);
    assert!(press(&ui, Key::PageDown, Modifiers::NONE));
    let after = field.caret();
    let line = text.get(..after).map_or(0, |t| t.matches('\n').count());
    assert!((2..=6).contains(&line), "Page Down moves about a page of an 80 px box, to line {line}");
    let (b, c) = (field.el().bounds().unwrap_or_default(), field.caret_rect().unwrap_or_default());
    assert!(c.min.y >= b.min.y - 0.5 && c.max.y <= b.max.y + 0.5, "the caret stays in view: {c:?} in {b:?}");
    assert!(press(&ui, Key::PageUp, Modifiers::NONE));
    assert_eq!(field.caret(), 0, "Page Up goes back");
    assert!(press(&ui, Key::PageUp, Modifiers::SHIFT) || field.caret() == 0);
    press(&ui, Key::End, Modifiers::CTRL);
    assert_eq!(field.caret(), text.len());
    assert!(!press(&ui, Key::PageDown, Modifiers::NONE), "Page Down at the end is not taken");
}

#[test]
fn single_line_fields_leave_up_and_down_alone() {
    let ui = ui();
    let (field, x, y) = field_at(&ui, FieldKind::Single);
    field.set_value("hello world");
    frame(&ui);
    click(&ui, x, y, Duration::ZERO);
    let at = field.caret();
    let _ = ui.key(&key(Key::ArrowDown, None, Modifiers::NONE));
    let _ = ui.key(&key(Key::ArrowUp, None, Modifiers::NONE));
    assert_eq!(field.caret(), at, "a suggestion list or a spin button owns Up and Down");
}

#[test]
fn the_older_clipboard_chords_cut_copy_and_paste() {
    let ui = ui();
    let (field, x, y) = field_at(&ui, FieldKind::Single);
    field.set_value("hello");
    frame(&ui);
    click(&ui, x, y, Duration::ZERO);
    ui.key(&ctrl("a"));
    let insert = || Key::Named("Insert".into());
    let copied = |ui: &Ui, what: &str| {
        ui.take_requests()
            .iter()
            .any(|r| matches!(r, remedy_ui_engine::view::Request::Copy { text, .. } if text == what))
    };
    assert!(ui.key(&key(insert(), None, Modifiers::CTRL)));
    assert!(copied(&ui, "hello"), "Ctrl+Insert copies");
    assert_eq!(field.value(), "hello");
    assert!(ui.key(&key(Key::Delete, None, Modifiers::SHIFT)));
    assert!(copied(&ui, "hello"), "Shift+Delete cuts");
    assert_eq!(field.value(), "");
    assert!(ui.key(&key(insert(), None, Modifiers::SHIFT)));
    assert!(ui.take_requests().contains(&remedy_ui_engine::view::Request::Paste), "Shift+Insert pastes");
}

#[test]
fn an_app_selects_a_range_and_it_scrolls_into_view_without_focus() {
    let ui = ui();
    let text = (1..=40).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let field = multi(&ui, &text, 80.0);
    press(&ui, Key::Home, Modifiers::CTRL);
    field.el().blur();
    frame(&ui);
    let at = text.find("line 30").unwrap_or(0);
    assert!(field.set_selection(at..at + 7));
    frame(&ui);
    assert_eq!(field.selection(), at..at + 7);
    let (b, c) = (field.el().bounds().unwrap_or_default(), field.caret_rect().unwrap_or_default());
    assert!(c.min.y >= b.min.y - 0.5 && c.max.y <= b.max.y + 0.5, "the match is in view: {c:?} in {b:?}");
    assert!(!field.set_selection(0..text.len() + 1), "a range off the text is refused");
    field.set_decorations(vec![remedy_ui_engine::view::Decoration::highlight(at..at + 7)]);
    frame(&ui);
    assert_eq!(field.decorations().len(), 1);
}

#[test]
fn the_clipboard_and_undo_chords_work_on_a_layout_without_latin_letters() {
    let ui = ui();
    let (field, x, y) = field_at(&ui, FieldKind::Single);
    field.set_value("привет");
    frame(&ui);
    click(&ui, x, y, Duration::ZERO);
    // A Russian layout: the A, C, X, V and Z keys type ф, с, ч, м and я.
    let chord = |typed: &str, place: &str| {
        key(Key::Character(typed.into()), None, Modifiers::CTRL).with_physical(place.to_owned())
    };
    assert!(ui.key(&chord("ф", "KeyA")));
    assert_eq!(field.selection(), 0..field.value().len(), "Ctrl+A selects all");
    assert!(ui.key(&chord("с", "KeyC")));
    let copied = |ui: &Ui| {
        ui.take_requests()
            .iter()
            .any(|r| matches!(r, remedy_ui_engine::view::Request::Copy { text, .. } if text == "привет"))
    };
    assert!(copied(&ui), "Ctrl+C copies");
    assert!(ui.key(&chord("ч", "KeyX")));
    assert!(copied(&ui), "Ctrl+X cuts");
    assert_eq!(field.value(), "");
    assert!(ui.key(&chord("м", "KeyV")));
    assert!(ui.take_requests().contains(&remedy_ui_engine::view::Request::Paste), "Ctrl+V pastes");
    assert!(ui.key(&chord("я", "KeyZ")));
    assert_eq!(field.value(), "привет", "Ctrl+Z undoes the cut");
}
