//! Boards on a real view with the theme sheet: what visitors see, scripted widgets at work,
//! arranging with the keyboard and the pointer, undo, and a hostile script kept in its box.
//! Every scenario with a scripted widget runs once with the Rhai example and once with the
//! WebAssembly one.

#![cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]

#[path = "../../remedy-ui-script-wasm/tests/hostile/mod.rs"]
mod hostile;

use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui_core::{Point, Size};
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_script_rhai::RhaiEngine;
use remedy_ui_script_wasm::WasmEngine;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};
use remedy_ui_widgets_board::*;

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(1200.0, 800.0), 1.0);
    for r in ui.take_requests() {
        if let Request::ScrollTo { scroller, offset } = r {
            ui.set_scroll_offset(scroller, offset);
        }
    }
    let _ = ui.frame(Size::new(1200.0, 800.0), 1.0);
}

fn all(ui: &Ui) -> Vec<El> {
    let mut out = Vec::new();
    for layer in ui.layers() {
        let mut stack = vec![layer];
        while let Some(e) = stack.pop() {
            stack.extend(e.child_elements());
            out.push(e);
        }
    }
    out
}

fn named(ui: &Ui, name: &str) -> El {
    all(ui)
        .into_iter()
        .find(|e| {
            e.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)).as_deref()
                == Some(name)
        })
        .unwrap_or_else(|| unreachable!("nothing named {name:?}"))
}

fn has_text(ui: &Ui, text: &str) -> bool {
    all(ui).iter().any(|e| e.info().and_then(|i| i.text).as_deref() == Some(text))
}

fn texts_like(ui: &Ui, part: &str) -> Vec<String> {
    all(ui).iter().filter_map(|e| e.info().and_then(|i| i.text)).filter(|t| t.contains(part)).collect()
}

fn widgets(ui: &Ui, w: &str) -> Vec<El> {
    all(ui).into_iter().filter(|e| e.info().is_some_and(|i| i.widget == w && !i.hidden)).collect()
}

fn pointer(ui: &Ui, at: Point, phase: PointerPhase, buttons: Buttons) {
    ui.pointer(&PointerEvent::new(phase, at).with_buttons(buttons).with_time(Duration::from_secs(1)));
}

fn click(ui: &Ui, el: &El) {
    let c = el.bounds().unwrap_or_default().center();
    pointer(ui, c, PointerPhase::Down, Buttons::PRIMARY);
    pointer(ui, c, PointerPhase::Up, Buttons::default());
    frame(ui);
}

fn key(ui: &Ui, k: Key, m: Modifiers) {
    let _ = ui.key(&KeyEvent::press(k).with_modifiers(m).with_text(None));
    frame(ui);
}

fn audit(ui: &Ui) -> Vec<String> {
    frame(ui);
    remedy_ui_devtools::audit(ui)
        .iter()
        .map(|p| {
            let what =
                ui.element(p.element).info().map(|i| remedy_ui_devtools::describe(&i)).unwrap_or_default();
            format!("{} [{what}]", p.message())
        })
        .collect()
}

/// The example widget in each language: the same widget, written by hand in Rhai and
/// compiled from Rust to WebAssembly.
fn claps() -> [Package; 2] {
    [
        Package::rhai(include_str!("../../remedy-ui-script-rhai/examples/claps.rhai")),
        Package::wasm(include_bytes!("../../remedy-ui-script-wasm/examples/claps.wasm")),
    ]
}

/// Both engines.
fn engines() -> Engines {
    Engines::new().with(Rc::new(RhaiEngine::new())).with(Rc::new(WasmEngine::new()))
}

/// Time enough for an unoptimized test build; the hostile test sets its own.
fn limits() -> Limits {
    Limits::default().time(Duration::from_secs(5))
}

