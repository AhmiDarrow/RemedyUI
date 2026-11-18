//! Views end to end: build elements, run frames, composite with the CPU renderer, and route
//! input.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Insets, Point, Size, Vec2};
use remedy_ui_host::ImeEvent;
use remedy_ui_input::{
    Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerKind, PointerPhase, ScrollPhysics,
};
use remedy_ui_layout::Length;
use remedy_ui_reactive::Signal;
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{FieldKind, Request, Ui};

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
const BLUE: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

fn fonts() -> FontCollection {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    f
}

fn ui() -> Ui {
    let mut sheet = StyleSheet::new();
    sheet.rule(
        "window",
        None,
        States::NONE,
        StylePatch { background: Some(Color::WHITE), ..StylePatch::default() },
    );
    sheet.rule("button", None, States::NONE, StylePatch { background: Some(RED), ..StylePatch::default() });
    sheet.rule("button", None, States::HOVER, StylePatch { background: Some(BLUE), ..StylePatch::default() });
    Ui::new(Theme::dark_forest(), sheet, fonts())
}

struct Screen {
    ui: Ui,
    comp: Compositor,
    cpu: CpuCompositor,
    t: u64,
    last: Option<Frame>,
}

impl Screen {
    fn new(ui: Ui) -> Self {
        Screen {
            ui,
            comp: Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED),
            cpu: CpuCompositor::new(),
            t: 0,
            last: None,
        }
    }

    /// Runs a frame; returns whether the view produced new layers.
    fn frame(&mut self) -> bool {
        let f = self.ui.frame(Size::new(200.0, 120.0), 1.0);
        let changed = f.layers.is_some();
        if let Some(l) = f.layers {
            self.comp.commit(l);
        }
        self.t += 16;
        let plan = self.comp.frame(Duration::from_millis(self.t));
        if let Ok(frame) = self.cpu.present(&plan, 200, 120, Color::BLACK) {
            self.last = Some(frame);
        }
        changed
    }

    fn px(&self, x: u32, y: u32) -> [u8; 4] {
        self.last.as_ref().and_then(|f| f.pixel(x, y)).unwrap_or([0; 4])
    }
}

fn pointer(phase: PointerPhase, x: f32, y: f32, down: bool) -> PointerEvent {
    PointerEvent {
        id: 0,
        kind: PointerKind::Mouse,
        phase,
        position: Point::new(x, y),
        buttons: if down { Buttons::PRIMARY } else { Buttons::default() },
        pressure: 0.5,
        time: Duration::ZERO,
        coalesced: Vec::new(),
        modifiers: remedy_ui_input::Modifiers::NONE,
    }
}

fn key(k: Key, text: Option<&str>, m: Modifiers) -> KeyEvent {
    KeyEvent {
        key: k,
        physical: String::new(),
        modifiers: m,
        pressed: true,
        repeat: false,
        composing: false,
        text: text.map(str::to_owned),
    }
}

fn typed(c: &str) -> KeyEvent {
    key(Key::Character(c.to_lowercase()), Some(c), Modifiers::NONE)
}

fn ctrl(c: &str) -> KeyEvent {
    key(Key::Character(c.into()), None, Modifiers { ctrl: true, ..Modifiers::NONE })
}

fn button(ui: &Ui, clicks: Rc<Cell<u32>>) -> remedy_ui_view::El {
    ui.el("button")
        .layout(|l| {
            l.width = Length::Px(80.0);
            l.height = Length::Px(30.0);
        })
        .on_click(move || clicks.set(clicks.get() + 1))
}

#[test]
fn elements_paint_and_unchanged_frames_produce_nothing() {
    let ui = ui();
    let clicks = Rc::new(Cell::new(0));
    ui.root().padding(Insets::all(10.0)).child(button(&ui, clicks));
    let mut s = Screen::new(ui);
    assert!(s.frame());
    assert_eq!(s.px(5, 5), [255, 255, 255, 255], "the window background");
    assert_eq!(s.px(20, 20), [255, 0, 0, 255], "the button, inside the padding");
    assert_eq!(s.px(95, 20), [255, 255, 255, 255], "past the button's width");
    assert!(!s.frame(), "nothing changed, nothing produced");
}

