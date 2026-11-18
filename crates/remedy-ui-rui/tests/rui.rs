//! `.rui` files parsed, checked, interpreted, compiled and overridden.

use std::rc::Rc;

use remedy_ui_core::Size;
use remedy_ui_reactive::Signal;
use remedy_ui_rui::spec::Kind;
use remedy_ui_rui::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};

/// The settings screen from the design (docs/design/07), and the golden file it compiles to.
const SETTINGS: &str = include_str!("ui/settings_voice.rui");

mod compiled {
    include!("golden/settings_voice.rs");
}

trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) -> Vec<u64> {
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
    ui.take_requests()
        .into_iter()
        .filter_map(|r| match r {
            Request::Timer { id, .. } => Some(id),
            _ => None,
        })
        .collect()
}

/// Every element under `el` as `widget: name=value checked`, depth first.
fn outline(el: &El) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(el.clone(), 0usize)];
    while let Some((e, depth)) = stack.pop() {
        let Some(info) = e.info() else { continue };
        if info.hidden {
            continue;
        }
        let s = e.composed_semantics();
        out.push(format!(
            "{}{} {:?} {:?} {:?}",
            "  ".repeat(depth),
            info.widget,
            s.as_ref().and_then(|s| s.name().map(str::to_owned)),
            s.as_ref().and_then(|s| s.value_text().map(str::to_owned)),
            s.as_ref().and_then(remedy_ui_a11y::Semantics::checked_state),
        ));
        let kids = e.child_elements();
        stack.extend(kids.into_iter().rev().map(|k| (k, depth + 1)));
    }
    out
}

fn find(el: &El, name: &str) -> Option<El> {
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.composed_semantics().is_some_and(|s| s.name() == Some(name)) {
            return Some(e);
        }
        stack.extend(e.child_elements());
    }
    None
}

struct Voice {
    devices: Signal<Vec<String>>,
    device: Signal<usize>,
    gain: Signal<f64>,
    noise: Signal<bool>,
    nickname: Signal<String>,
    advanced: Signal<bool>,
    status: Signal<String>,
    saved: Rc<std::cell::Cell<u32>>,
}

impl Voice {
    fn new() -> Self {
        Voice {
            devices: Signal::new(vec!["Built-in microphone".to_owned(), "USB headset".to_owned()]),
            device: Signal::new(1),
            gain: Signal::new(100.0),
            noise: Signal::new(false),
            nickname: Signal::new("Sam".to_owned()),
            advanced: Signal::new(false),
            status: Signal::new("Ready".to_owned()),
            saved: Rc::default(),
        }
    }

    fn scope(&self) -> Scope {
        let saved = self.saved.clone();
        Scope::new()
            .items("input_devices", self.devices)
            .choice("input_device", self.device)
            .number("input_gain", self.gain)
            .flag("noise_suppression", self.noise)
            .text("nickname", self.nickname)
            .flag("advanced", self.advanced)
            .text("status", self.status)
            .command("save", move || saved.set(saved.get() + 1))
            .translator(|s| format!("«{s}»"))
    }

    fn bindings(&self) -> compiled::Bindings {
        let saved = self.saved.clone();
        compiled::Bindings {
            input_devices: self.devices,
            input_device: self.device,
            input_gain: self.gain,
            noise_suppression: self.noise,
            nickname: self.nickname,
            advanced: self.advanced,
            status: self.status,
            save: Rc::new(move || saved.set(saved.get() + 1)),
        }
    }
}

fn err(src: &str) -> RuiError {
    match Markup::parse("t.rui", src) {
        Err(e) => e,
        Ok(m) => match check(&m, &Voice::new().scope(), &Widgets::new()) {
            Err(e) => e,
            Ok(()) => unreachable!("{src} should not load"),
        },
    }
}