fn home() -> (Board, String, String, String) {
    let (b, welcome) = Board::default().add_area("Welcome", 8, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, side) = b.add_area("Around the house", 4, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let mut text = serde_json::Map::new();
    text.insert("title".into(), "Hello, house".into());
    text.insert("body".into(), "Dinner is at seven.".into());
    let (b, note) = b.add_widget(&welcome, "text", text).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&welcome, "claps", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&side, "reactions", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    (b, welcome, side, note)
}

fn registry(claps: Package) -> Rc<Registry> {
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("claps", claps, &engines(), limits()).unwrap_or_else(|e| unreachable!("{e}")),
    ));
    Rc::new(r)
}

fn context() -> Context {
    let mut cx = Context::local(|_| {});
    cx.host.set(serde_json::json!({ "house": "the Den" }));
    cx.engines = engines();
    cx.limits = limits();
    cx
}

#[test]
fn a_visitor_sees_the_board_and_scripted_widgets_work() {
    for claps in claps() {
        a_visitor_sees_the_board(claps);
    }
}

fn a_visitor_sees_the_board(claps: Package) {
    let ui = ui();
    let (b, ..) = home();
    let state = BoardState::new(b);
    ui.root().child(board(&ui, &state, registry(claps), context()));
    frame(&ui);
    assert!(has_text(&ui, "Hello, house") && has_text(&ui, "Dinner is at seven."));
    assert!(has_text(&ui, "0 claps from the Den"), "{:?}", texts_like(&ui, "claps"));
    click(&ui, &named(&ui, "Clap"));
    click(&ui, &named(&ui, "Clap"));
    assert!(has_text(&ui, "2 claps from the Den"), "{:?}", texts_like(&ui, "claps"));
    // Reactions toggle and say whose they are.
    click(&ui, &named(&ui, "🎉, 0 reactions"));
    assert!(
        all(&ui).iter().any(|e| e
            .info()
            .and_then(|i| i.semantics)
            .and_then(|s| s.name().map(str::to_owned))
            .as_deref()
            == Some("🎉, 1 reaction, including yours"))
    );
    // No editing chrome for visitors.
    assert!(widgets(&ui, "board_item").is_empty());
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn the_owner_arranges_with_the_keyboard_and_undoes() {
    for claps in claps() {
        the_owner_arranges(claps);
    }
}

fn the_owner_arranges(claps: Package) {
    let ui = ui();
    let (b, welcome, side, note) = home();
    let state = BoardState::new(b);
    state.editing.set(true);
    ui.root().child(board(&ui, &state, registry(claps), context()));
    frame(&ui);
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
    // Move the side area down a row and widen it: Shift with an arrow resizes.
    named(&ui, "Around the house area").focus();
    frame(&ui);
    assert_eq!(state.selected.get_untracked().ok().flatten(), Some(Selection::Area(side.clone())));
    key(&ui, Key::ArrowDown, Modifiers::NONE);
    key(&ui, Key::ArrowLeft, Modifiers { shift: true, ..Modifiers::NONE });
    let a =
        state.doc.get_untracked().ok().and_then(|b| b.area(&side).cloned()).unwrap_or_else(|| unreachable!());
    assert_eq!((a.col, a.row, a.cols), (8, 1, 3));
    // Moving onto the welcome area is refused and said.
    key(&ui, Key::ArrowUp, Modifiers::NONE);
    key(&ui, Key::ArrowLeft, Modifiers::NONE);
    assert!(state.message.get_untracked().ok().flatten().is_some_and(|m| m.contains(&welcome)));
    // Undo goes back step by step.
    let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
    key(&ui, Key::Character("z".into()), ctrl);
    key(&ui, Key::Character("z".into()), ctrl);
    key(&ui, Key::Character("z".into()), ctrl);
    let a =
        state.doc.get_untracked().ok().and_then(|b| b.area(&side).cloned()).unwrap_or_else(|| unreachable!());
    assert_eq!((a.col, a.row, a.cols), (8, 0, 4));
    // The panel edits the selected widget's settings, typing being one undo step.
    named(&ui, "Text widget").focus();
    frame(&ui);
    assert_eq!(state.selected.get_untracked().ok().flatten(), Some(Selection::Widget(note.clone())));
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
    // Alt with Down moves the widget below the next; Delete removes it; undo brings it back.
    key(&ui, Key::ArrowDown, Modifiers { alt: true, ..Modifiers::NONE });
    assert_eq!(state.doc.get_untracked().ok().and_then(|b| b.widget_place(&note)), Some((0, 1)));
    key(&ui, Key::Delete, Modifiers::NONE);
    assert!(state.doc.get_untracked().ok().is_some_and(|b| b.widget(&note).is_none()));
    key(&ui, Key::Character("z".into()), ctrl);
    assert!(state.doc.get_untracked().ok().is_some_and(|b| b.widget(&note).is_some()));
}

#[test]
fn widgets_are_added_from_the_panel_and_dragged_between_areas() {
    for claps in claps() {
        widgets_are_added_and_dragged(claps);
    }
}

fn widgets_are_added_and_dragged(claps: Package) {
    let ui = ui();
    let (b, welcome, side, note) = home();
    let state = BoardState::new(b);
    state.editing.set(true);
    ui.root().child(board(&ui, &state, registry(claps), context()));
    frame(&ui);
    click(&ui, &named(&ui, "Around the house area"));
    click(&ui, &named(&ui, "Add widget"));
    let added = state.doc.get_untracked().ok().and_then(|b| b.area(&side).map(|a| a.widgets.len()));
    assert_eq!(added, Some(2), "a widget joined the selected area");
    // Drag the note onto the side area.
    let from = named(&ui, "Text widget").bounds().unwrap_or_default().center();
    let to = named(&ui, "Around the house area").bounds().unwrap_or_default().center();
    pointer(&ui, from, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&ui, Point::new(from.x + 20.0, from.y + 5.0), PointerPhase::Move, Buttons::PRIMARY);
    pointer(&ui, to, PointerPhase::Move, Buttons::PRIMARY);
    pointer(&ui, to, PointerPhase::Up, Buttons::default());
    frame(&ui);
    let b = state.doc.get_untracked().unwrap_or_default();
    assert_eq!(b.widget_place(&note).and_then(|(ai, _)| b.areas.get(ai)).map(|a| a.id.clone()), Some(side));
    assert!(b.area(&welcome).is_some_and(|a| a.widgets.iter().all(|w| w.id != note)));
}

/// Hostile widgets in each language: an endless loop, runaway recursion, a memory hog.
fn hostile_widgets() -> Vec<(&'static str, Package)> {
    let rhai = |body: &str| Package::rhai(&format!("fn render(s, h) {{ {body} }} #{{ name: \"Hostile\" }}"));
    vec![
        ("a Rhai loop", rhai("loop {}")),
        (
            "Rhai recursion",
            Package::rhai("fn down(n) { down(n + 1) } fn render(s, h) { down(0) } #{ name: \"Deep\" }"),
        ),
        ("a Rhai hog", rhai("let s = \"x\"; loop { s += s; }")),
        ("a WebAssembly loop", Package::wasm(&hostile::spin())),
        ("WebAssembly recursion", Package::wasm(&hostile::recurse())),
        ("a WebAssembly hog", Package::wasm(&hostile::hog())),
    ]
}

#[test]
fn a_hostile_script_stops_in_its_box_and_the_board_stays_usable() {
    for (what, package) in hostile_widgets() {
        let ui = ui();
        let mut r = Registry::with_builtins();
        let limits = Limits::default().time(Duration::from_millis(15)).memory(4 * 1024 * 1024);
        // Loading is fine (the trouble is only in render); rendering is stopped.
        r.add(Rc::new(
            ScriptKind::new("hostile", package, &engines(), limits)
                .unwrap_or_else(|e| unreachable!("{what}: {e}")),
        ));
        let (b, area) = Board::default().add_area("Home", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, _) =
            b.add_widget(&area, "hostile", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, _) =
            b.add_widget(&area, "text", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
        let state = BoardState::new(b);
        let mut cx = context();
        cx.limits = limits;
        let start = Instant::now();
        ui.root().child(board(&ui, &state, Rc::new(r), cx));
        frame(&ui);
        assert!(start.elapsed() < Duration::from_secs(2), "{what}: {:?}", start.elapsed());
        assert!(!texts_like(&ui, "This widget stopped").is_empty(), "{what}");
        assert!(has_text(&ui, "Welcome"), "{what}: the widgets around it still show");
        // The stop is one of the window's failures, caught by the widget's own boundary.
        let failures = ui.failures();
        assert_eq!(failures.len(), 1, "{what}: {failures:?}");
        assert!(failures.iter().all(|f| f.boundary.is_some()), "{what}: {failures:?}");
    }
}

#[test]
fn a_handler_that_stops_is_reported_and_the_widget_starts_again() {
    let src = "fn render(s, h) { ui::column([ui::text(\"ready\"), ui::button(\"Spin\", \"spin\")]) } \
               fn spin() { loop {} } #{ name: \"Spinner\" }";
    let limits = limits().operations(Some(100_000));
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("spin", Package::rhai(src), &engines(), limits)
            .unwrap_or_else(|e| unreachable!("{e}")),
    ));
    let (b, area) = Board::default().add_area("Home", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) = b.add_widget(&area, "spin", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let state = BoardState::new(b);
    let mut cx = context();
    cx.limits = limits;
    let ui = ui();
    ui.root().child(board(&ui, &state, Rc::new(r), cx));
    frame(&ui);
    assert!(has_text(&ui, "ready"));
    click(&ui, &named(&ui, "Spin"));
    let stopped = texts_like(&ui, "This widget stopped");
    assert!(stopped.iter().any(|t| t.contains("too many steps")), "{stopped:?}");
    let failures = ui.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures.iter().all(|f| f.phase == remedy_ui_view::FailurePhase::Handler), "{failures:?}");
    click(&ui, &named(&ui, "Start again"));
    assert!(has_text(&ui, "ready"), "the widget runs again");
    assert!(texts_like(&ui, "This widget stopped").is_empty());
}