#[test]
fn hover_restyles_and_clicks_come_from_pointer_and_keyboard() {
    let ui = ui();
    let clicks = Rc::new(Cell::new(0));
    ui.root().child(button(&ui, clicks.clone()));
    let mut s = Screen::new(ui.clone());
    s.frame();
    ui.pointer(&pointer(PointerPhase::Move, 10.0, 10.0, false));
    assert!(s.frame());
    assert_eq!(s.px(10, 10), [0, 0, 255, 255], "hovered");
    ui.pointer(&pointer(PointerPhase::Down, 10.0, 10.0, true));
    ui.pointer(&pointer(PointerPhase::Up, 10.0, 10.0, false));
    assert_eq!(clicks.get(), 1);
    // Press inside, release outside: no click.
    ui.pointer(&pointer(PointerPhase::Down, 10.0, 10.0, true));
    ui.pointer(&pointer(PointerPhase::Up, 150.0, 100.0, false));
    assert_eq!(clicks.get(), 1);
    ui.pointer(&pointer(PointerPhase::Move, 150.0, 100.0, false));
    s.frame();
    assert_eq!(s.px(10, 10), [255, 0, 0, 255], "hover left");
    // Keyboard: Tab to the button, Enter clicks it.
    assert!(ui.key(&key(Key::Tab, None, Modifiers::NONE)));
    assert!(ui.key(&key(Key::Enter, None, Modifiers::NONE)));
    assert_eq!(clicks.get(), 2);
}

#[test]
fn reactive_text_updates_only_what_reads_it() {
    let ui = ui();
    let label = Signal::new(String::from("One"));
    let l2 = label;
    let text = ui.el("label").text_dyn(move || l2.get());
    ui.root().child(text.clone());
    let mut s = Screen::new(ui);
    s.frame();
    let ink = |s: &Screen| {
        (0..60)
            .flat_map(|x| (0..20).map(move |y| (x, y)))
            .filter(|&(x, y)| s.px(x, y) != [255, 255, 255, 255])
            .count()
    };
    let before = ink(&s);
    assert!(before > 0, "text draws");
    assert!(!s.frame());
    label.set(String::from("One two three"));
    assert!(s.frame(), "the binding changed the text");
    assert!(ink(&s) > before);
    text.remove();
    s.frame();
    label.set(String::from("gone"));
    assert!(!s.frame(), "a removed element's bindings are disposed");
}

#[test]
fn text_fields_edit_through_keys_clipboard_and_input_methods() {
    let ui = ui();
    let changes = Rc::new(Cell::new(0));
    let c2 = changes.clone();
    let field = ui.text_field("text_input", FieldKind::Single).on_change(move || c2.set(c2.get() + 1));
    ui.root().child(field.el().layout(|l| l.width = Length::Px(150.0)));
    let mut s = Screen::new(ui.clone());
    s.frame();
    let b = field.el().bounds().unwrap_or_default();
    ui.pointer(&pointer(PointerPhase::Down, b.min.x + 2.0, b.min.y + 2.0, true));
    ui.pointer(&pointer(PointerPhase::Up, b.min.x + 2.0, b.min.y + 2.0, false));
    assert!(field.el().is_focused());
    assert!(ui.take_requests().iter().any(|r| matches!(r, Request::TextInput(Some(st)) if !st.private)));
    for c in ["H", "i", "!"] {
        ui.key(&typed(c));
    }
    assert_eq!(field.value(), "Hi!", "typed text keeps its case");
    ui.key(&key(Key::Backspace, None, Modifiers::NONE));
    assert_eq!(field.value(), "Hi");
    ui.key(&ctrl("a"));
    ui.key(&ctrl("c"));
    assert!(ui.take_requests().contains(&Request::Copy { text: "Hi".into(), sensitive: false }));
    ui.key(&ctrl("v"));
    assert!(ui.take_requests().contains(&Request::Paste));
    ui.paste("Hello");
    assert_eq!(field.value(), "Hello", "the paste replaced the selection");
    ui.ime(&ImeEvent::Preedit { text: "にほ".into(), cursor: Some(6..6) });
    assert_eq!(field.value(), "Hello", "a composition is not text yet");
    ui.ime(&ImeEvent::Commit("日本".into()));
    assert_eq!(field.value(), "Hello日本");
    ui.key(&ctrl("z"));
    assert_ne!(field.value(), "Hello日本", "undo");
    assert!(changes.get() >= 5);
    assert!(s.frame());
}

