//! Boards on a real view with the theme sheet: what visitors see, scripted widgets at work,
//! arranging with the keyboard and the pointer, undo, and a hostile script kept in its box.
//! Every scenario with a scripted widget runs once with the RemedyLua example and once with
//! the WebAssembly one.

#![cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]

#[path = "../../../remedy-ui-engine/tests/script_wasm/hostile/mod.rs"]
mod hostile;

use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_core::{Point, Size};
use remedy_ui_engine::script_lua::LuaEngine;
use remedy_ui_engine::script_wasm::WasmEngine;
use remedy_ui_engine::view::{El, Request, Ui};
use remedy_ui_text::FontCollection;
use remedy_ui_widgets::board::*;

fn ui() -> Ui {
    let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts)
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
    remedy_ui_widgets::devtools::audit(ui)
        .iter()
        .map(|p| {
            let what = ui
                .element(p.element)
                .info()
                .map(|i| remedy_ui_widgets::devtools::describe(&i))
                .unwrap_or_default();
            format!("{} [{what}]", p.message())
        })
        .collect()
}

/// The example widget in each language: the same widget, written by hand in RemedyLua and
/// compiled from Rust to WebAssembly.
fn claps() -> [Package; 2] {
    [
        Package::lua(include_str!("../../../remedy-ui-engine/examples/script_lua/claps.lua")),
        Package::wasm(include_bytes!("../../../remedy-ui-engine/examples/script_wasm/claps.wasm")),
    ]
}

