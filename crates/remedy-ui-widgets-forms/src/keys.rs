//! Recording a keyboard shortcut.

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::input::{Chord, Key, KeyEvent, Modifiers};
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::style::States;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

fn is_modifier(k: &Key) -> bool {
    matches!(k, Key::Named(n) if matches!(n.as_str(), "Shift" | "Control" | "Alt" | "AltGraph" | "Meta" | "Super" | "Hyper" | "Fn" | "CapsLock" | "NumLock"))
}

/// A field that records a shortcut. Focus it and press the chord: it takes the next key that
/// is not a modifier, with the modifiers held. Escape leaves it unchanged, Backspace or
/// Delete with no modifiers clears it. Shortcuts show the platform's way (`Ctrl+Shift+K`,
/// `⇧⌘K`).
#[track_caller]
pub fn key_capture(ui: &Ui, label: impl Into<String>, value: Signal<Option<Chord>>) -> El {
    let label = label.into();
    let recording = Signal::new(false);
    let el = ui.el("text_input").focusable(true).fill_width();
    // Key names are the platform's, not the app's strings.
    let text = w::text(ui, "").data_text();
    let el = el.child(text.clone());
    let (u2, e2, e3, e4, l2) = (ui.clone(), el.clone(), el.clone(), el.clone(), label.clone());
    // Activating it (a press, or a screen reader's activate) starts recording.
    el.on_focus_change(move |f| recording.set(f))
        .on_click(move || e4.focus())
        .on_key(move |e: &KeyEvent| {
            if !e.pressed || e.repeat {
                return e.pressed;
            }
            if is_modifier(&e.key) {
                return true;
            }
            match e.key {
                Key::Escape => {
                    e2.blur();
                    return true;
                }
                Key::Tab if !e.modifiers.ctrl && !e.modifiers.alt && !e.modifiers.meta => return false,
                Key::Backspace | Key::Delete if e.modifiers == Modifiers::NONE => {
                    value.set(None);
                    return true;
                }
                _ => {}
            }
            value.set(Some(Chord::new(e.modifiers, e.key.clone())));
            true
        })
        .bind(move || {
            let chord = value.get();
            let rec = recording.get();
            let shown = match (&chord, rec) {
                (Some(c), _) => c.label(u2.platform()),
                (None, true) => crate::tr(&u2, "keys-prompt"),
                (None, false) => crate::tr(&u2, "keys-none"),
            };
            let _ = text.clone().text(remedy_ui_view::RichText::plain(shown.clone()));
            e3.state(States::SELECTED, rec);
            let _ = e3.clone().semantics(
                Semantics::new(Role::TextInput)
                    .label(l2.clone())
                    .value(shown)
                    .description(crate::tr(&u2, if rec { "keys-recording" } else { "keys-idle" }))
                    .action(Action::Click)
                    .action(Action::Focus),
            );
        })
}
