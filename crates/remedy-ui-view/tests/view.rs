//! Views end to end: build elements, run frames, composite with the CPU renderer, and route
//! input.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Insets, Point, Size, Vec2};
use remedy_ui_host::ImeEvent;
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase, ScrollPhysics};
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
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    sheet.rule("button", None, States::NONE, StylePatch::default().background(RED));
    sheet.rule("button", None, States::HOVER, StylePatch::default().background(BLUE));
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
    PointerEvent::new(phase, Point::new(x, y))
        .with_buttons(if down { Buttons::PRIMARY } else { Buttons::default() })
        .with_modifiers(remedy_ui_input::Modifiers::NONE)
}

fn key(k: Key, text: Option<&str>, m: Modifiers) -> KeyEvent {
    KeyEvent::press(k).with_modifiers(m).with_text(text.map(str::to_owned))
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
    let text = ui.el("label").text(move || l2.get());
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
    assert!(ui.take_requests().contains(&Request::Copy { text: "Hi".into(), sensitive: false, html: None }));
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
fn fields_that_copy_as_secret_send_copies_and_cuts_to_the_secure_clipboard() {
    let ui = ui();
    let keep = std::time::Duration::from_secs(30);
    let copies = Rc::new(Cell::new(0));
    let c2 = copies.clone();
    let note = ui
        .text_field("text_area", FieldKind::Multi)
        .copy_as_secret(keep)
        .on_copy(move || c2.set(c2.get() + 1));
    let rich = ui.text_field("rich_input", FieldKind::Rich).copy_as_secret(keep);
    let plain = ui.text_field("text_input", FieldKind::Single);
    assert_eq!((note.secret_copy(), plain.secret_copy()), (Some(keep), None));
    ui.root().child(note.el()).child(rich.el()).child(plain.el());
    let mut s = Screen::new(ui.clone());
    s.frame();
    note.set_value("token: abc123");
    note.el().focus();
    let _ = ui.take_requests();

    // Nothing selected: nothing copied, and nobody told.
    ui.key(&ctrl("c"));
    assert!(
        !ui.take_requests().iter().any(|r| matches!(r, Request::Copy { .. } | Request::CopySecret { .. }))
    );
    assert_eq!(copies.get(), 0);

    ui.key(&ctrl("a"));
    ui.key(&ctrl("c"));
    let r = ui.take_requests();
    assert!(r.contains(&Request::CopySecret { text: "token: abc123".into(), keep }), "{r:?}");
    assert!(!r.iter().any(|r| matches!(r, Request::Copy { .. })), "never an ordinary copy");
    assert_eq!(copies.get(), 1);

    ui.key(&ctrl("x"));
    assert!(ui.take_requests().contains(&Request::CopySecret { text: "token: abc123".into(), keep }));
    assert_eq!(note.value(), "", "cut");
    assert_eq!(copies.get(), 2);

    // A rich field's secret copy is plain text, with no HTML to outlive the clear.
    rich.set_value("**bold** key");
    rich.el().focus();
    ui.key(&ctrl("a"));
    ui.key(&ctrl("c"));
    let r = ui.take_requests();
    assert!(r.iter().any(|r| matches!(r, Request::CopySecret { text, .. } if text.contains("key"))), "{r:?}");
    assert!(!r.iter().any(|r| matches!(r, Request::Copy { .. })));

    // Other fields copy as before.
    plain.set_value("hello");
    plain.el().focus();
    ui.key(&ctrl("a"));
    ui.key(&ctrl("c"));
    assert!(ui.take_requests().contains(&Request::Copy {
        text: "hello".into(),
        sensitive: false,
        html: None
    }));
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
            .style(StylePatch::default().background(BLUE))
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
    let el = ui.el("label").text(move || name.get()).paint(move |_| {
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
        .style(StylePatch::default().outline_width(0.0));
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
        .style(StylePatch::default().background(BLUE))
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

#[test]
fn text_people_wrote_selects_across_messages_and_copies() {
    let ui = ui();
    let first = ui.el("label").text(remedy_ui_view::RichText::plain("Dinner at seven")).selectable(true);
    let second = ui.el("label").text(remedy_ui_view::RichText::plain("bring the salad")).selectable(true);
    let plain = ui.el("label").text(remedy_ui_view::RichText::plain("not selectable"));
    let button = ui.el("button").width(80.0).height(24.0).on_click(|| {});
    ui.root().child(
        ui.el("col")
            .column()
            .gap(10.0)
            .child(first.clone())
            .child(second.clone())
            .child(plain)
            .child(button.clone()),
    );
    let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
    let a = first.bounds().unwrap_or_default();
    let b = second.bounds().unwrap_or_default();
    // From the start of the first message to the middle of the second.
    mv(&ui, PointerPhase::Down, a.min.x + 0.5, a.center().y, true);
    // A little way into the second message (the label spans the column; its text is short).
    mv(&ui, PointerPhase::Move, b.min.x + 30.0, b.center().y, true);
    mv(&ui, PointerPhase::Up, b.min.x + 30.0, b.center().y, false);
    let got = ui.selected_text().unwrap_or_default();
    assert!(got.starts_with("Dinner at seven\nbring"), "{got:?}");
    assert!(got.len() < "Dinner at seven\nbring the salad".len(), "only part of the second: {got:?}");
    // Ctrl+C copies it (no field has focus).
    let _ = ui.take_requests();
    let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
    assert!(ui.key(&key(Key::Character("c".into()), None, ctrl)));
    assert!(ui.take_requests().iter().any(|r| matches!(r, Request::Copy { text, .. } if *text == got)));
    // A press on a control clears it.
    let c = button.bounds().unwrap_or_default().center();
    mv(&ui, PointerPhase::Down, c.x, c.y, true);
    mv(&ui, PointerPhase::Up, c.x, c.y, false);
    assert_eq!(ui.selected_text(), None);
    // A double press selects a word.
    let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
    let x = a.min.x + 2.0;
    mv(&ui, PointerPhase::Down, x, a.center().y, true);
    mv(&ui, PointerPhase::Up, x, a.center().y, false);
    mv(&ui, PointerPhase::Down, x, a.center().y, true);
    mv(&ui, PointerPhase::Up, x, a.center().y, false);
    assert_eq!(ui.selected_text().as_deref().map(str::trim), Some("Dinner"));
    // When the message goes, so does the selection.
    first.remove();
    assert_eq!(ui.selected_text(), None);
}

#[test]
fn links_in_text_follow_from_any_line_by_keyboard_and_read_as_links() {
    use remedy_ui_view::{RichText, TextStylePatch};
    let ui = ui();
    let followed = Rc::new(Cell::new(0u64));
    let f = followed.clone();
    let text = RichText::new()
        .text("Read the ")
        .styled("release notes for this version of the app", TextStylePatch::link(7))
        .text(" or ")
        .styled("ask", TextStylePatch::link(9))
        .text(".");
    let para = ui.el("label").width(160.0).text(text).selectable(true).on_link(move |id| f.set(id));
    ui.root().child(ui.el("col").column().padding(Insets::all(10.0)).child(para.clone()));
    let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
    let links = para.link_elements();
    let [notes_link, ask_link] = links.as_slice() else { unreachable!("two links: {}", links.len()) };
    let notes = notes_link.bounds().unwrap_or_default();
    let p = para.bounds().unwrap_or_default();
    assert!(notes.width() > 10.0 && notes.height() > 5.0, "placed over its words: {notes:?}");
    assert!(notes.min.x > p.min.x + 20.0, "after 'Read the ': {notes:?} in {p:?}");
    assert!(p.height() > notes.height() * 1.5, "the link wraps onto another line: {p:?}");
    // A click on its first line follows it.
    let c = notes.center();
    mv(&ui, PointerPhase::Down, c.x, c.y, true);
    mv(&ui, PointerPhase::Up, c.x, c.y, false);
    assert_eq!(followed.get(), 7);
    assert_eq!(ui.selected_text(), None, "following a link selects nothing");
    // A click on its second line follows it too.
    followed.set(0);
    let y2 = notes.max.y + notes.height() / 2.0;
    mv(&ui, PointerPhase::Down, p.min.x + 4.0, y2, true);
    mv(&ui, PointerPhase::Up, p.min.x + 4.0, y2, false);
    assert_eq!(followed.get(), 7);
    // A click on plain words follows nothing.
    followed.set(0);
    mv(&ui, PointerPhase::Down, p.min.x + 2.0, notes.center().y, true);
    mv(&ui, PointerPhase::Up, p.min.x + 2.0, notes.center().y, false);
    assert_eq!(followed.get(), 0);
    // Tab reaches each link in order; Enter follows it.
    ui.root().focus();
    for want in [7, 9] {
        ui.key(&key(Key::Tab, None, Modifiers::NONE));
        ui.key(&key(Key::Enter, None, Modifiers::NONE));
        assert_eq!(followed.get(), want);
    }
    // Screen readers get each link by its words.
    let s = ask_link.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!(s.role(), remedy_ui_a11y::Role::Link);
    assert_eq!(s.name(), Some("ask"));
    // New text, new links.
    let _ = para.clone().text(RichText::new().text("See ").styled("help", TextStylePatch::link(3)));
    let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
    assert_eq!(para.link_elements().len(), 1);
    assert!(notes_link.info().is_none(), "the old link elements went");
}

/// Counts pixels that are clearly blue inside `b` (the decoration color used by the tests).
fn blue_pixels(s: &Screen, b: remedy_ui_core::Rect) -> usize {
    let mut n = 0;
    for y in b.min.y.max(0.0) as u32..b.max.y.max(0.0) as u32 {
        for x in b.min.x.max(0.0) as u32..b.max.x.max(0.0) as u32 {
            let p = s.px(x, y);
            if p[2] > 150 && p[0] < 100 && p[1] < 100 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn field_decorations_draw_under_ranges_and_follow_edits() {
    use remedy_ui_view::{Decoration, DecorationKind};
    let ui = ui();
    let field = ui.text_field("text_input", FieldKind::Single);
    field.set_value("hello wrold again");
    ui.root().child(field.el().layout(|l| l.width = Length::Px(190.0)));
    let mut s = Screen::new(ui.clone());
    s.frame();
    let b = field.el().bounds().unwrap_or_default();
    assert_eq!(blue_pixels(&s, b), 0, "nothing is drawn before decorations are set");

    field.set_decorations(vec![Decoration::squiggle(6..11).colored(BLUE)]);
    s.frame();
    let marked = blue_pixels(&s, b);
    assert!(marked > 8, "a squiggle under the word: {marked}");
    // Under the word's left half only: the first word has no mark.
    let before = remedy_ui_core::Rect::new(b.min.x, b.min.y, b.min.x + 20.0, b.max.y);
    assert_eq!(blue_pixels(&s, before), 0, "the first word is clean");

    // Typing before the word moves the mark with it.
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    ui.key(&key(Key::Home, None, Modifiers::NONE));
    ui.key(&typed("X"));
    assert_eq!(field.decorations(), vec![Decoration::squiggle(7..12).colored(BLUE)]);
    s.frame();
    assert!(blue_pixels(&s, b) > 8);
    // Editing inside the word drops its mark until the next check.
    assert!(field.replace_range(8..9, "Q"));
    assert!(field.decorations().is_empty());
    s.frame();
    assert_eq!(blue_pixels(&s, b), 0);

    // Results worked out for older text are carried onto the current text.
    field.set_decorations_for("hello wrold again", vec![Decoration::squiggle(12..17).colored(BLUE)]);
    assert_eq!(field.decorations().len(), 1, "the later word sits after the edits");
    field.set_decorations(vec![Decoration::squiggle(0..5).of_kind(DecorationKind::Underline).colored(BLUE)]);
    s.frame();
    assert!(blue_pixels(&s, b) > 8, "a straight underline draws too");
}

#[test]
fn field_decorations_are_found_by_point_and_secret_fields_refuse_them() {
    use remedy_ui_view::Decoration;
    let ui = ui();
    let field = ui.text_field("text_input", FieldKind::Single);
    field.set_value("good bad good");
    ui.root().child(field.el().layout(|l| l.width = Length::Px(190.0)));
    let mut s = Screen::new(ui.clone());
    s.frame();
    field.set_decorations(vec![Decoration::squiggle(5..8)]);
    s.frame();
    let b = field.el().bounds().unwrap_or_default();
    let on_bad = (0..190)
        .map(|x| Point::new(b.min.x + x as f32, b.min.y + 6.0))
        .find(|p| field.decoration_at(*p).is_some());
    let at = on_bad.unwrap_or_else(|| unreachable!("some point is over the marked word"));
    let idx = field.index_at(at).unwrap_or(usize::MAX);
    assert!((5..=8).contains(&idx), "the point over the word maps into it: {idx}");
    assert!(field.decoration_at(Point::new(b.min.x + 1.0, b.min.y + 6.0)).is_none());
    assert!(!field.replace_range(5..99, "x"), "a range off the text is refused");

    let secret = ui.text_field("secret_input", FieldKind::Secret);
    secret.set_decorations(vec![Decoration::squiggle(0..1)]);
    assert!(secret.decorations().is_empty(), "secrets are never marked");
    assert!(!secret.replace_range(0..0, "x"));
}

#[test]
fn field_observers_hear_typing_and_set_value_without_replacing_on_change() {
    let ui = ui();
    let changes = Rc::new(Cell::new(0));
    let c2 = changes.clone();
    let heard = Rc::new(Cell::new(0));
    let h2 = heard.clone();
    let h3 = heard.clone();
    let field = ui.text_field("text_input", FieldKind::Single).on_change(move || c2.set(c2.get() + 1));
    field.observe_text(move || h2.set(h2.get() + 1));
    field.observe_text(move || h3.set(h3.get() + 10));
    ui.root().child(field.el().layout(|l| l.width = Length::Px(150.0)));
    let mut s = Screen::new(ui.clone());
    s.frame();
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    ui.key(&typed("a"));
    assert_eq!((changes.get(), heard.get()), (1, 11));
    field.set_value("b");
    assert_eq!((changes.get(), heard.get()), (1, 22), "the app's own set_value is heard, not on_change");
}

fn rich_field(ui: &Ui) -> remedy_ui_view::TextField {
    let f = ui.text_field("text_input", FieldKind::Rich);
    ui.root().child(f.el().width(360.0));
    let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
    f.el().focus();
    f
}

fn type_all(ui: &Ui, s: &str) {
    for c in s.chars() {
        ui.key(&key(Key::Character(c.to_string()), Some(&c.to_string()), Modifiers::NONE));
    }
}

#[test]
fn a_rich_field_formats_with_shortcuts_and_lists() {
    use remedy_ui_text::{Block, Mark};
    let ui = ui();
    let f = rich_field(&ui);
    type_all(&ui, "Plan ");
    ui.key(&ctrl("b"));
    type_all(&ui, "now");
    ui.key(&ctrl("b"));
    let doc = f.doc().unwrap_or_default();
    assert_eq!(doc.text, "Plan now");
    assert_eq!(doc.marks, [(5..8, Mark::Bold)]);
    // Ctrl+Shift+8 makes a bulleted list; Enter continues it; Tab nests.
    let shift_ctrl = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    ui.key(&key(Key::Enter, None, Modifiers::NONE));
    ui.key(&key(Key::Character("8".into()), None, shift_ctrl));
    type_all(&ui, "one");
    ui.key(&key(Key::Enter, None, Modifiers::NONE));
    ui.key(&key(Key::Tab, None, Modifiers::NONE));
    type_all(&ui, "deeper");
    let doc = f.doc().unwrap_or_default();
    assert_eq!(doc.text, "Plan now\none\ndeeper");
    assert_eq!(doc.blocks, [Block::Paragraph, Block::Bullet(0), Block::Bullet(1)]);
    // Screen readers get the plain text.
    let s = f.el().info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!(s.role(), remedy_ui_a11y::Role::MultilineTextInput);
    // Undo takes back the nesting, then the typing.
    ui.key(&ctrl("z"));
    ui.key(&ctrl("z"));
    assert_eq!(f.doc().map(|d| d.blocks), Some(vec![Block::Paragraph, Block::Bullet(0), Block::Bullet(0)]));
}

#[test]
fn a_press_on_a_task_box_ticks_it() {
    use remedy_ui_text::{Block, RichDoc};
    let ui = ui();
    let f = rich_field(&ui);
    f.set_doc(RichDoc {
        text: "milk".into(),
        marks: Vec::new(),
        blocks: vec![Block::Task { done: false, level: 0 }],
    });
    let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
    let b = f.el().bounds().unwrap_or_default();
    // The box is the first thing on the line.
    let at = Point::new(b.min.x + f.el().info().map_or(0.0, |i| i.style.padding.start) + 4.0, b.center().y);
    ui.pointer(&pointer(PointerPhase::Down, at.x, at.y, true));
    ui.pointer(&pointer(PointerPhase::Up, at.x, at.y, false));
    assert_eq!(f.doc().map(|d| d.blocks), Some(vec![Block::Task { done: true, level: 0 }]));
}

#[test]
fn rich_copy_and_paste_carry_formatting_and_keep_mentions_in_the_app() {
    use remedy_ui_text::{Mark, RichDoc};
    let ui = ui();
    let f = rich_field(&ui);
    f.set_doc(RichDoc::plain("hi "));
    f.edit_rich(|e| e.insert_mention("u1", "@Alex", 0));
    f.edit_rich(|e| {
        e.select_all();
    });
    let _ = ui.take_requests();
    ui.key(&ctrl("c"));
    let copied = ui.take_requests().into_iter().find_map(|r| match r {
        Request::Copy { text, html, .. } => Some((text, html)),
        _ => None,
    });
    let (text, html) = copied.unwrap_or_else(|| unreachable!("a copy"));
    assert_eq!(text, "hi @Alex ");
    let html = html.unwrap_or_default();
    assert!(html.contains("@Alex"), "{html}");
    // Pasting our own copy back keeps the mention, which HTML alone never carries.
    f.set_doc(RichDoc::plain(""));
    ui.paste_html(&text, Some(&html));
    assert!(f.doc().is_some_and(|d| d.marks.iter().any(|(_, m)| *m == Mark::Mention("u1".into()))));
    // Another app's HTML comes in as allowed formatting only.
    f.set_doc(RichDoc::plain(""));
    ui.paste_html("x", Some("<b>bold</b><script>bad()</script> <a href=\"javascript:x\">link</a>"));
    let doc = f.doc().unwrap_or_default();
    assert_eq!(doc.text, "bold link");
    assert_eq!(doc.marks, [(0..4, Mark::Bold)]);
    // A plain field takes the text.
    let plain = ui.text_field("text_input", FieldKind::Multi);
    ui.root().child(plain.el());
    plain.el().focus();
    ui.paste_html("just text", Some("<b>just text</b>"));
    assert_eq!(plain.value(), "just text");
}

#[test]
fn a_plain_text_area_can_submit_on_enter() {
    let ui = ui();
    let sent = Rc::new(Cell::new(0u32));
    let s2 = sent.clone();
    let area = ui
        .text_field("text_input", FieldKind::Multi)
        .submit_on_enter(true)
        .on_submit(move || s2.set(s2.get() + 1));
    ui.root().child(area.el());
    area.el().focus();
    let _ = ui.frame(Size::new(200.0, 80.0), 1.0);
    ui.key(&typed("a"));
    ui.key(&key(Key::Enter, None, Modifiers::NONE));
    assert_eq!((sent.get(), area.value()), (1, "a".to_owned()), "Enter sends");
    ui.key(&key(Key::Enter, None, Modifiers::SHIFT));
    ui.key(&typed("b"));
    assert_eq!((sent.get(), area.value()), (1, "a\nb".to_owned()), "Shift+Enter breaks the line");
    // Without it, Enter breaks the line, as before.
    let plain = ui.text_field("text_input", FieldKind::Multi);
    ui.root().child(plain.el());
    plain.el().focus();
    ui.key(&typed("x"));
    ui.key(&key(Key::Enter, None, Modifiers::NONE));
    assert_eq!(plain.value(), "x\n");
}

#[test]
fn elements_hear_when_they_are_seen_scrolled_away_hidden_or_covered() {
    let ui = ui();
    let heard: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let h2 = heard.clone();
    let spacer = ui.el("spacer").layout(|l| l.height = Length::Px(150.0));
    let target = ui
        .el("target")
        .layout(|l| l.height = Length::Px(30.0))
        .on_visibility(move |s| h2.borrow_mut().push(s));
    let list = ui.el("list").scroll().layout(|l| {
        l.height = Length::Px(100.0);
        l.display = remedy_ui_layout::Display::Flex;
        l.axis = remedy_ui_layout::Axis::Column;
    });
    let list = list.child(spacer).child(target.clone());
    ui.root().child(list.clone());
    let size = Size::new(200.0, 120.0);
    let _ = ui.frame(size, 1.0);
    // Below the 100 px viewport: unseen, and told so at once.
    assert_eq!(*heard.borrow(), vec![false]);
    let _ = ui.frame(size, 1.0);
    assert_eq!(heard.borrow().len(), 1, "told only when it changes");
    // Scrolled until it shows.
    ui.set_scroll_offset(list.id(), Vec2::new(0.0, 80.0));
    let _ = ui.frame(size, 1.0);
    assert_eq!(*heard.borrow(), vec![false, true]);
    // A covered or minimized window shows nothing.
    ui.set_occluded(true);
    assert!(ui.take_requests().contains(&Request::Frame));
    let _ = ui.frame(size, 1.0);
    assert_eq!(*heard.borrow(), vec![false, true, false]);
    ui.set_occluded(false);
    let _ = ui.frame(size, 1.0);
    assert_eq!(*heard.borrow(), vec![false, true, false, true]);
    // Hidden by an ancestor.
    let _ = list.clone().hidden(true);
    let _ = ui.frame(size, 1.0);
    assert_eq!(heard.borrow().last(), Some(&false));
    let _ = list.hidden(false);
    let _ = ui.frame(size, 1.0);
    assert_eq!(heard.borrow().last(), Some(&true));
}

#[test]
fn reduced_motion_is_tracked_so_what_reads_it_follows_the_setting() {
    let ui = ui();
    let seen: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let (u2, s2) = (ui.clone(), seen.clone());
    let _fx = remedy_ui_reactive::Effect::new(move || s2.borrow_mut().push(u2.reduced_motion()));
    ui.set_reduced_motion(true);
    ui.set_reduced_motion(true);
    ui.set_reduced_motion(false);
    assert_eq!(*seen.borrow(), vec![false, true, false]);
}

#[test]
fn closing_a_view_lets_everything_it_held_go() {
    let ui = ui();
    let gone = Rc::new(Cell::new(false));
    let g = gone.clone();
    // Closures that hold the view itself, as widgets' do: a cycle until the view is closed.
    let u2 = ui.clone();
    let b = ui.el("button").on_click(move || {
        let _ = u2.root();
    });
    let u3 = ui.clone();
    let b = b.bind(move || {
        let _ = u3.reduced_motion();
    });
    let u4 = ui.clone();
    let _ = ui.root().on_key(move |_| {
        let _ = u4.root();
        false
    });
    let u5 = ui.clone();
    let _ = ui.after(Duration::from_secs(60), move || {
        let _ = u5.root();
    });
    ui.root().child(b.on_removed(move || g.set(true)));
    let _ = ui.frame(Size::new(100.0, 100.0), 1.0);
    let weak = ui.downgrade();
    ui.close();
    assert!(gone.get(), "elements heard they were removed");
    drop(ui);
    assert!(weak.upgrade().is_none(), "nothing keeps the closed view alive");
}

#[test]
fn packs_hear_about_memory_and_the_machine() {
    use remedy_ui_view::device;
    let heard = Rc::new(Cell::new(0));
    let h = heard.clone();
    device::on_memory_pressure(move |_| h.set(h.get() + 1));
    device::memory_pressure(device::MemoryPressure::Critical);
    assert_eq!(heard.get(), 1);
    device::set_total_memory(Some(16 << 30));
    assert_eq!(device::total_memory(), Some(16 << 30));
    device::set_total_memory(None);
    assert_eq!(device::total_memory(), None);
}

#[test]
fn painters_write_text_and_know_the_layout_direction() {
    use remedy_ui_view::TextPaint;
    let ui = ui();
    let seen = Rc::new(Cell::new((0.0f32, 0.0f32, false)));
    let s2 = seen.clone();
    let el = ui.el("canvas").width(160.0).height(40.0).paint(move |cx| {
        let small = TextPaint { size: 12.0, weight: 400.0, color: BLUE, mono: false };
        let big = TextPaint { size: 24.0, ..small };
        let a = cx.measure_text("Fret 12", small);
        let b = cx.measure_text("Fret 12", big);
        // Drawn centered on the middle of the element.
        let drawn = cx.text("Fret 12", Point::new(80.0, 20.0), Vec2::new(0.5, 0.5), big);
        assert_eq!(drawn, b);
        s2.set((a.width, b.width, cx.rtl));
    });
    ui.root().child(el.clone());
    let mut screen = Screen::new(ui);
    assert!(screen.frame());
    let (small, big, rtl) = seen.get();
    assert!(small > 10.0 && big > small * 1.8, "text measures by its size: {small} {big}");
    assert!(!rtl && !el.is_rtl());
    // The text is really there: blue pixels near the middle of the element, none far from it.
    let b = el.bounds().unwrap_or_default();
    let white = |x0: f32, x1: f32| {
        (x0 as u32..x1 as u32).any(|x| {
            (b.min.y as u32..b.max.y as u32).any(|y| screen.px(x, y)[0] < 60 && screen.px(x, y)[2] > 200)
        })
    };
    assert!(white(b.min.x + 55.0, b.max.x - 55.0));
    assert!(!white(b.min.x, b.min.x + 20.0));
    // A right-to-left window says so to its painters and elements.
    let _ = screen.ui.root().layout(|l| l.direction = remedy_ui_layout::Direction::Rtl);
    screen.ui.root().child(screen.ui.el("canvas").width(10.0).height(10.0).paint(move |cx| assert!(cx.rtl)));
    assert!(screen.frame());
    assert!(el.is_rtl());
}

#[test]
fn merged_changes_fold_into_one_entry_until_the_run_ends() {
    let ui = ui();
    let value = Rc::new(Cell::new(0));
    let set = |label: &str, key: &str, from: i32, to: i32| {
        let (u, r) = (value.clone(), value.clone());
        value.set(to);
        ui.record_merged_change(label, None, key, move || u.set(from), move || r.set(to));
    };
    // A drag: three changes to one color, one entry.
    set("Change accent", "accent", 0, 1);
    set("Change accent", "accent", 1, 2);
    set("Change accent", "accent", 2, 3);
    assert_eq!(ui.history().len(), 1);
    // Another key starts its own entry.
    set("Change body size", "body", 3, 4);
    assert_eq!(ui.history().len(), 2);
    assert!(ui.undo_change());
    assert_eq!(value.get(), 3);
    assert!(ui.undo_change());
    assert_eq!(value.get(), 0, "one undo goes back to before the whole drag");
    assert!(ui.redo_change());
    assert_eq!(value.get(), 3, "and redo makes its last change again");
    // A run that was undone and redone is closed.
    set("Change accent", "accent", 3, 5);
    assert_eq!(ui.history().len(), 2);
    // So is one ended explicitly.
    ui.end_merge();
    set("Change accent", "accent", 5, 6);
    assert_eq!(ui.history().len(), 3);
    // Plain changes never fold.
    let (u, r) = (value.clone(), value.clone());
    ui.record_change("Reset", None, move || u.set(6), move || r.set(0));
    set("Change accent", "accent", 0, 7);
    assert_eq!(ui.history().len(), 5);
    // An agent's change does not fold into the person's.
    let (u, r) = (value.clone(), value.clone());
    ui.acting_as("assistant", || {
        ui.record_merged_change("Change accent", None, "accent", move || u.set(7), move || r.set(8))
    });
    assert_eq!(ui.history().len(), 6);
}

#[test]
fn fonts_added_later_and_new_families_reshape_every_text() {
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    let ui = Ui::new(Theme::dark_forest(), sheet, FontCollection::new());
    let label = ui.el("label").text(remedy_ui_view::RichText::plain("Save changes")).nowrap();
    ui.root().child(ui.el("box").row().child(label.clone()));
    let size = Size::new(600.0, 200.0);
    let _ = ui.frame(size, 1.0);
    let width = |e: &remedy_ui_view::El| e.bounds().map(|b| b.width()).unwrap_or(0.0);
    let without = width(&label);
    assert!(ui.font_families().is_empty());
    let face = remedy_ui_fonts::all().into_iter().next().unwrap_or_else(|| unreachable!("a bundled font"));
    let names = ui.add_font(face.data.to_vec()).unwrap_or_else(|e| unreachable!("a font: {e}"));
    assert!(names.iter().any(|n| n == face.family));
    assert!(ui.font_families().iter().any(|n| n == face.family));
    let _ = ui.frame(size, 1.0);
    let with = width(&label);
    assert!(with > without, "the text was shaped again with the new font: {without} -> {with}");
    assert!(ui.add_font(b"not a font".to_vec()).is_err());

    // A theme naming other families shapes everything again too.
    let mut t = Theme::dark_forest();
    t.font_families = vec![face.family.to_owned()];
    ui.set_theme(t.clone(), StyleSheet::new());
    let _ = ui.frame(size, 1.0);
    let named = width(&label);
    t.font_families = vec!["A family nobody has".to_owned()];
    ui.set_theme(t, StyleSheet::new());
    let _ = ui.frame(size, 1.0);
    assert!(named > 0.0 && width(&label) < named, "shaped again: a family nobody has draws nothing here");
}