/// Both engines.
fn engines() -> Engines {
    Engines::new().with(Rc::new(LuaEngine::new())).with(Rc::new(WasmEngine::new()))
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
    let (b, _, side, note) = home();
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
    assert!(state.message.get_untracked().ok().flatten().is_some_and(|m| m.contains("Welcome")));
    // Undo goes back step by step.
    let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
    key(&ui, Key::Character("z".into()), ctrl);
    key(&ui, Key::Character("z".into()), ctrl);
    key(&ui, Key::Character("z".into()), ctrl);
    let a =
        state.doc.get_untracked().ok().and_then(|b| b.area(&side).cloned()).unwrap_or_else(|| unreachable!());
    assert_eq!((a.col, a.row, a.cols), (8, 0, 4));
    // The panel edits the selected widget's settings, typing being one undo step.
    named(&ui, "Hello, house widget").focus();
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
    let from = named(&ui, "Hello, house widget").bounds().unwrap_or_default().center();
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
    let lua =
        |body: &str| Package::lua(&format!("return {{ name = 'Hostile', render = function() {body} end }}"));
    vec![
        ("a Lua loop", lua("while true do end")),
        (
            "Lua recursion",
            Package::lua(
                "local function down(n) return down(n + 1) + 0 end return { name = 'Deep', render = function() down(0) end }",
            ),
        ),
        ("a Lua hog", lua("local s = 'x' while true do s = s .. s end")),
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
    let src = "function spin() while true do end end\n\
               return { name = 'Spinner', render = function() return ui.column{ ui.text('ready'), ui.button('Spin', 'spin') } end }";
    let limits = limits().operations(Some(100_000));
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("spin", Package::lua(src), &engines(), limits)
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
    assert!(
        failures.iter().all(|f| f.phase == remedy_ui_engine::view::FailurePhase::Handler),
        "{failures:?}"
    );
    click(&ui, &named(&ui, "Start again"));
    assert!(has_text(&ui, "ready"), "the widget runs again");
    assert!(texts_like(&ui, "This widget stopped").is_empty());
}

#[test]
fn a_board_runs_only_the_script_languages_it_accepts() {
    let [lua, wasm] = claps();
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("claps-lua", lua, &engines(), limits()).unwrap_or_else(|e| unreachable!("{e}")),
    ));
    r.add(Rc::new(
        ScriptKind::new("claps-wasm", wasm, &engines(), limits()).unwrap_or_else(|e| unreachable!("{e}")),
    ));
    let (b, area) = Board::default().add_area("Home", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&area, "claps-lua", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&area, "claps-wasm", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let state = BoardState::new(b);
    let mut cx = context();
    cx.engines = Engines::new().with(Rc::new(LuaEngine::new()));
    let ui = ui();
    ui.root().child(board(&ui, &state, Rc::new(r), cx));
    frame(&ui);
    assert!(has_text(&ui, "0 claps from the Den"), "the Lua widget runs");
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

fn find_named(ui: &Ui, name: &str) -> Option<El> {
    all(ui).into_iter().find(|e| {
        e.info().is_some_and(|i| {
            !i.hidden && i.semantics.and_then(|s| s.name().map(str::to_owned)).as_deref() == Some(name)
        })
    })
}

/// The text field named `name`.
fn field(ui: &Ui, name: &str) -> El {
    all(ui)
        .into_iter()
        .find(|e| {
            e.info().is_some_and(|i| {
                i.field && i.semantics.and_then(|s| s.name().map(str::to_owned)).as_deref() == Some(name)
            })
        })
        .unwrap_or_else(|| unreachable!("no field named {name:?}"))
}

fn type_text(ui: &Ui, text: &str) {
    for c in text.chars() {
        let s = c.to_string();
        let _ = ui.key(&KeyEvent::press(Key::Character(s.clone())).with_text(Some(s)));
    }
}

fn setting(state: &BoardState, id: &str, key: &str) -> Option<serde_json::Value> {
    state.doc.get_untracked().ok().and_then(|b| b.widget(id).and_then(|w| w.settings.get(key).cloned()))
}

/// A kind with a setting of each newer kind: a moment, a labeled choice, a list, a picture.
struct Event;

impl WidgetKind for Event {
    fn id(&self) -> &str {
        "event"
    }
    fn name(&self) -> String {
        "Event".into()
    }
    fn settings(&self) -> Vec<SettingSpec> {
        let rooms = vec![("7".to_owned(), "Lobby".to_owned()), ("8".to_owned(), "Stage".to_owned())];
        vec![
            SettingSpec::new("when", "When", SettingKind::Time, serde_json::Value::Null),
            SettingSpec::new("room", "Room", SettingKind::LabeledChoice(rooms), "7".into()),
            SettingSpec::new("tags", "Tags", SettingKind::List, serde_json::json!(["music"])),
            SettingSpec::new("art", "Poster", SettingKind::Picture, "".into()),
        ]
    }
    fn build(&self, ui: &Ui, _id: &str, s: &serde_json::Map<String, serde_json::Value>, _cx: &Context) -> El {
        remedy_ui_widgets::base::text(
            ui,
            format!("An event in room {}", s.get("room").cloned().unwrap_or_default()),
        )
    }
}

#[test]
fn every_setting_kind_has_its_editor_and_typing_settles_once() {
    let ui = ui();
    let mut r = Registry::with_builtins();
    r.add(Rc::new(Event));
    let (b, area) = Board::default().add_area("Events", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, id) =
        b.add_widget(&area, "event", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let state = BoardState::with_options(b, BoardOptions::default().settle_after(None));
    state.editing.set(true);
    let mut cx = context();
    cx.picture =
        Some(Rc::new(|ui: &Ui, src: &str| remedy_ui_widgets::base::text(ui, format!("Showing {src}"))));
    cx.choose_picture = Some(Rc::new(|done: Box<dyn FnOnce(String)>| done("asset:9".into())));
    ui.root().child(board(&ui, &state, Rc::new(r), cx));
    state.selected.set(Some(Selection::Widget(id.clone())));
    frame(&ui);
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
    // The moment is not set until chosen; the choice shows its label, not its value.
    assert!(has_text(&ui, "Not set") && find_named(&ui, "When: date").is_some());
    assert!(has_text(&ui, "Lobby"), "{:?}", texts_like(&ui, "obby"));
    // The app chooses and shows pictures.
    click(&ui, &named(&ui, "Choose picture"));
    assert_eq!(setting(&state, &id, "art"), Some("asset:9".into()));
    assert!(has_text(&ui, "Showing asset:9"));
    // Typing in a list item: one step, the field keeps focus, settled once it is left.
    let settled_before = state.settled.get_untracked().ok();
    let tags = field(&ui, "Tags 1");
    tags.focus();
    frame(&ui);
    type_text(&ui, "!!");
    frame(&ui);
    let typed = setting(&state, &id, "tags");
    assert!(typed.as_ref().is_some_and(|v| v.to_string().contains("!!")), "{typed:?}");
    assert_eq!(ui.tree().focused(), Some(field(&ui, "Tags 1").id()), "the panel kept the field");
    assert_eq!(state.settled.get_untracked().ok(), settled_before, "not settled while typing");
    named(&ui, "Events area").focus();
    frame(&ui);
    assert_eq!(state.settled.get_untracked().ok(), state.doc.get_untracked().ok(), "settled when left");
    // Adding an item focuses it; removing one takes it out.
    state.selected.set(Some(Selection::Widget(id.clone())));
    frame(&ui);
    click(&ui, &named(&ui, "Add to Tags"));
    assert_eq!(setting(&state, &id, "tags").and_then(|v| v.as_array().map(Vec::len)), Some(2));
    assert_eq!(ui.tree().focused(), Some(field(&ui, "Tags 2").id()));
    click(&ui, &named(&ui, "Remove Tags 1"));
    assert_eq!(setting(&state, &id, "tags"), Some(serde_json::json!([""])));
    // A set moment can be cleared; a stored choice no longer offered says so.
    state.apply(state.current().set_setting(&id, "when", 1_795_093_500.into()));
    state.apply(state.current().set_setting(&id, "room", "99".into()));
    frame(&ui);
    assert!(has_text(&ui, "99 (not available)"), "{:?}", texts_like(&ui, "99"));
    click(&ui, &named(&ui, "Clear"));
    assert_eq!(setting(&state, &id, "when"), Some(serde_json::Value::Null));
    assert!(has_text(&ui, "Not set"));
}

#[test]
fn typing_settles_after_a_pause() {
    let ui = ui();
    let (b, _, _, note) = home();
    let state =
        BoardState::with_options(b, BoardOptions::default().settle_after(Some(Duration::from_millis(300))));
    state.editing.set(true);
    ui.root().child(board(&ui, &state, registry(claps()[0].clone()), context()));
    state.selected.set(Some(Selection::Widget(note.clone())));
    frame(&ui);
    let title = field(&ui, "Title");
    title.focus();
    frame(&ui);
    let _ = ui.take_requests();
    type_text(&ui, "!");
    assert!(state.is_typing());
    let timers: Vec<u64> = ui
        .take_requests()
        .into_iter()
        .filter_map(|r| if let Request::Timer { id, .. } = r { Some(id) } else { None })
        .collect();
    assert!(!timers.is_empty(), "a settle timer waits");
    for t in timers {
        ui.fire_timer(t);
    }
    assert!(!state.is_typing(), "the pause finished the typing");
    assert_eq!(state.settled.get_untracked().ok(), state.doc.get_untracked().ok());
}

#[test]
fn the_board_speaks_the_apps_words() {
    let words = BoardLabels::default().translated(|en| {
        match en {
            "Undo" => "Rückgängig",
            "{name} area" => "Bereich {name}",
            "Add widget" => "Widget hinzufügen",
            "Nothing here yet." => "Noch nichts hier.",
            "{name} widget" => "Widget {name}",
            other => other,
        }
        .to_owned()
    });
    let ui = ui();
    let (b, ..) = home();
    let state = BoardState::new(b);
    state.editing.set(true);
    let mut cx = context();
    cx.labels = words.clone();
    let mut r = Registry::with_builtins_in(&words);
    r.add(Rc::new(
        ScriptKind::new("claps", claps()[0].clone(), &engines(), limits())
            .unwrap_or_else(|e| unreachable!("{e}")),
    ));
    ui.root().child(board(&ui, &state, Rc::new(r), cx.clone()));
    frame(&ui);
    assert!(find_named(&ui, "Rückgängig").is_some());
    assert!(find_named(&ui, "Bereich Welcome").is_some());
    assert!(find_named(&ui, "Widget Hello, house").is_some());
    click(&ui, &named(&ui, "Bereich Welcome"));
    assert!(find_named(&ui, "Widget hinzufügen").is_some());
    let ui = ui_empty(&cx);
    assert!(has_text(&ui, "Noch nichts hier."));
}

fn ui_empty(cx: &Context) -> Ui {
    let ui = ui();
    let state = BoardState::new(Board::default());
    ui.root().child(board(&ui, &state, Rc::new(Registry::with_builtins()), cx.clone()));
    frame(&ui);
    ui
}

#[test]
fn fixed_rows_one_widget_areas_and_a_panel_apart() {
    let ui = ui();
    let (b, welcome, side, _) = home();
    let options = BoardOptions::default().row_height(Some(120.0)).panel(Panel::Off);
    let state = BoardState::with_options(b, options);
    state.editing.set(true);
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("claps", claps()[0].clone(), &engines(), limits())
            .unwrap_or_else(|e| unreachable!("{e}")),
    ));
    r.set_addable("links", false);
    r.set_addable("reactions", false);
    r.set_addable("claps", false);
    let r = Rc::new(r);
    ui.root().child(board(&ui, &state, r.clone(), context()));
    frame(&ui);
    assert!(find_named(&ui, "Undo").is_none(), "no panel with the board");
    let h = named(&ui, "Welcome area").bounds().unwrap_or_default().height();
    assert!((h - 120.0).abs() < 0.5, "a fixed row: {h}");
    // The app's own panel, driven by selecting from outside.
    ui.root().child(board_panel(&ui, &state, r.clone(), &context()));
    state.selected.set(Some(Selection::Area(side.clone())));
    frame(&ui);
    // Only the kinds this viewer may add are offered: the one left is text.
    click(&ui, &named(&ui, "Add widget"));
    let kinds: Vec<String> = state
        .doc
        .get_untracked()
        .ok()
        .and_then(|b| b.area(&side).map(|a| a.widgets.iter().map(|w| w.kind.clone()).collect()))
        .unwrap_or_default();
    assert_eq!(kinds, ["reactions", "text"]);
    // One widget an area: a full area is not offered more.
    state.options.update(|o| o.one_widget_per_area = true);
    state.selected.set(Some(Selection::Area(welcome.clone())));
    frame(&ui);
    assert!(find_named(&ui, "Add widget").is_none());
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The `{name}` placeholders of a word.
fn placeholders(word: &str) -> std::collections::BTreeSet<String> {
    word.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_owned())).collect()
}