#[test]
fn errors_point_at_the_file_line_and_column_and_say_what_is_exposed() {
    let e = err("column {\n  switch(label: \"A\" checked: $noise_suppression)\n}");
    assert_eq!((e.at.line, e.at.col), (2, 21), "{e}");
    assert!(e.to_string().starts_with("t.rui:2:21: expected `,` or `)`"), "{e}");

    let e = err("column { text(text: \"never closed) }");
    assert_eq!((e.at.line, e.at.col), (1, 21));
    assert!(e.message.contains("never closed"));

    let e = err("column {}\nrow {}");
    assert!(e.message.contains("one element"), "{e}");

    let e = err("colum {}");
    assert!(e.message.contains("no element is called `colum`"));
    assert!(e.help.as_deref().is_some_and(|h| h.contains("column")), "{e}");

    let e = err("switch(label: \"A\", checked: $noise_suppression, colour: \"red\")");
    assert!(e.message.contains("takes no `colour`"));
    assert!(e.help.as_deref().is_some_and(|h| h.contains("checked") && h.contains("gap")));

    let e = err("switch(label: \"A\")");
    assert!(e.message.contains("needs `checked`"), "{e}");

    let e = err("switch(label: \"A\", checked: true)");
    assert!(e.message.contains("takes a flag signal"), "{e}");

    let e = err("switch(label: \"A\", checked: $noise)");
    assert_eq!(e.message, "`$noise` is not exposed to this file");
    assert_eq!(e.help.as_deref(), Some("exposed: $advanced, $noise_suppression"));

    let e = err("switch(label: \"A\", checked: $input_gain)");
    assert!(e.message.contains("is a number signal but this takes a flag"), "{e}");

    let e = err("column { switch(label: \"A\", checked: $x) slider(label: \"B\", value: $x, range: 0..1) }");
    assert!(e.message.contains("used as a number signal here but as a flag signal before"), "{e}");

    let e = err("button(label: \"Go\", on_press: !launch)");
    assert_eq!(e.help.as_deref(), Some("exposed: !save"));

    let e = err("column(align: middle) {}");
    assert!(e.help.as_deref().is_some_and(|h| h.contains("center")));

    let e = err("text(text: \"a\") { text(text: \"b\") }");
    assert!(e.message.contains("holds no children"));

    let e = err("slider(label: \"A\", value: $input_gain, range: 10..0)");
    assert!(e.message.contains("end before it starts"));

    let deep = format!("{}{}", "column {".repeat(500), "}".repeat(500));
    let e = err(&deep);
    assert!(e.message.contains("nest more than"), "{e}");

    let e = err("text(text: @x(\"a\"))");
    assert_eq!(e.help.as_deref(), Some("use @t(\"text\")"));
}

#[test]
fn an_interpreted_screen_follows_and_writes_the_apps_signals() {
    let ui = ui();
    let v = Voice::new();
    let m = Markup::parse("settings_voice.rui", SETTINGS).must("parses");
    let el = build_markup(&ui, &m, &v.scope(), &Widgets::new()).must("builds");
    let _ = ui.root().child(el.clone());
    frame(&ui);

    // Translations go through the app's catalog.
    let noise = find(&el, "«Noise suppression»").must("the switch, translated");
    assert!(ui.accessibility_action(noise.node_id(), remedy_ui_a11y::Action::Click));
    frame(&ui);
    assert!(v.noise.get(), "the switch writes the app's signal");
    v.noise.set(false);
    frame(&ui);
    assert_eq!(noise.composed_semantics().and_then(|s| s.checked_state()), Some(false), "and follows it");

    // A text field and its signal follow each other.
    let nick = find(&el, "«Nickname»").must("the field");
    assert!(ui.accessibility_set_value(nick.node_id(), "Alex"));
    frame(&ui);
    assert_eq!(v.nickname.get(), "Alex");
    v.nickname.set("Kim".to_owned());
    frame(&ui);
    assert_eq!(
        nick.composed_semantics().and_then(|s| s.value_text().map(str::to_owned)).as_deref(),
        Some("Kim")
    );

    // Live text.
    assert!(outline(&el).iter().any(|l| l.contains("Ready")));
    v.status.set("Saved".to_owned());
    frame(&ui);
    assert!(outline(&el).iter().any(|l| l.contains("Saved")));

    // A list of choices rebuilds when it changes; the choice is the app's and carries over.
    v.devices.set(vec!["Built-in microphone".to_owned(), "USB headset".to_owned(), "Webcam".to_owned()]);
    frame(&ui);
    assert!(outline(&el).iter().any(|l| l.contains("USB headset")), "{:#?}", outline(&el));
    assert_eq!(v.device.get(), 1);

    // Shown only while a flag is on.
    assert!(
        find(&el, "«Echo cancellation»")
            .and_then(|e| e.info())
            .is_none_or(|_| { !outline(&el).iter().any(|l| l.contains("Echo cancellation")) })
    );
    v.advanced.set(true);
    frame(&ui);
    assert!(outline(&el).iter().any(|l| l.contains("Echo cancellation")));

    // Commands.
    let save = find(&el, "«Save»").must("the button");
    assert!(ui.accessibility_action(save.node_id(), remedy_ui_a11y::Action::Click));
    assert_eq!(v.saved.get(), 1);
}