#[test]
fn secret_fields_mask_never_copy_and_skip_input_methods() {
    let ui = ui();
    let submitted = Rc::new(Cell::new(false));
    let s2 = submitted.clone();
    let field = ui.text_field("secret_input", FieldKind::Secret).on_submit(move || s2.set(true));
    ui.root().child(field.el().layout(|l| l.width = Length::Px(150.0)));
    let mut s = Screen::new(ui.clone());
    s.frame();
    field.el().focus();
    assert!(
        ui.take_requests()
            .iter()
            .any(|r| matches!(r, Request::TextInput(Some(st)) if st.private && st.surrounding.is_empty()))
    );
    for c in ["p", "@", "S", "s"] {
        ui.key(&typed(c));
    }
    assert_eq!(field.len(), 4);
    assert_eq!(field.value(), "", "a secret's text never comes out as a plain string");
    ui.key(&ctrl("a"));
    ui.key(&ctrl("c"));
    assert!(
        !ui.take_requests().iter().any(|r| matches!(r, Request::Copy { .. })),
        "secrets are never copied"
    );
    assert!(!ui.ime(&ImeEvent::Commit("x".into())), "no input method edits a secret");
    ui.key(&key(Key::Enter, None, Modifiers::NONE));
    assert!(submitted.get());
    assert_eq!(field.take_secret().expose(), "p@Ss");
    assert!(field.is_empty());
    s.frame();
}

#[test]
fn scroll_containers_become_compositor_layers_and_hit_testing_follows_offsets() {
    let ui = ui();
    let clicks = Rc::new(Cell::new(0));
    let list = ui.el("list").scroll().layout(|l| {
        l.height = Length::Px(100.0);
        l.display = remedy_ui_layout::Display::Flex;
        l.axis = remedy_ui_layout::Axis::Column;
    });
    let list = list.child(ui.el("spacer").layout(|l| l.height = Length::Px(150.0)));
    let list = list.child(button(&ui, clicks.clone()));
    ui.root().child(list.clone());
    let mut s = Screen::new(ui.clone());
    let f = ui.frame(Size::new(200.0, 120.0), 1.0);
    assert_eq!(f.scrollers.len(), 1, "one scroll layer");
    let Some(&(layer, el)) = f.scrollers.first() else { unreachable!("one scroller") };
    assert_eq!(el, list.id());
    if let Some(l) = f.layers {
        s.comp.commit(l);
    }
    // The button sits at y 150..180, below the 100 px viewport; scrolling as far as it goes
    // (180 - 100 = 80) brings it to 70..100.
    s.comp.scroll_by(layer, Vec2::new(0.0, 500.0));
    let offset = s.comp.scroll_offset(layer).unwrap_or(Vec2::ZERO);
    ui.set_scroll_offset(list.id(), offset);
    s.frame();
    assert_eq!(offset, Vec2::new(0.0, 80.0));
    assert_eq!(s.px(10, 85), [255, 0, 0, 255], "the button shows at 150 - 80");
    assert_eq!(s.px(10, 60), [255, 255, 255, 255]);
    ui.pointer(&pointer(PointerPhase::Down, 10.0, 85.0, true));
    ui.pointer(&pointer(PointerPhase::Up, 10.0, 85.0, false));
    assert_eq!(clicks.get(), 1, "the click lands where the button is shown");
}

#[test]
fn semantics_describe_the_window_without_exposing_secrets() {
    let ui = ui();
    let clicks = Rc::new(Cell::new(0));
    let field = ui.text_field("secret_input", FieldKind::Secret);
    ui.root().child(button(&ui, clicks)).child(ui.el("label").text("Name")).child(field.el());
    ui.connect_accessibility();
    let mut s = Screen::new(ui.clone());
    s.frame();
    field.el().focus();
    ui.key(&typed("z"));
    s.frame();
    let update = ui.take_accessibility_update();
    assert!(update.is_some(), "a connected client gets the tree");
    let dump = format!("{update:?}");
    assert!(dump.contains("Name") && dump.contains("Click") && dump.contains("PasswordInput"), "{dump}");
    assert!(!dump.contains("\"z\""), "no secret text in the tree");
}

