//! Text.

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_engine::view::{El, IntoProp, RichText, Ui};

/// Body text (wraps to its width): a string, or a signal or closure giving one. A variant of the style changes its look: `muted`, `small`,
/// `error`, `strong` or `code` (monospace, in a box).
///
/// ```
/// # fn build(ui: &remedy_ui_engine::view::Ui) {
/// use remedy_ui_widgets::base::text;
///
/// ui.root().child(text(ui, "Saved")).child(text(ui, "2 minutes ago").variant("muted"));
/// # }
/// ```
#[track_caller]
pub fn text(ui: &Ui, s: impl IntoProp<String>) -> El {
    ui.el("label").text(s.into_prop().map(RichText::plain))
}

/// Text in a variant of the label style.
#[deprecated(since = "0.1.0", note = "use `text(ui, s).variant(variant)`; the variant is optional")]
#[track_caller]
pub fn label(ui: &Ui, s: impl IntoProp<String>, variant: impl IntoProp<&'static str>) -> El {
    text(ui, s).variant(variant)
}

/// A heading, level 1 to 3.
#[track_caller]
pub fn heading(ui: &Ui, level: u8, s: impl IntoProp<String>) -> El {
    let s = s.into_prop();
    let variant = match level {
        0 | 1 => "1",
        2 => "2",
        _ => "3",
    };
    let role = Role::Heading(level.clamp(1, 6));
    // A real heading, so screen readers can move from one to the next.
    ui.el("heading")
        .variant(variant)
        .text(s.clone().map(RichText::plain))
        .semantics(s.map(move |s| Semantics::new(role).label(s)))
}

/// Text that does something when activated (opens a page, shows details).
#[track_caller]
pub fn link(ui: &Ui, s: impl IntoProp<String>, on_activate: impl Fn() + 'static) -> El {
    let s = s.into_prop();
    let accent = ui.theme().colors.accent;
    let color = remedy_ui_core::style::ensure_contrast(accent, ui.theme().colors.bg, 4.5);
    crate::base::min_target(ui.el("link"))
        .cursor(remedy_ui_engine::view::Cursor::Pointer)
        .text(s.clone().map(move |s| {
            RichText::new().styled(&s, {
                let mut v_ = remedy_ui_engine::view::TextStylePatch::default();
                v_.color = Some(color);
                v_.underline = Some(true);
                v_
            })
        }))
        .on_click(on_activate)
        .semantics(s.map(|s| Semantics::new(Role::Link).label(s).action(Action::Click)))
}
