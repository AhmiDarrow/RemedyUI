//! The update banner follows the update's state.

use remedy_ui_core::Size;
use remedy_ui_core::reactive::Signal;
use remedy_ui_services_update::{Release, UpdateState, update_banner};
use remedy_ui_engine::view::Ui;

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
    state.set(UpdateState::Available(Release::new("v0.2.0", "")));
    let t = tree(&ui);
    assert!(t.contains("Version 0.2.0 is available.") && t.contains("Install"), "{t}");
    state.set(UpdateState::Downloading(Some(0.5)));
    let t = tree(&ui);
    assert!(t.contains("Downloading") && !t.contains("\"Install\""), "{t}");
    state.set(UpdateState::Ready);
    assert!(tree(&ui).contains("Restart now"));
}

#[test]
fn the_banner_speaks_the_app_s_language() {
    let ui = ui();
    ui.connect_accessibility();
    let state = Signal::new(UpdateState::Idle);
    let texts = {
        let mut v_ = remedy_ui_services_update::BannerText::default();
        v_.available = "Version {version} ist verfügbar.".into();
        v_.install = "Update installieren".into();
        v_.restart = None;
        v_.failure = std::rc::Rc::new(|e: &str| format!("Fehler: {e}"));
        v_
    };
    ui.root().child(remedy_ui_services_update::update_banner_with(&ui, state, &texts, || {}, || {}));
    let tree = |ui: &Ui| {
        let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
        format!("{:?}", ui.take_accessibility_update())
    };
    let _ = tree(&ui);
    state.set(UpdateState::Available(Release::new("v0.5.0", "")));
    let t = tree(&ui);
    assert!(t.contains("Version 0.5.0 ist verfügbar.") && t.contains("Update installieren"), "{t}");
    state.set(UpdateState::Ready);
    assert!(!tree(&ui).contains("Restart"), "no Restart button when the app has none");
    state.set(UpdateState::Failed("disk full".into()));
    assert!(tree(&ui).contains("Fehler: disk full"));
}

#[test]
fn the_banner_follows_the_window_s_locale() {
    let ui = ui();
    ui.set_locale("de-DE");
    ui.connect_accessibility();
    let state = Signal::new(UpdateState::Available(Release::new("v0.2.0", "")));
    ui.root().child(update_banner(&ui, state, || {}, || {}));
    let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
    let t = format!("{:?}", ui.take_accessibility_update());
    assert!(t.contains("Version 0.2.0 ist verfügbar.") && t.contains("Installieren"), "{t}");
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_services_update::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}
