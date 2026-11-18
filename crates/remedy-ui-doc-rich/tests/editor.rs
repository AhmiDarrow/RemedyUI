//! The rich editor on a real view: the toolbar, the link box, mentions, Enter to send, and
//! what it saves.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_a11y::Role;
use remedy_ui_doc_rich::*;
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_text::{Block, Mark, RichDoc};
use remedy_ui_view::{El, TextField, Ui};

mod common;
use common::*;

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

fn editor(options: RichEditorOptions) -> (Ui, El, TextField) {
    let ui = ui();
    let (el, field) = rich_editor(&ui, options);
    ui.root().child(ui.el("page").column().fill_width().child(el.clone()));
    frame(&ui);
    field.el().focus();
    (ui, el, field)
}

/// Elements with `role` in any of the window's layers (popups sit above the content).
fn anywhere(ui: &Ui, role: Role) -> Vec<El> {
    ui.layers().iter().flat_map(|l| with_role(l, role)).collect()
}

fn button(el: &El, name: &str) -> El {
    with_role(el, Role::Button)
        .into_iter()
        .find(|b| common::name(b) == name)
        .unwrap_or_else(|| unreachable!("{name}"))
}

fn checked(b: &El) -> bool {
    b.info().and_then(|i| i.semantics).and_then(|s| s.checked_state()).unwrap_or(false)
}

#[test]
fn the_toolbar_formats_and_shows_what_is_on() {
    let (ui, el, field) = editor(RichEditorOptions::default());
    typed(&ui, "make this bold");
    field.edit_rich(|e| e.set_selection(10, 14));
    click(&ui, centre(&button(&el, "Bold")), Duration::from_secs(1));
    assert_eq!(field.doc().map(|d| d.marks), Some(vec![(10..14, Mark::Bold)]));
    assert!(checked(&button(&el, "Bold")), "the button shows bold is on");
    assert!(!checked(&button(&el, "Italic")));
    // Focus went back to the text.
    assert!(field.el().info().is_some_and(|i| i.states.contains(remedy_ui_style::States::FOCUS)));
    // The caret in plain text: bold shows off.
    field.edit_rich(|e| e.set_selection(2, 2));
    frame(&ui);
    assert!(!checked(&button(&el, "Bold")));
    // A list from the toolbar.
    click(&ui, centre(&button(&el, "Bulleted list")), Duration::from_secs(3));
    assert_eq!(field.doc().map(|d| d.blocks), Some(vec![Block::Bullet(0)]));
    assert!(checked(&button(&el, "Bulleted list")));
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn ctrl_k_links_the_selection() {
    let (ui, _el, field) = editor(RichEditorOptions::default());
    typed(&ui, "read the docs");
    field.edit_rich(|e| e.set_selection(9, 13));
    key(&ui, Key::Character("k".into()), CTRL);
    typed(&ui, "https://example.org");
    key(&ui, Key::Enter, NONE);
    assert_eq!(field.doc().map(|d| d.marks), Some(vec![(9..13, Mark::Link("https://example.org".into()))]));
    // Opening it again on the link shows its address; Remove link takes it off.
    field.edit_rich(|e| e.set_selection(10, 10));
    key(&ui, Key::Character("k".into()), CTRL);
    let dialog = anywhere(&ui, Role::Dialog);
    let [dialog] = dialog.as_slice() else { unreachable!("one link box: {}", dialog.len()) };
    let remove = with_role(dialog, Role::Button).into_iter().find(|b| name(b) == "Remove link");
    click(&ui, centre(&remove.unwrap_or_else(|| unreachable!("remove"))), Duration::from_secs(5));
    assert_eq!(field.doc().map(|d| d.marks), Some(Vec::new()));
}

#[test]
fn typing_at_suggests_people_to_mention() {
    let people = [
        MentionCandidate { id: "u1".into(), name: "Alex".into() },
        MentionCandidate { id: "u2".into(), name: "Alba".into() },
        MentionCandidate { id: "u3".into(), name: "Sam".into() },
    ];
    let asked = Rc::new(Cell::new(0));
    let a2 = asked.clone();
    let source: MentionSource = Rc::new(move |q: &str| {
        a2.set(a2.get() + 1);
        people.iter().filter(|p| p.name.to_lowercase().starts_with(&q.to_lowercase())).cloned().collect()
    });
    let (ui, _el, field) = editor({
        let mut v_ = RichEditorOptions::default();
        v_.mentions = Some(source);
        v_
    });
    typed(&ui, "hi @al");
    let list = anywhere(&ui, Role::List);
    let names: Vec<String> =
        list.iter().flat_map(|l| with_role(l, Role::ListItem)).map(|i| name(&i)).collect();
    assert_eq!(names, ["Alex", "Alba"]);
    key(&ui, Key::ArrowDown, NONE);
    key(&ui, Key::Enter, NONE);
    let doc = field.doc().unwrap_or_default();
    assert_eq!(doc.text, "hi @Alba ");
    assert_eq!(doc.marks, [(3..8, Mark::Mention("u2".into()))]);
    assert!(anywhere(&ui, Role::List).is_empty(), "the list closed");
    // Escape closes the list and it stays closed for that "@".
    typed(&ui, "@s");
    assert_eq!(anywhere(&ui, Role::List).len(), 1);
    key(&ui, Key::Escape, NONE);
    typed(&ui, "a");
    assert!(anywhere(&ui, Role::List).is_empty());
    // An "@" inside a word is not a mention.
    typed(&ui, " me@al");
    assert!(anywhere(&ui, Role::List).is_empty());
    assert!(asked.get() > 0);
}

#[test]
fn a_composer_sends_on_enter_and_saves_markdown() {
    let (ui, _el, field) = editor({
        let mut v_ = RichEditorOptions::default();
        v_.submit_on_enter = true;
        v_
    });
    let sent = Rc::new(Cell::new(0));
    let s2 = sent.clone();
    let field = field.on_submit(move || s2.set(s2.get() + 1));
    typed(&ui, "one");
    key(&ui, Key::Enter, SHIFT);
    typed(&ui, "two");
    assert_eq!(sent.get(), 0);
    assert_eq!(field.value(), "one\ntwo");
    key(&ui, Key::Enter, NONE);
    assert_eq!(sent.get(), 1);
    field.edit_rich(|e| {
        e.select_all();
        e.toggle(Mark::Italic);
    });
    let md = MarkdownFormat.write(&field.doc().unwrap_or_default());
    assert_eq!(md, "*one*\n\n*two*");
    field.set_doc(MarkdownFormat.read("**hello** there"));
    assert_eq!(
        field.doc(),
        Some(RichDoc {
            text: "hello there".into(),
            marks: vec![(0..5, Mark::Bold)],
            blocks: vec![Block::Paragraph]
        })
    );
}
