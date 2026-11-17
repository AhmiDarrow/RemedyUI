//! Text.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_view::{El, Ui};

/// Body text (wraps to its width).
#[track_caller]
pub fn text(ui: &Ui, s: impl Into<String>) -> El {
    ui.el("label").text(remedy_ui_view::RichText::plain(s.into()))
}

/// Text in a variant of the label style: `muted`, `small`, `error`, `strong` or `code`
/// (monospace, in a box).
#[track_caller]
pub fn label(ui: &Ui, s: impl Into<String>, variant: &'static str) -> El {
    text(ui, s).variant(variant)
}

/// A heading, level 1 to 3.
#[track_caller]
pub fn heading(ui: &Ui, level: u8, s: impl Into<String>) -> El {
    let s = s.into();
    let variant = match level {
        0 | 1 => "1",
        2 => "2",
        _ => "3",
    };
    ui.el("heading").variant(variant).text(remedy_ui_view::RichText::plain(s.clone())).semantics(
        Semantics::new(Role::Label).label(s).description(format!("heading level {}", level.clamp(1, 3))),
    )
}

/// Text that does something when activated (opens a page, shows details).
#[track_caller]
pub fn link(ui: &Ui, s: impl Into<String>, on_activate: impl Fn() + 'static) -> El {
    let s = s.into();
    let accent = ui.theme().colors.accent;
    let color = remedy_ui_style::ensure_contrast(accent, ui.theme().colors.bg, 4.5);
    ui.el("link")
        .text(remedy_ui_view::RichText::new().styled(
            &s,
            remedy_ui_view::TextPatch { color: Some(color), underline: Some(true), ..Default::default() },
        ))
        .on_click(on_activate)
        .semantics(Semantics::new(Role::Link).label(s).action(Action::Click))
}