#[test]
fn overlays_draw_above_content_trap_focus_and_timers_fire() {
    let ui = ui();
    let clicks = Rc::new(Cell::new(0));
    ui.root().child(button(&ui, clicks.clone()));
    let mut s = Screen::new(ui.clone());
    s.frame();
    // A dialog over the button, with its own button, keeps Tab inside itself.
    let inner = Rc::new(Cell::new(0));
    let dialog = ui.overlay(
        ui.el("dialog")
            .style(StylePatch { background: Some(BLUE), ..StylePatch::default() })
            .layout(|l| {
                l.position = remedy_ui_layout::Position::Absolute;
                l.inset = Insets::ZERO;
            })
            .focus_scope()
            .child(button(&ui, inner.clone()).layout(|l| l.margin = Insets::all(50.0))),
    );
    s.frame();
    assert!(dialog.focus_first(), "opening a modal focuses its first control");
    assert_eq!(s.px(10, 10), [0, 0, 255, 255], "the overlay covers the content");
    ui.pointer(&pointer(PointerPhase::Down, 10.0, 10.0, true));
    ui.pointer(&pointer(PointerPhase::Up, 10.0, 10.0, false));
    assert_eq!(clicks.get(), 0, "content under an overlay cannot be clicked");
    assert!(ui.tree().focused().is_some(), "clicking the modal's background keeps focus in it");
    for _ in 0..3 {
        ui.key(&key(Key::Enter, None, Modifiers::NONE));
        ui.key(&key(Key::Tab, None, Modifiers::NONE));
    }
    assert_eq!((clicks.get(), inner.get()), (0, 3), "Tab stays in the dialog");
    // Keys reach handlers on focused ancestors.
    let escaped = Rc::new(Cell::new(false));
    let e2 = escaped.clone();
    let d2 = dialog.clone();
    let _ = dialog.on_key(move |e| {
        if e.key == Key::Escape {
            e2.set(true);
            d2.remove();
            return true;
        }
        false
    });
    assert!(ui.key(&key(Key::Escape, None, Modifiers::NONE)));
    assert!(escaped.get());
    s.frame();
    assert_eq!(s.px(10, 10), [255, 0, 0, 255], "the dialog is gone");
    // Timers go out as requests and run when the runner fires them.
    let fired = Rc::new(Cell::new(false));
    let f2 = fired.clone();
    let id = ui.after(Duration::from_millis(30), move || f2.set(true));
    assert!(ui.take_requests().contains(&Request::Timer { id, delay: Duration::from_millis(30) }));
    ui.fire_timer(id);
    assert!(fired.get());
    let gone = ui.after(Duration::from_millis(30), || {});
    ui.cancel_timer(gone);
    assert!(ui.take_requests().contains(&Request::CancelTimer(gone)));
}

#[test]
fn painters_repaint_when_the_signals_they_read_change() {
    let ui = ui();
    let color = Signal::new(Color::BLACK);
    let other = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let r2 = runs.clone();
    let el = ui.el("box").width(20.0).height(20.0).paint(move |cx| {
        r2.set(r2.get() + 1);
        let _ = color.get();
        let _ = cx.size;
    });
    ui.root().child(el);
    let size = Size::new(100.0, 100.0);
    let _ = ui.frame(size, 1.0);
    assert_eq!(runs.get(), 1);
    let _ = ui.take_requests();
    let _ = ui.frame(size, 1.0);
    assert_eq!(runs.get(), 1, "nothing changed: the chunk is reused");
    other.set(1);
    let _ = ui.frame(size, 1.0);
    assert_eq!(runs.get(), 1, "an unread signal does not repaint");
    color.set(Color::WHITE);
    assert!(ui.take_requests().iter().any(|r| matches!(r, Request::Frame)), "a frame was requested");
    let _ = ui.frame(size, 1.0);
    assert_eq!(runs.get(), 2, "the read signal repainted it");
}