#[test]
fn the_compiled_screen_builds_what_the_interpreter_builds() {
    // (tests/golden.rs holds the compiler to the golden file this test runs.)
    assert_eq!(compiled::FILE, "settings_voice.rui");
    assert_eq!(compiled::SOURCE, SETTINGS);

    let tr = |s: &str| format!("«{s}»");
    let (ui_a, ui_b) = (ui(), ui());
    let (va, vb) = (Voice::new(), Voice::new());
    let a = compiled::build(&ui_a, &va.bindings(), &tr);
    let m = Markup::parse("settings_voice.rui", SETTINGS).must("parses");
    let b = build_markup(&ui_b, &m, &vb.scope(), &Widgets::new()).must("builds");
    let _ = ui_a.root().child(a.clone());
    let _ = ui_b.root().child(b.clone());
    frame(&ui_a);
    frame(&ui_b);
    assert_eq!(outline(&a), outline(&b));

    // And it is live the same way.
    let noise = find(&a, "«Noise suppression»").must("switch");
    assert!(ui_a.accessibility_action(noise.node_id(), remedy_ui_a11y::Action::Click));
    assert!(va.noise.get());
    // Its scope lets a person's own version load against the same signals.
    let scope = va.bindings().scope(tr);
    assert!(check(&m, &scope, &Widgets::new()).is_ok());
}