#[test]
fn localized_words_keep_every_placeholder() {
    let en = BoardLabels::default();
    let de = BoardLabels::localized("de-DE");
    assert_ne!(de, en);
    assert_eq!(de.undo, "Rückgängig");
    assert_eq!(de.reactions.group, "Reaktionen", "nested words are localized too");
    let words =
        |l: &BoardLabels| -> Vec<String> { l.words().into_iter().map(|(_, w)| w.to_owned()).collect() };
    let (en_words, de_words) = (words(&en), words(&de));
    assert_eq!(en_words.len(), 63, "{en_words:?}");
    assert_eq!(en.words().first().map(|(id, _)| id.as_str()), Some("undo"));
    assert_eq!(en.words().last().map(|(id, _)| id.as_str()), Some("reaction-spoken-mine"));
    for (e, d) in en_words.iter().zip(&de_words) {
        assert_eq!(placeholders(e), placeholders(d), "{e:?} became {d:?}");
        assert!(!d.contains(['\u{2068}', '\u{2069}', '\u{27E8}']), "{d:?}");
    }
    // In English the catalog says what the defaults say.
    assert_eq!(words(&BoardLabels::localized("en")), en_words);
    // Every locale keeps the placeholders too.
    for l in remedy_ui_engine::view::strings::LOCALES {
        for (e, w) in en_words.iter().zip(&words(&BoardLabels::localized(l))) {
            assert_eq!(placeholders(e), placeholders(w), "{l}: {e:?} became {w:?}");
        }
    }
}

