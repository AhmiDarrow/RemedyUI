//! Text inputs.

use remedy_ui_layout::Align;
use remedy_ui_view::{El, FieldKind, TextField, Ui};

/// A one-line text input. Its value is read with [`TextField::value`]; Enter calls
/// `on_submit`.
#[track_caller]
pub fn text_input(ui: &Ui, label: impl Into<String>) -> TextField {
    ui.text_field("text_input", FieldKind::Single).label(label).fill()
}

/// A multi-line text input; Enter breaks the line.
/// It grows with its text; given a height, longer text scrolls inside it to the caret.
#[track_caller]
pub fn text_area(ui: &Ui, label: impl Into<String>) -> TextField {
    ui.text_field("text_area", FieldKind::Multi).label(label).fill()
}

/// A search box: a one-line input with a search icon, Escape clears it.
#[track_caller]
pub fn search(ui: &Ui, label: impl Into<String>, on_change: impl Fn(String) + 'static) -> El {
    let label = label.into();
    let field = ui.text_field("text_input", FieldKind::Single).label(label.clone()).placeholder(label);
    let _ = field.el().variant("bare");
    let f2 = field.clone();
    let field = field.on_change(move || on_change(f2.value()));
    let f3 = field.clone();
    let gap = ui.theme().spacing.unit * 1.5;
    ui.el("search")
        .row()
        .align(Align::Center)
        .gap(gap)
        .child(crate::icon(ui, crate::Icon::Search, ui.theme().type_scale.body + 2.0))
        .child(field.el().grow(1.0))
        .on_key(move |e| {
            if e.pressed && e.key == remedy_ui_input::Key::Escape && !f3.is_empty() {
                f3.set_value("");
                return true;
            }
            false
        })
}

/// A secret input (passwords, keys): masked, held in a zeroizing buffer, never copied, no
/// input method, protected from assistive technology. Take its value with
/// [`TextField::take_secret`].
#[track_caller]
pub fn secret_input(ui: &Ui, label: impl Into<String>) -> TextField {
    ui.text_field("secret_input", FieldKind::Secret).label(label).fill()
}

/// An eye button that shows or hides a secret field's text.
#[track_caller]
pub fn reveal_toggle(ui: &Ui, field: &TextField) -> El {
    let f = field.clone();
    let shown = remedy_ui_reactive::Signal::new(false);
    let u = ui.clone();
    let b = crate::icon_button(ui, crate::Icon::Eye, crate::tr(ui, "show-password"), move || {
        shown.update(|v| *v = !*v)
    });
    let b2 = b.clone();
    b.bind(move || {
        let on = shown.get();
        f.set_reveal(on);
        b2.state(remedy_ui_style::States::CHECKED, on);
        let label = crate::tr(&u, if on { "hide-password" } else { "show-password" });
        let _ = b2.clone().semantics(
            remedy_ui_a11y::Semantics::new(remedy_ui_a11y::Role::Button)
                .label(label)
                .checked(on)
                .action(remedy_ui_a11y::Action::Click),
        );
    })
}

/// A labeled form field: the label above `control`, and an optional hint or error below.
#[track_caller]
pub fn field(ui: &Ui, label: impl Into<String>, control: El) -> El {
    let gap = ui.theme().spacing.unit;
    ui.el("form_field")
        .column()
        .gap(gap)
        .fill_width()
        .child(ui.el("field").text(remedy_ui_view::RichText::plain(label.into())))
        .child(control)
}

trait Fill {
    fn fill(self) -> Self;
}

impl Fill for TextField {
    fn fill(self) -> Self {
        let _ = self.el().fill_width();
        self
    }
}
