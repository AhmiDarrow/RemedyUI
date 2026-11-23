//! Text field editing: word and all selection, drag, focus-selects-all, and the
//! usual keyboard chords.

use std::time::Duration;

use remedy_ui_core::{Point, Size};
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_layout::Length;
use remedy_ui_style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{FieldKind, InputPurpose, Ui};

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

fn field_at(ui: &Ui, kind: FieldKind) -> (remedy_ui_view::TextField, f32, f32) {
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
            .any(|r| matches!(r, remedy_ui_view::Request::Copy { text, .. } if text == "world"))
    );
    ui.key(&ctrl("v"));
    assert!(ui.take_requests().contains(&remedy_ui_view::Request::Paste));
    ui.paste("world");
    assert_eq!(field.value(), "world");
    ui.key(&ctrl("z"));
    assert_eq!(field.value(), "", "Ctrl+Z undoes the paste");
    ui.key(&ctrl("y"));
    assert_eq!(field.value(), "world", "Ctrl+Y redoes it");
}