#[test]
fn elements_report_the_signals_their_bindings_and_painters_read() {
    let ui = ui();
    let name = Signal::new(String::from("Ada"));
    let tint = Signal::new(Color::BLACK);
    let unread = Signal::new(0u32);
    let el = ui.el("label").text_dyn(move || name.get()).paint(move |_| {
        let _ = tint.get();
    });
    let _ = unread;
    ui.root().child(el.clone());
    let _ = ui.frame(Size::new(100.0, 100.0), 1.0);
    let reads = el.info().map(|i| i.reads).unwrap_or_default();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert!(reads.iter().all(|s| s.created_at.file().ends_with("view.rs")), "{reads:?}");
    assert!(reads.iter().all(|s| s.kind == remedy_ui_reactive::SourceKind::Signal));
    assert!(reads.iter().all(|s| s.to_string().starts_with("signal at ")));
}

#[test]
fn pseudo_locales_lengthen_app_text_but_not_typed_text() {
    let ui = ui();
    let label = ui.el("label").text(remedy_ui_view::RichText::plain("Save changes")).nowrap();
    let field = ui.text_field("text_input", FieldKind::Single);
    field.set_value("typed by a person");
    ui.root().child(ui.el("box").row().child(label.clone())).child(field.el());
    let size = Size::new(600.0, 200.0);
    let _ = ui.frame(size, 1.0);
    let plain = label.bounds().map(|b| b.width()).unwrap_or(0.0);
    ui.set_locale("en-XA");
    let _ = ui.frame(size, 1.0);
    let long = label.bounds().map(|b| b.width()).unwrap_or(0.0);
    assert!(long > plain * 1.3, "{plain} -> {long}");
    assert_eq!(field.value(), "typed by a person", "what people type is never rewritten");
    ui.set_locale("en-US");
    let _ = ui.frame(size, 1.0);
    let back = label.bounds().map(|b| b.width()).unwrap_or(0.0);
    assert!((back - plain).abs() < 0.5, "{plain} vs {back}");
}

#[test]
fn data_text_is_left_alone_by_pseudo_locales() {
    let ui = ui();
    let name = ui.el("label").text(remedy_ui_view::RichText::plain("Ada Lovelace")).nowrap().data_text();
    let app = ui.el("label").text(remedy_ui_view::RichText::plain("Ada Lovelace")).nowrap();
    ui.root().child(ui.el("box").row().child(name.clone())).child(ui.el("box").row().child(app.clone()));
    ui.set_locale("en-XA");
    let _ = ui.frame(Size::new(600.0, 200.0), 1.0);
    let w = |e: &remedy_ui_view::El| e.bounds().map(|b| b.width()).unwrap_or(0.0);
    assert!(w(&app) > w(&name) * 1.3, "{} vs {}", w(&app), w(&name));
}

#[test]
fn keyboard_focus_draws_the_theme_focus_ring_and_a_pointer_press_does_not() {
    let ui = ui();
    let target = ui.el("button").width(40.0).height(30.0).focusable(true);
    ui.root().child(ui.el("pad").padding(Insets::all(10.0)).child(target.clone()));
    let mut s = Screen::new(ui.clone());
    s.frame();
    let ring = Theme::dark_forest().derive().focus;
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    let want = [byte(ring.r), byte(ring.g), byte(ring.b)];
    let is_ring = |p: [u8; 4]| want.iter().zip(p).all(|(w, g)| (w - i32::from(g)).abs() <= 2);
    // The ring sits 2 px outside the box, 2 px wide: x 6..8 to the left of x 10.
    assert!(!is_ring(s.px(7, 25)), "no ring before focus");
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    s.frame();
    assert!(target.is_focused());
    assert!(is_ring(s.px(7, 25)), "keyboard focus shows the ring: {:?}", s.px(7, 25));
    assert!(is_ring(s.px(30, 7)), "on every side");
    // Focus that came from a pointer press does not draw it.
    target.blur();
    s.frame();
    ui.pointer(&pointer(PointerPhase::Down, 20.0, 20.0, true));
    ui.pointer(&pointer(PointerPhase::Up, 20.0, 20.0, false));
    s.frame();
    assert!(target.is_focused());
    assert!(!is_ring(s.px(7, 25)), "pointer focus shows no ring");
    // A style that shows focus another way turns the ring off.
    let quiet = ui
        .el("button")
        .width(40.0)
        .height(30.0)
        .focusable(true)
        .style(StylePatch { outline_width: Some(0.0), ..StylePatch::default() });
    ui.root().clear();
    ui.root().child(ui.el("pad").padding(Insets::all(10.0)).child(quiet.clone()));
    s.frame();
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    s.frame();
    assert!(quiet.is_focused());
    assert!(!is_ring(s.px(7, 25)));
}

