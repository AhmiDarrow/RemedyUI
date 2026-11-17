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
