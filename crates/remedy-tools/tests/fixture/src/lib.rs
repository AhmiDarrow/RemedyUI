//! A small app the tools' tests preview, audit and test (remedy-mcp's and cargo remedy's).
//! The tests copy it into the target folder with a Cargo.toml naming this checkout's facade.

use remedy_ui::core::Insets;
use remedy_ui::widgets as w;
use remedy_ui::{El, Ui};

/// The screen previewed.
pub fn home(ui: &Ui) -> El {
    w::column(ui)
        .gap(8.0)
        .padding(Insets::all(16.0))
        .child(w::text(ui, "Settings").variant("strong"))
        .child(w::button(ui, "Save", || {}).variant("primary"))
        .child(ui.el("swatch").width(40.0).height(40.0).on_click(|| {}))
}

pub mod screens {
    use remedy_ui::{El, Ui};

    /// A second screen, by path.
    pub fn about(ui: &Ui) -> El {
        remedy_ui::widgets::text(ui, "About").variant("muted")
    }
}
