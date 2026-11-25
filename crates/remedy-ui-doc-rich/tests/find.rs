//! The find bar on a real view: matches marked and counted, stepping round them, the current
//! one selected and scrolled to, case folded in any script, and Escape ending it all.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_doc_rich::*;
use remedy_ui_core::input::{Key, KeyEvent};
use remedy_ui_engine::view::{El, Ui};

mod common;
use common::*;

fn page(ui: &Ui) -> El {
    let doc = markdown(
        ui,
        "# The book\n\nThe cat sat on the mat.\n\nÉcole and école.\n\nNothing here.\n\nThe end.",
        |_| {},
    );
    ui.el("page").column().fill_width().child(doc)
}

fn typed(ui: &Ui, s: &str) {
    for c in s.chars() {
        ui.key(
            &KeyEvent::press(Key::Character(c.to_string()))
                .with_modifiers(NONE)
                .with_text(Some(c.to_string())),
        );
    }
    frame(ui);
}

struct Bar {
    ui: Ui,
    state: FindState,
    bar: El,
    closed: Rc<Cell<u32>>,
}

fn bar() -> Bar {
    let ui = ui();
    let content = page(&ui);
    let state = FindState::new(&content);
    let closed = Rc::new(Cell::new(0));
    let c = closed.clone();
    let bar = find_bar(&ui, &state, move || c.set(c.get() + 1));
    ui.root().child(ui.el("col").column().fill_width().child(bar.clone()).child(content));
    frame(&ui);
    Bar { ui, state, bar, closed }
}

fn status(b: &Bar) -> String {
    with_role(&b.bar, remedy_ui_core::a11y::Role::Status).first().map(name).unwrap_or_default()
}

#[test]
fn typing_marks_counts_and_selects_the_first_match() {
    let b = bar();
    first(&b.bar, "text_input").focus();
    typed(&b.ui, "the");
    assert_eq!(b.state.count.get(), 4, "The book, The cat, the mat, The end");
    assert_eq!(status(&b), "1 of 4");
    assert_eq!(b.ui.selected_text().as_deref(), Some("The"));
    // Enter steps on, Shift+Enter back, both round the ends.
    key(&b.ui, Key::Enter, NONE);
    assert_eq!(status(&b), "2 of 4");
    key(&b.ui, Key::Enter, SHIFT);
    key(&b.ui, Key::Enter, SHIFT);
    assert_eq!(status(&b), "4 of 4");
    assert_eq!(b.ui.selected_text().as_deref(), Some("The"));
    key(&b.ui, Key::F(3), NONE);
    assert_eq!(status(&b), "1 of 4");
}

#[test]
fn case_folds_in_every_script_and_says_when_nothing_matches() {
    let b = bar();
    b.state.search("ÉCOLE");
    frame(&b.ui);
    assert_eq!(b.state.count.get(), 2);
    b.state.search("zebra");
    frame(&b.ui);
    assert_eq!(status(&b), "No matches");
    assert_eq!(b.ui.selected_text(), None);
}

#[test]
fn escape_ends_the_search_and_closes() {
    let b = bar();
    first(&b.bar, "text_input").focus();
    typed(&b.ui, "cat");
    assert_eq!(b.state.count.get(), 1);
    key(&b.ui, Key::Escape, NONE);
    assert_eq!(b.closed.get(), 1);
    assert_eq!(b.state.count.get(), 0);
    assert_eq!(b.ui.selected_text(), None);
    assert_eq!(status(&b), "");
}

#[test]
fn the_bar_passes_the_audit() {
    let b = bar();
    b.state.search("the");
    let found = audit(&b.ui);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_doc_rich::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn the_bar_speaks_the_windows_language() {
    let b = bar();
    b.state.search("the");
    frame(&b.ui);
    assert_eq!(status(&b), "1 of 4");
    b.ui.set_locale("de");
    frame(&b.ui);
    assert_eq!(status(&b), "1 von 4");
    let names: Vec<String> = with_role(&b.bar, remedy_ui_core::a11y::Role::Button).iter().map(name).collect();
    assert!(names.iter().any(|n| n == "Nächster Treffer"), "{names:?}");
    b.state.search("zebra");
    frame(&b.ui);
    assert_eq!(status(&b), "Keine Treffer");
}