#[test]
fn a_board_speaks_the_windows_language_with_its_plurals() {
    let ui = ui();
    ui.set_locale("ar");
    let (b, _) = Board::default().add_area("", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let area = b.areas.first().map(|a| a.id.clone()).unwrap_or_default();
    let (b, _) =
        b.add_widget(&area, "reactions", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let state = BoardState::new(b);
    ui.root().child(board(&ui, &state, Rc::new(Registry::with_builtins()), Context::local(|_| {})));
    frame(&ui);
    // The built-in widget's default question, and its reactions counted in Arabic.
    assert!(has_text(&ui, "كيف حال الجميع؟"));
    let _ = named(&ui, "التفاعلات");
    click(&ui, &named(&ui, "🎉، لا تفاعلات"));
    let _ = named(&ui, "🎉، تفاعل واحد، بما في ذلك تفاعلك");
    // The locale changes: the board follows.
    ui.set_locale("ru");
    frame(&ui);
    let _ = named(&ui, "🎉, 1 реакция, включая вашу");
    // Words the app sets are used as they are, whatever the locale.
    let ui = self::ui();
    ui.set_locale("ru");
    let state = BoardState::new(Board::default());
    let mut cx = Context::local(|_| {});
    cx.labels.empty_board_viewing = "Nobody home".into();
    ui.root().child(board(&ui, &state, Rc::new(Registry::with_builtins()), cx));
    frame(&ui);
    assert!(has_text(&ui, "Nobody home") && has_text(&ui, "Empty board"));
}

#[test]
fn toggling_a_reaction_keeps_its_button_and_the_keyboards_focus() {
    use remedy_ui_core::reactive::Signal;
    let ui = ui();
    let shown = Signal::new(vec![Reaction { emoji: "👍".into(), count: 2, mine: false }]);
    let offered = vec!["👍".to_owned(), "❤️".to_owned(), "🎉".to_owned()];
    ui.root().child(reactions(&ui, shown, offered, move |e| shown.update(|v| reactions::toggle(v, e))));
    frame(&ui);
    let thumb = named(&ui, "👍, 2 reactions");
    thumb.focus();
    key(&ui, Key::Enter, Modifiers::default());
    assert_eq!(named(&ui, "👍, 3 reactions, including yours").id(), thumb.id(), "the same button");
    assert!(thumb.is_focused());
    // A first reaction moves its button before the ones nobody chose: it stays focused.
    let party = named(&ui, "🎉, 0 reactions");
    party.focus();
    key(&ui, Key::Enter, Modifiers::default());
    assert_eq!(named(&ui, "🎉, 1 reaction, including yours").id(), party.id());
    assert!(party.is_focused());
    let left = |e: &El| e.bounds().unwrap_or_default().min.x;
    assert!(left(&thumb) < left(&party) && left(&party) < left(&named(&ui, "❤️, 0 reactions")));
    assert_eq!(widgets(&ui, "button").len(), 3);
    // Taken back, an emoji nobody offered goes with its button.
    shown.set(vec![Reaction { emoji: "🦊".into(), count: 1, mine: false }]);
    frame(&ui);
    assert_eq!(widgets(&ui, "button").len(), 4);
    shown.set(Vec::new());
    frame(&ui);
    assert_eq!(widgets(&ui, "button").len(), 3);
}
