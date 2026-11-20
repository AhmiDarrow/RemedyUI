//! Prism effects in `.rui` markup: the `effect` element draws a built-in effect, holds what
//! the file puts inside it, and a number signal drives one of its parameters live.

use remedy_ui::effects::register_markup;
use remedy_ui::markup::{Markup, Scope, Widgets, build_markup, check};
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::{AppConfig, WindowSpec};

#[test]
fn markup_draws_effects_and_drives_their_parameters() {
    let heat = Signal::new(1.0_f64);
    let h2 = heat;
    let mut t = TestApp::new(AppConfig::new("Effects markup"), move |app| {
        // No ambient motion: a frame draws the effect again only when something changed.
        app.set_ambient_policy(remedy_ui::view::AmbientPolicy::off());
        let _ = app.open(WindowSpec::new("Fire").size(320.0, 200.0), move |ui, _| {
            let mut widgets = Widgets::new();
            register_markup(&mut widgets);
            let scope = Scope::new().number("heat", h2);
            let src = r#"effect(name: "fire", param: "intensity", value: $heat) { text(text: "Warm") }"#;
            let Ok(m) = Markup::parse("fire.rui", src) else { unreachable!("parses") };
            let Ok(el) = build_markup(ui, &m, &scope, &widgets) else { unreachable!("builds") };
            ui.root().child(el.grow(1.0));
        });
    });
    t.frames(3);
    let Some(ui) = t.ui() else { unreachable!("a window is open") };
    let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
    assert!(names.iter().any(|n| n == "Warm"), "what the file put inside shows: {names:?}");
    t.frames(2);
    heat.set(0.3);
    t.frames(1);
    // The new value reaches the effect: that frame draws it again, and only it.
    let repainted = ui.last_frame_stats().repainted;
    assert_eq!(repainted.len(), 1, "the signal drew the effect again, alone: {repainted:?}");

    // Files are checked like any other: an unknown argument is refused with where it is.
    let mut widgets = Widgets::new();
    register_markup(&mut widgets);
    let Ok(bad) = Markup::parse("bad.rui", r#"effect(name: "fire", shader: "x")"#) else {
        unreachable!("parses")
    };
    let e = check(&bad, &Scope::new(), &widgets).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("bad.rui:1:"), "{e}");
}
