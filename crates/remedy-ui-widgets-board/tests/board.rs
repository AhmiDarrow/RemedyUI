//! Boards on a real view with the theme sheet: what visitors see, scripted widgets at work,
//! arranging with the keyboard and the pointer, undo, and a hostile script kept in its box.

use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui_core::{Point, Size};
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
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

const CLAPS: &str = r#"
return {
  name = "Claps",
  settings = { { key = "title", label = "Title", kind = "text", default = "Claps" } },
  render = function(settings, state, ui, host)
    state.claps = state.claps or 0
    return ui.column({ gap = 8 }, {
      ui.heading(settings.title),
      ui.text(state.claps .. " claps from " .. (host.house or "nobody")),
      ui.button("Clap", function() state.claps = state.claps + 1 end),
    })
  end,
}
"#;

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

fn registry() -> Rc<Registry> {
    let mut r = Registry::with_builtins();
    r.add(Rc::new(
        ScriptKind::new("claps", CLAPS, Limits::default()).unwrap_or_else(|e| unreachable!("{e}")),
    ));
    Rc::new(r)
}

fn context() -> Context {
    let cx = Context::local(|_| {});
    cx.host.set(serde_json::json!({ "house": "the Den" }));
    cx
}

#[test]
fn a_visitor_sees_the_board_and_scripted_widgets_work() {
    let ui = ui();
    let (b, ..) = home();
    let state = BoardState::new(b);
    ui.root().child(board(&ui, &state, registry(), context()));
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
    let ui = ui();
    let (b, welcome, side, note) = home();
    let state = BoardState::new(b);
    state.editing.set(true);
    ui.root().child(board(&ui, &state, registry(), context()));
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
    let ui = ui();
    let (b, welcome, side, note) = home();
    let state = BoardState::new(b);
    state.editing.set(true);
    ui.root().child(board(&ui, &state, registry(), context()));
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

#[test]
fn a_hostile_script_stops_in_its_box_and_the_board_stays_usable() {
    let ui = ui();
    let mut r = Registry::with_builtins();
    let spin = "return { name = 'Spinner', render = function(s, st, ui) while true do end end }";
    let limits = Limits { time: Duration::from_millis(15), ..Limits::default() };
    // Loading is fine (the loop is only in render); rendering is stopped.
    r.add(Rc::new(ScriptKind::new("spin", spin, limits).unwrap_or_else(|e| unreachable!("{e}"))));
    let (b, area) = Board::default().add_area("Home", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) = b.add_widget(&area, "spin", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) = b.add_widget(&area, "text", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    let state = BoardState::new(b);
    let mut cx = context();
    cx.limits = limits;
    let start = Instant::now();
    ui.root().child(board(&ui, &state, Rc::new(r), cx));
    frame(&ui);
    assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
    assert!(!texts_like(&ui, "This widget stopped").is_empty());
    assert!(has_text(&ui, "Welcome"), "the widgets around it still show");
}

#[test]
fn a_board_document_from_a_peer_shows_as_sent() {
    let ui = ui();
    let (b, ..) = home();
    let sent = b.to_json();
    let state = BoardState::new(Board::default());
    ui.root().child(board(&ui, &state, registry(), context()));
    frame(&ui);
    assert!(has_text(&ui, "Nothing here yet."));
    state.doc.set(Board::from_json(&sent).unwrap_or_else(|e| unreachable!("{e}")));
    frame(&ui);
    assert!(has_text(&ui, "Hello, house"));
    // A kind this app lacks is named, not hidden.
    let (b, area) = Board::default().add_area("X", 12, 1).unwrap_or_else(|e| unreachable!("{e}"));
    let (b, _) =
        b.add_widget(&area, "voice-room", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
    state.doc.set(b);
    frame(&ui);
    assert!(!texts_like(&ui, "voice-room").is_empty());
}