#[test]
fn hidden_elements_take_no_space_and_no_keyboard_focus() {
    let ui = ui();
    let first = ui.el("button").width(40.0).height(30.0).focusable(true);
    let middle = ui.el("button").width(40.0).height(30.0).focusable(true);
    let last = ui.el("button").width(40.0).height(30.0).focusable(true);
    let col = ui.el("col").column().child(first.clone()).child(middle.clone()).child(last.clone());
    ui.root().child(col);
    let _ = ui.frame(Size::new(200.0, 120.0), 1.0);
    let y = |e: &remedy_ui_view::El| e.bounds().map(|b| b.min.y).unwrap_or(-1.0);
    assert_eq!(y(&last), 60.0);
    // Hidden: the next element moves up into its place, and Tab passes over it.
    middle.focus();
    let middle = middle.hidden(true);
    let _ = ui.frame(Size::new(200.0, 120.0), 1.0);
    assert_eq!(y(&last), 30.0);
    assert!(!middle.is_focused(), "hiding the focused element takes focus away");
    first.focus();
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    assert!(last.is_focused(), "Tab skips the hidden element");
    middle.focus();
    assert!(!middle.is_focused(), "a hidden element cannot be focused");
    // Disabled elements stay in place but are skipped too.
    let middle = middle.hidden(false).disabled(true);
    let _ = ui.frame(Size::new(200.0, 120.0), 1.0);
    assert_eq!(y(&last), 60.0);
    first.focus();
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    assert!(last.is_focused());
    let _ = middle;
}

fn mv(ui: &Ui, phase: PointerPhase, x: f32, y: f32, down: bool) {
    ui.pointer(&pointer(phase, x, y, down));
}

#[test]
fn elements_drag_onto_targets_inside_the_app() {
    use remedy_ui_host::{DragData, DropEffect};
    use remedy_ui_view::Drag;
    let ui = ui();
    let ended = Rc::new(Cell::new(None));
    let clicked = Rc::new(Cell::new(0));
    let got: Rc<std::cell::RefCell<Vec<(String, Point)>>> = Rc::default();
    let (e2, c2, g2) = (ended.clone(), clicked.clone(), got.clone());
    let source =
        ui.el("button").width(40.0).height(30.0).on_click(move || c2.set(c2.get() + 1)).draggable(
            || Some(Drag::new(DragData::text("hello")).with_local(7u32)),
            move |e| e2.set(Some(e)),
        );
    let target = ui.el("box").width(80.0).height(60.0).on_drop(
        |d, _| if d.local::<u32>() == Some(&7) { DropEffect::Move } else { DropEffect::None },
        move |d, at, _| g2.borrow_mut().push((d.data.text.clone().unwrap_or_default(), at)),
    );
    let refuses = ui.el("box").width(40.0).height(20.0).on_drop(|_, _| DropEffect::None, |_, _, _| {});
    ui.root().child(ui.el("col").column().child(source.clone()).child(target.clone()).child(refuses.clone()));
    let _ = ui.frame(Size::new(200.0, 120.0), 1.0);
    // Press, move a little (still a press), then far: a drag.
    mv(&ui, PointerPhase::Down, 10.0, 10.0, true);
    mv(&ui, PointerPhase::Move, 12.0, 11.0, true);
    assert!(ui.dragging().is_none(), "a small move is not a drag");
    mv(&ui, PointerPhase::Move, 20.0, 60.0, true);
    assert!(ui.dragging().is_some());
    assert!(source.info().is_some_and(|i| i.states.contains(States::DRAGGING)));
    assert!(target.info().is_some_and(|i| i.states.contains(States::DROP_TARGET)));
    // Over the refusing target nothing is highlighted.
    mv(&ui, PointerPhase::Move, 10.0, 100.0, true);
    assert!(!refuses.info().is_some_and(|i| i.states.contains(States::DROP_TARGET)));
    assert!(!target.info().is_some_and(|i| i.states.contains(States::DROP_TARGET)));
    // Back over the target and let go: it gets the drop, at its own coordinates.
    mv(&ui, PointerPhase::Move, 25.0, 50.0, true);
    mv(&ui, PointerPhase::Up, 25.0, 50.0, false);
    assert_eq!(*got.borrow(), vec![("hello".to_owned(), Point::new(25.0, 20.0))]);
    assert_eq!(ended.get(), Some(DropEffect::Move));
    assert_eq!(clicked.get(), 0, "a drag is not a click");
    assert!(!source.info().is_some_and(|i| i.states.contains(States::DRAGGING)));
    assert!(!target.info().is_some_and(|i| i.states.contains(States::DROP_TARGET)));
    // Escape cancels a drag.
    mv(&ui, PointerPhase::Down, 10.0, 10.0, true);
    mv(&ui, PointerPhase::Move, 20.0, 50.0, true);
    assert!(ui.key(&key(Key::Escape, None, Modifiers::NONE)));
    assert_eq!(ended.get(), Some(DropEffect::None));
    mv(&ui, PointerPhase::Up, 20.0, 50.0, false);
    assert_eq!(got.borrow().len(), 1);
}