#[test]
fn a_board_runs_only_the_script_languages_it_accepts() {
    let [rhai, wasm] = claps();
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("claps-rhai", rhai, &engines(), limits()).unwrap_or_else(|e| unreachable!("{e}")),
    ));
    r.add(Rc::new(
        ScriptKind::new("claps-wasm", wasm, &engines(), limits()).unwrap_or_else(|e| unreachable!("{e}")),
    ));
    let (b, area) = Board::default().add_area("Home", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&area, "claps-rhai", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&area, "claps-wasm", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let state = BoardState::new(b);
    let mut cx = context();
    cx.engines = Engines::new().with(Rc::new(RhaiEngine::new()));
    let ui = ui();
    ui.root().child(board(&ui, &state, Rc::new(r), cx));
    frame(&ui);
    assert!(has_text(&ui, "0 claps from the Den"), "the Rhai widget runs");
    let stopped = texts_like(&ui, "This widget stopped");
    assert!(stopped.iter().any(|t| t.contains("WebAssembly widgets are not accepted here")), "{stopped:?}");
}

#[test]
fn a_board_document_from_a_peer_shows_as_sent() {
    for claps in claps() {
        a_peer_board_shows_as_sent(claps);
    }
}

fn a_peer_board_shows_as_sent(claps: Package) {
    let ui = ui();
    let (b, ..) = home();
    let sent = b.to_json();
    let state = BoardState::new(Board::default());
    ui.root().child(board(&ui, &state, registry(claps), context()));
    frame(&ui);
    assert!(has_text(&ui, "Nothing here yet."));
    state.doc.set(Board::from_json(&sent).unwrap_or_else(|e| unreachable!("{e}")));
    frame(&ui);
    assert!(has_text(&ui, "Hello, house"));
    assert!(has_text(&ui, "0 claps from the Den"), "the scripted widget shows as sent");
    // A kind this app lacks is named, not hidden.
    let (b, area) = Board::default().add_area("X", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&area, "voice-room", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    state.doc.set(b);
    frame(&ui);
    assert!(!texts_like(&ui, "voice-room").is_empty());
}
