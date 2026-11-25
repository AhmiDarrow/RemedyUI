//! Editing right-to-left and mixed-direction text: the arrow keys move the caret the way
//! the text is drawn, selections grow and collapse visually, and word steps cross runs.

use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Size};
use remedy_ui_core::input::{Key, KeyEvent, Modifiers, ScrollPhysics};
use remedy_ui_core::layout::Length;
use remedy_ui_render_cpu::CpuCompositor;
use remedy_ui_core::style::{StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{FieldKind, TextField, Ui};

fn ui(locale: &str) -> Ui {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    for fb in remedy_ui_fonts::script_fallbacks() {
        let _ = f.add_script_fallback(fb.script, fb.locale, fb.family);
    }
    let ui = Ui::new(Theme::dark_forest(), StyleSheet::new(), f);
    ui.set_locale(locale);
    ui
}

fn frame(ui: &Ui) {
    let f = ui.frame(Size::new(400.0, 100.0), 1.0);
    let mut comp = Compositor::new(400, 100, 1.0, ScrollPhysics::CLAMPED);
    if let Some(l) = f.layers {
        comp.commit(l);
    }
    let plan = comp.frame(Duration::ZERO);
    let _ = CpuCompositor::new().present(&plan, 400, 100, Color::BLACK);
}

fn press(ui: &Ui, k: Key, shift: bool, ctrl: bool) {
    let m = Modifiers { shift, ctrl, ..Modifiers::NONE };
    ui.key(&KeyEvent::press(k).with_modifiers(m).with_text(None));
    frame(ui);
}

fn field(ui: &Ui, kind: FieldKind, text: &str) -> TextField {
    let f = ui.text_field(if kind == FieldKind::Rich { "rich_input" } else { "text_input" }, kind);
    let _ = ui.root().child(f.el().layout(|l| l.width = Length::Px(380.0)));
    f.set_value(text);
    frame(ui);
    f.el().focus();
    frame(ui);
    f
}

/// The caret's x on screen.
fn caret_x(f: &TextField) -> f32 {
    f.caret_rect().map_or(f32::NAN, |r| r.min.x)
}

/// Presses `k` until the caret stops, recording each caret index and x.
fn walk(ui: &Ui, f: &TextField, k: Key) -> Vec<(usize, f32)> {
    let mut seen = vec![(f.caret(), caret_x(f))];
    for _ in 0..200 {
        press(ui, k.clone(), false, false);
        let now = (f.caret(), caret_x(f));
        if seen.last().is_some_and(|l| l.0 == now.0) {
            break;
        }
        seen.push(now);
    }
    seen
}

fn home(ui: &Ui, f: &TextField) {
    press(ui, Key::Home, false, false);
    assert_eq!(f.caret(), 0);
}

#[test]
fn right_moves_toward_the_start_of_arabic_text() {
    let ui = ui("ar");
    let text = "مرحبا بكم";
    let f = field(&ui, FieldKind::Single, text);
    home(&ui, &f);
    // The start of Arabic text is its right edge: Right cannot go further, Left goes on.
    let x0 = caret_x(&f);
    press(&ui, Key::ArrowRight, false, false);
    assert_eq!(f.caret(), 0, "already at the right edge");
    let steps = walk(&ui, &f, Key::ArrowLeft);
    assert_eq!(steps.last().map(|s| s.0), Some(text.len()), "Left reaches the end, at the left edge");
    assert_eq!(steps.len(), text.chars().count() + 1, "one step per character: {steps:?}");
    for w in steps.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.last()) else { continue };
        assert!(b.1 < a.1 + 0.01, "each Left press moves the caret left: {steps:?}");
    }
    assert!(steps.last().is_some_and(|s| s.1 < x0 - 40.0), "the end is drawn left of the start");
    // And Right walks back to the start.
    let back = walk(&ui, &f, Key::ArrowRight);
    assert_eq!(back.last().map(|s| s.0), Some(0));
}

