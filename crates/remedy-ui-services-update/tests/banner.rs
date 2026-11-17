//! The update banner follows the update's state.

use remedy_ui_core::Size;
use remedy_ui_reactive::Signal;
use remedy_ui_services_update::{Release, UpdateState, update_banner};
use remedy_ui_view::Ui;

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = remedy_ui_text::FontCollection::new();
    for f in remedy_ui_fonts::all() {
        let _ = fonts.add_static(f.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

#[test]
fn the_banner_shows_each_state() {
    let ui = ui();
    ui.connect_accessibility();
    let state = Signal::new(UpdateState::Idle);
    ui.root().child(update_banner(&ui, state, || {}, || {}));
    let tree = |ui: &Ui| {
        let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
        format!("{:?}", ui.take_accessibility_update())
    };
    let idle = tree(&ui);
    assert!(!idle.contains("Install") && !idle.contains("Restart"), "nothing while idle");
    state.set(UpdateState::Available(Release {
        version: "v0.2.0".into(),
        notes: String::new(),
        url: String::new(),
        signature: String::new(),
        sha256: None,
    }));
    let t = tree(&ui);
    assert!(t.contains("Version 0.2.0 is available.") && t.contains("Install"), "{t}");
    state.set(UpdateState::Downloading(Some(0.5)));
    let t = tree(&ui);
    assert!(t.contains("Downloading") && !t.contains("\"Install\""), "{t}");
    state.set(UpdateState::Ready);
    assert!(tree(&ui).contains("Restart now"));
}
