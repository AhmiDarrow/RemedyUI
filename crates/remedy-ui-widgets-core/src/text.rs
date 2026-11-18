//! Text.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_view::{El, Ui};

/// Body text (wraps to its width). A variant of the style changes its look: `muted`, `small`,
/// `error`, `strong` or `code` (monospace, in a box).
///
/// ```
/// # fn build(ui: &remedy_ui_view::Ui) {
/// use remedy_ui_widgets_core::text;
///
/// ui.root().child(text(ui, "Saved")).child(text(ui, "2 minutes ago").variant("muted"));
/// # }
/// ```
#[track_caller]
pub fn text(ui: &Ui, s: impl Into<String>) -> El {
    ui.el("label").text(remedy_ui_view::RichText::plain(s.into()))
}

/// Text in a variant of the label style.
#[deprecated(since = "0.1.0", note = "use `text(ui, s).variant(variant)`; the variant is optional")]
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
    // A real heading, so screen readers can move from one to the next.
    ui.el("heading")
        .variant(variant)
        .text(remedy_ui_view::RichText::plain(s.clone()))
        .semantics(Semantics::new(Role::Heading(level.clamp(1, 6))).label(s))
}

/// Text that does something when activated (opens a page, shows details).
#[track_caller]
pub fn link(ui: &Ui, s: impl Into<String>, on_activate: impl Fn() + 'static) -> El {
    let s = s.into();
    let accent = ui.theme().colors.accent;
    let color = remedy_ui_style::ensure_contrast(accent, ui.theme().colors.bg, 4.5);
    crate::min_target(ui.el("link"))
        .text(remedy_ui_view::RichText::new().styled(&s, {
            let mut v_ = remedy_ui_view::TextStylePatch::default();
            v_.color = Some(color);
            v_.underline = Some(true);
            v_
        }))
        .on_click(on_activate)
        .semantics(Semantics::new(Role::Link).label(s).action(Action::Click))
}