#[test]
fn drags_leave_for_other_apps_and_come_in_from_them() {
    use remedy_ui_host::{DragData, DragEvent, DropEffect};
    use remedy_ui_view::Drag;
    let ui = ui();
    let ended = Rc::new(Cell::new(None));
    let e2 = ended.clone();
    let source = ui.el("button").width(40.0).height(30.0).draggable(
        || Some(Drag::new(DragData::files(vec!["C:/notes.txt".into()]))),
        move |e| e2.set(Some(e)),
    );
    let files: Rc<std::cell::RefCell<Vec<std::path::PathBuf>>> = Rc::default();
    let f2 = files.clone();
    let target = ui.el("box").width(200.0).height(80.0).on_drop(
        |d, _| if d.data.files.is_empty() { DropEffect::None } else { DropEffect::Copy },
        move |d, _, _| f2.borrow_mut().extend(d.data.files.iter().cloned()),
    );
    ui.root().child(ui.el("col").column().child(source.clone()).child(target.clone()));
    let _ = ui.frame(Size::new(200.0, 120.0), 1.0);
    let _ = ui.take_requests();
    // Out of the window: the platform takes over.
    mv(&ui, PointerPhase::Down, 10.0, 10.0, true);
    mv(&ui, PointerPhase::Move, 100.0, 10.0, true);
    mv(&ui, PointerPhase::Move, 260.0, 10.0, true);
    let asked = ui.take_requests().into_iter().find_map(|r| match r {
        Request::StartDrag { data, allowed } => Some((data, allowed)),
        _ => None,
    });
    assert_eq!(
        asked.map(|(d, a)| (d.files, a)),
        Some((vec!["C:/notes.txt".into()], vec![DropEffect::Copy, DropEffect::Move]))
    );
    ui.drag_finished(DropEffect::Copy);
    assert_eq!(ended.get(), Some(DropEffect::Copy));
    assert!(ui.dragging().is_none());
    // In from another app: files over the target, then dropped.
    let data = DragData::files(vec!["D:/photo.png".into()]);
    assert_eq!(
        ui.drag_event(&DragEvent::Enter { position: Some(Point::new(50.0, 50.0)), data: data.clone() }),
        DropEffect::Copy
    );
    assert!(target.info().is_some_and(|i| i.states.contains(States::DROP_TARGET)));
    assert_eq!(
        ui.drag_event(&DragEvent::Over { position: Point::new(50.0, 5.0) }),
        DropEffect::None,
        "not over the target"
    );
    assert_eq!(
        ui.drag_event(&DragEvent::Drop { position: Some(Point::new(60.0, 60.0)), data }),
        DropEffect::Copy
    );
    assert_eq!(*files.borrow(), vec![std::path::PathBuf::from("D:/photo.png")]);
    assert!(ui.dragging().is_none());
    // A platform that does not say where: the window's outermost target takes it.
    assert_eq!(
        ui.drag_event(&DragEvent::Drop { position: None, data: DragData::files(vec!["E:/a.txt".into()]) }),
        DropEffect::Copy
    );
    assert_eq!(files.borrow().len(), 2);
    // Text alone is refused.
    assert_eq!(
        ui.drag_event(&DragEvent::Enter {
            position: Some(Point::new(50.0, 50.0)),
            data: DragData::text("x")
        }),
        DropEffect::None
    );
    assert_eq!(ui.drag_event(&DragEvent::Leave), DropEffect::None);
    let _ = source;
}