#[test]
fn keywords_become_raw_fields_and_folders_compile() {
    let rust =
        codegen::compile_in(codegen::DIRECT_ROOT, "k.rui", "switch(label: \"A\", checked: $type)", &[])
            .must("compiles");
    assert!(rust.contains("pub r#type: remedy_ui_markup::rt::reactive::Signal<bool>,"));
    let dir = std::env::temp_dir().join(format!("rui-dir-{}", std::process::id()));
    let out = dir.join("out");
    std::fs::create_dir_all(&dir).must("dir");
    std::fs::write(dir.join("my-screen.rui"), "column(gap: s2) { text(text: \"hi\") }").must("write");
    std::fs::write(dir.join("notes.txt"), "not markup").must("write");
    let written = build::compile_dir_to(codegen::DIRECT_ROOT, &dir, &out, &[], false).must("compiles");
    assert_eq!(written, vec![out.join("my_screen.rs")]);
    std::fs::write(dir.join("bad.rui"), "column {").must("write");
    let e = build::compile_dir_to(codegen::DIRECT_ROOT, &dir, &out, &[], false)
        .err()
        .must("a broken file fails the build");
    assert!(e.to_string().starts_with("bad.rui:1:"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn apps_add_their_own_elements() {
    let ui = ui();
    let mut widgets = Widgets::new();
    widgets.register(
        "meter",
        &[spec::Param { name: "level", kind: Kind::Level, required: true }],
        false,
        |ui, call| {
            let level = call.level("level").unwrap_or_else(|| Signal::new(None));
            remedy_ui_rui::rt::w::progress(ui, "Input level", level)
        },
    );
    let level = Signal::new(Some(0.25_f32));
    let scope = Scope::new().level("input_level", level);
    let m = Markup::parse("m.rui", "column { meter(level: $input_level) }").must("parses");
    let el = build_markup(&ui, &m, &scope, &widgets).must("builds");
    let _ = ui.root().child(el.clone());
    frame(&ui);
    assert!(find(&el, "Input level").is_some());
    let rust = codegen::compile_in(
        codegen::DIRECT_ROOT,
        "m.rui",
        "column { meter(level: $input_level) }",
        &[codegen::Custom {
            name: "meter".to_owned(),
            params: vec![spec::Param { name: "level", kind: Kind::Level, required: true }],
            children: false,
            path: "crate::meter".to_owned(),
        }],
    )
    .must("compiles");
    assert!(rust.contains("crate::meter(ui, remedy_ui_markup::Call::new().arg(\"level\", remedy_ui_markup::Bound::Binding(remedy_ui_markup::Binding::Level(b.input_level))))"), "{rust}");
}

#[test]
fn peoples_layouts_load_fall_back_with_a_notice_and_reset_undoably() {
    let ui = ui();
    let v = Voice::new();
    let dir = std::env::temp_dir().join(format!("rui-layouts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let layouts = Layouts::new(&dir, Widgets::new());
    let tr = |s: &str| format!("«{s}»");
    let b = v.bindings();
    let built_in = move |ui: &Ui| compiled::build(ui, &b, &tr);
    let layout = layouts.build(&ui, "settings_voice.rui", &v.scope(), built_in);
    let _ = ui.root().child(layout.el());
    frame(&ui);
    assert!(!layout.is_custom());
    assert!(find(&layout.el(), "«Noise suppression»").is_some(), "the built-in screen");

    // A person's own: just the switch, in a row.
    let mine = "row(gap: s4) { switch(label: \"Quiet\", checked: $noise_suppression) }";
    assert!(matches!(
        layout.save("row { switch(label: \"A\", checked: $nope) }"),
        Err(SaveError::Invalid(_))
    ));
    layout.save(mine).must("saves");
    frame(&ui);
    assert!(layout.is_custom());
    let quiet = find(&layout.el(), "Quiet").must("their switch");
    assert!(ui.accessibility_action(quiet.node_id(), remedy_ui_a11y::Action::Click));
    assert!(v.noise.get(), "bound to the same signal");
    assert!(find(&layout.el(), "«Noise suppression»").is_none());
    assert_eq!(ui.next_undo().map(|h| h.label).as_deref(), Some("Change layout settings_voice.rui"));

    // A broken file (edited by hand) falls back, says why, and offers Reset.
    std::fs::write(dir.join("settings_voice.rui"), "row { switch(label: \"Quiet\" }").must("write");
    layout.reload();
    frame(&ui);
    assert!(layout.error().is_some());
    assert!(find(&layout.el(), "«Noise suppression»").is_some(), "the built-in screen is back");
    assert!(
        outline(&layout.el()).iter().any(|l| l.contains("could not load")),
        "{:#?}",
        outline(&layout.el())
    );
    let reset = find(&layout.el(), "Reset").must("a reset button");
    assert!(ui.accessibility_action(reset.node_id(), remedy_ui_a11y::Action::Click));
    frame(&ui);
    assert!(layout.error().is_none() && !layout.is_custom());
    assert!(!dir.join("settings_voice.rui").exists());
    // Reset is undoable.
    assert!(ui.undo_change());
    frame(&ui);
    assert!(dir.join("settings_voice.rui").exists());
    assert!(layout.error().is_some(), "the broken file came back as it was");

    // Watching: a change on disk shows without a reload call.
    std::fs::write(dir.join("settings_voice.rui"), mine).must("write");
    layout.watch();
    for id in frame(&ui) {
        ui.fire_timer(id);
    }
    // File times can be coarse; the watcher compares against what it last loaded.
    std::thread::sleep(std::time::Duration::from_millis(20));
    for _ in 0..3 {
        for id in frame(&ui) {
            ui.fire_timer(id);
        }
    }
    assert!(find(&layout.el(), "Quiet").is_some(), "the edited file shows");
    let _ = std::fs::remove_dir_all(&dir);
}