#[test]
fn arrows_cross_mixed_runs_the_way_they_are_drawn() {
    // An English sentence with an Arabic name in it (left-to-right paragraph), and an Arabic
    // sentence with an English word in it (right-to-left paragraph).
    for (locale, text) in [("en", "Send مرحبا now"), ("ar", "أرسل report الآن")] {
        let ui = ui(locale);
        let f = field(&ui, FieldKind::Single, text);
        home(&ui, &f);
        // Go to the left edge first, then walk right across the whole line.
        let _ = walk(&ui, &f, Key::ArrowLeft);
        let steps = walk(&ui, &f, Key::ArrowRight);
        let xs: Vec<f32> = steps.iter().map(|s| s.1).collect();
        for w in xs.windows(2) {
            let (Some(a), Some(b)) = (w.first(), w.last()) else { continue };
            assert!(*b > a + 0.5, "{locale}: each Right press moves the caret right: {steps:?}");
        }
        // One stop per place a caret can be drawn: as many as there are character
        // boundaries (where runs meet, one place serves two indices, and another index
        // stands for it), and every boundary inside each run.
        let boundaries: Vec<usize> = (0..=text.len()).filter(|i| text.is_char_boundary(*i)).collect();
        assert_eq!(steps.len(), boundaries.len(), "{locale}: {steps:?}");
        let visited: Vec<usize> = steps.iter().map(|s| s.0).collect();
        let edges = |c: char| c == ' ';
        for i in boundaries.iter().filter(|i| {
            let before = text.get(..**i).and_then(|t| t.chars().last());
            let after = text.get(**i..).and_then(|t| t.chars().next());
            // Inside a word: not at a run's edge.
            before.is_some_and(|c| !edges(c)) && after.is_some_and(|c| !edges(c))
        }) {
            assert!(visited.contains(i), "{locale}: {i} not reached: {steps:?}");
        }
    }
}

#[test]
fn selections_grow_and_collapse_visually() {
    let ui = ui("ar");
    let text = "مرحبا بكم";
    let f = field(&ui, FieldKind::Single, text);
    home(&ui, &f);
    // Shift+Left from the start selects the first letters (they are to its left).
    press(&ui, Key::ArrowLeft, true, false);
    press(&ui, Key::ArrowLeft, true, false);
    let first_two: usize = text.chars().take(2).map(char::len_utf8).sum();
    assert_eq!(f.selection(), 0..first_two);
    // Left without Shift collapses to the selection's left edge: its logical end.
    press(&ui, Key::ArrowLeft, false, false);
    assert_eq!(f.selection(), first_two..first_two);
    // Select again, then Right collapses to the right edge: the logical start.
    press(&ui, Key::ArrowLeft, true, false);
    press(&ui, Key::ArrowRight, false, false);
    assert_eq!(f.caret(), first_two);
}

#[test]
fn word_steps_follow_the_drawn_order() {
    let ui = ui("ar");
    let text = "مرحبا بكم جميعا";
    let f = field(&ui, FieldKind::Single, text);
    home(&ui, &f);
    // Ctrl+Left from the start of Arabic text moves forward by a word.
    press(&ui, Key::ArrowLeft, false, true);
    let after = f.caret();
    assert!(after > 0 && after < text.len(), "{after}");
    assert!(text.get(..after).is_some_and(|s| s.trim_end() == "مرحبا"), "one word: {:?}", text.get(..after));
    press(&ui, Key::ArrowRight, false, true);
    assert_eq!(f.caret(), 0, "and back");
}

#[test]
fn rich_fields_move_visually_too() {
    // A chat composer is a rich field: Arabic messages edit the same way.
    let ui = ui("ar");
    let text = "مرحبا بكم";
    let f = field(&ui, FieldKind::Rich, text);
    home(&ui, &f);
    press(&ui, Key::ArrowLeft, false, false);
    assert_eq!(f.caret(), "م".len(), "Left moves forward through Arabic");
    press(&ui, Key::ArrowRight, false, false);
    assert_eq!(f.caret(), 0);
    let steps = walk(&ui, &f, Key::ArrowLeft);
    assert_eq!(steps.last().map(|s| s.0), Some(text.len()));
}

#[test]
fn english_fields_still_move_left_to_right() {
    let ui = ui("en");
    let f = field(&ui, FieldKind::Single, "hello");
    home(&ui, &f);
    press(&ui, Key::ArrowRight, false, false);
    assert_eq!(f.caret(), 1);
    press(&ui, Key::ArrowLeft, false, false);
    assert_eq!(f.caret(), 0);
}