#[test]
fn scroll_containers_inside_offset_elements_show_where_they_are() {
    let ui = ui();
    let list = ui.el("list").scroll().layout(|l| {
        l.height = Length::Px(60.0);
        l.display = remedy_ui_layout::Display::Flex;
        l.axis = remedy_ui_layout::Axis::Column;
    });
    let list = list.child(button(&ui, Rc::new(Cell::new(0))));
    // Two levels of offset around the scroll container.
    let inner =
        ui.el("pad").padding(Insets { top: 30.0, start: 20.0, bottom: 0.0, end: 0.0 }).child(list.clone());
    ui.root()
        .child(ui.el("pad").padding(Insets { top: 10.0, start: 10.0, bottom: 0.0, end: 0.0 }).child(inner));
    let mut s = Screen::new(ui.clone());
    s.frame();
    // The button (red, 80x30) sits at x 30, y 40, inside the scroll container.
    assert_eq!(s.px(35, 45), [255, 0, 0, 255], "the scroll content is where its container is");
    assert_eq!(s.px(5, 5), [255, 255, 255, 255], "not at the window's corner");
    assert_eq!(list.bounds().map(|b| (b.min.x, b.min.y)), Some((30.0, 40.0)));
}

#[test]
fn what_follows_a_scroll_container_paints_above_what_came_before_it() {
    let ui = ui();
    // An opaque column holding a scroll container, then a red button after it.
    let list = ui.el("list").scroll().layout(|l| {
        l.height = Length::Px(40.0);
        l.width = Length::Px(60.0);
    });
    let after = button(&ui, Rc::new(Cell::new(0)));
    let col = ui
        .el("col")
        .column()
        .style(StylePatch { background: Some(BLUE), ..StylePatch::default() })
        .layout(|l| {
            l.width = Length::Px(200.0);
            l.height = Length::Px(120.0);
        })
        .child(list)
        .child(after);
    ui.root().child(col);
    let mut s = Screen::new(ui.clone());
    s.frame();
    // The button is at y 40..70: drawn over the column's blue background.
    assert_eq!(s.px(10, 50), [255, 0, 0, 255], "the button after the scroll container shows");
    assert_eq!(s.px(150, 100), [0, 0, 255, 255]);
}

#[test]
fn hover_hints_hear_drags_come_and_go_even_when_refused() {
    use remedy_ui_host::{DragData, DragEvent, DropEffect};
    let ui = ui();
    let heard: Rc<std::cell::RefCell<Vec<Option<usize>>>> = Rc::default();
    let h2 = heard.clone();
    // A read-only folder: it takes nothing, but says why while something is over it.
    let target = ui
        .el("box")
        .width(200.0)
        .height(80.0)
        .on_drop(|_, _| DropEffect::None, |_, _, _| {})
        .on_drag_hover(move |d| h2.borrow_mut().push(d.map(|d| d.data.files.len())))
        .child(ui.el("box").width(50.0).height(20.0));
    ui.root().child(ui.el("col").column().child(ui.el("box").width(200.0).height(30.0)).child(target));
    let _ = ui.frame(Size::new(200.0, 120.0), 1.0);
    let data = DragData::files(vec!["D:/a.png".into(), "D:/b.png".into()]);
    assert_eq!(
        ui.drag_event(&DragEvent::Enter { position: Some(Point::new(10.0, 10.0)), data }),
        DropEffect::None
    );
    assert!(heard.borrow().is_empty(), "not over it yet");
    let _ = ui.drag_event(&DragEvent::Over { position: Point::new(20.0, 40.0) });
    // Over its child: still over it, nothing new.
    let _ = ui.drag_event(&DragEvent::Over { position: Point::new(30.0, 45.0) });
    assert_eq!(*heard.borrow(), vec![Some(2)]);
    let _ = ui.drag_event(&DragEvent::Over { position: Point::new(20.0, 5.0) });
    assert_eq!(*heard.borrow(), vec![Some(2), None], "left it");
    let _ = ui.drag_event(&DragEvent::Over { position: Point::new(20.0, 60.0) });
    let _ = ui.drag_event(&DragEvent::Leave);
    assert_eq!(*heard.borrow(), vec![Some(2), None, Some(2), None], "the drag going away ends the hint");
}
