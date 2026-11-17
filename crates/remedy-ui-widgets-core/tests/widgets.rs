//! The core widgets, driven by pointer and keyboard on a real view with the theme sheet.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Point, Size};
use remedy_ui_input::{
    Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerKind, PointerPhase, ScrollPhysics,
};
use remedy_ui_reactive::Signal;
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};
use remedy_ui_widgets_core as w;

struct T {
    ui: Ui,
    comp: Compositor,
    cpu: CpuCompositor,
    time: u64,
    frame: Option<Frame>,
    timers: Vec<u64>,
}

impl T {
    fn new() -> T {
        let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts);
        T {
            ui,
            comp: Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED),
            cpu: CpuCompositor::new(),
            time: 0,
            frame: None,
            timers: Vec::new(),
        }
    }

    fn frame(&mut self) {
        let f = self.ui.frame(Size::new(400.0, 300.0), 1.0);
        if let Some(l) = f.layers {
            self.comp.commit(l);
        }
        for (layer, el) in f.scrollers {
            if let Some(o) = self.comp.scroll_offset(layer) {
                self.ui.set_scroll_offset(el, o);
            }
        }
        self.time += 16;
        let plan = self.comp.frame(Duration::from_millis(self.time));
        if let Ok(fr) = self.cpu.present(&plan, 400, 300, remedy_ui_core::Color::BLACK) {
            self.frame = Some(fr);
        }
        for r in self.ui.take_requests() {
            match r {
                Request::Timer { id, .. } => self.timers.push(id),
                Request::CancelTimer(id) => self.timers.retain(|t| *t != id),
                _ => {}
            }
        }
    }

    /// Fires every pending timer, as the runner does when they come due.
    fn fire(&mut self) {
        self.frame();
        for id in std::mem::take(&mut self.timers) {
            self.ui.fire_timer(id);
        }
        self.frame();
    }

    fn click(&mut self, el: &El) {
        self.frame();
        let b = el.bounds().unwrap_or_default();
        let c = Point::new((b.min.x + b.max.x) / 2.0, (b.min.y + b.max.y) / 2.0);
        self.ui.pointer(&ptr(PointerPhase::Down, c, Buttons::PRIMARY));
        self.ui.pointer(&ptr(PointerPhase::Up, c, Buttons::default()));
        self.frame();
    }

    fn key(&mut self, k: Key) -> bool {
        let r = self.ui.key(&KeyEvent {
            key: k,
            physical: String::new(),
            modifiers: Modifiers::NONE,
            pressed: true,
            repeat: false,
            composing: false,
            text: None,
        });
        self.frame();
        r
    }

    fn semantics(&self) -> String {
        format!("{:?}", self.ui.take_accessibility_update())
    }
}

fn ptr(phase: PointerPhase, p: Point, buttons: Buttons) -> PointerEvent {
    PointerEvent {
        id: 0,
        kind: PointerKind::Mouse,
        phase,
        position: p,
        buttons,
        pressure: 0.5,
        time: Duration::ZERO,
        coalesced: Vec::new(),
    }
}

#[test]
fn buttons_click_from_pointer_and_keyboard_and_say_what_they_are() {
    let mut t = T::new();
    t.ui.connect_accessibility();
    let n = Rc::new(Cell::new(0));
    let n2 = n.clone();
    let b = w::button(&t.ui, "Save", move || n2.set(n2.get() + 1)).variant("primary");
    t.ui.root().child(w::row(&t.ui).child(b.clone()));
    t.click(&b);
    assert_eq!(n.get(), 1);
    b.focus();
    t.key(Key::Enter);
    t.key(Key::Space);
    assert_eq!(n.get(), 3);
    let _ = b.clone().disabled(true);
    t.click(&b);
    assert_eq!(n.get(), 3, "a disabled button does nothing");
    let s = t.semantics();
    assert!(s.contains("Button") && s.contains("\"Save\""), "{s}");
}

#[test]
fn choices_follow_their_signals() {
    let mut t = T::new();
    let on = Signal::new(false);
    let sw = Signal::new(true);
    let pick = Signal::new(0usize);
    let seg = Signal::new(0usize);
    let c = w::check(&t.ui, "Remember", on);
    let s = w::switch(&t.ui, "Pinned", sw);
    let r = w::radio_group(&t.ui, "Size", &["Small", "Large"], pick);
    let g = w::segmented(&t.ui, "Lock after", &["Off", "5 min", "15 min"], seg);
    t.ui.root().child(c.clone()).child(s.clone()).child(r.clone()).child(g.clone());
    t.click(&c);
    assert!(on.get());
    t.click(&s);
    assert!(!sw.get());
    let first = r.child_elements().into_iter().next();
    if let Some(f) = first {
        f.focus();
    }
    t.key(Key::ArrowDown);
    assert_eq!(pick.get(), 1);
    t.key(Key::ArrowDown);
    assert_eq!(pick.get(), 0, "arrows wrap");
    let third = g.child_elements().get(2).cloned();
    if let Some(third) = third {
        t.click(&third);
    }
    assert_eq!(seg.get(), 2);
    // The box shows the app's value, whoever changes it.
    let mark = c.child_elements().into_iter().next();
    assert!(mark.as_ref().is_some_and(|m| m.states().contains(remedy_ui_style::States::CHECKED)));
    on.set(false);
    t.frame();
    assert!(
        mark.is_some_and(|m| !m.states().contains(remedy_ui_style::States::CHECKED)),
        "set from outside, shown inside"
    );
}

#[test]
fn values_step_within_bounds() {
    let mut t = T::new();
    let v = Signal::new(5.0f64);
    let st = Signal::new(1.0f64);
    let sl = w::slider(&t.ui, "Volume", v, 0.0, 10.0, 1.0);
    let sp = w::stepper(&t.ui, "Copies", st, 1.0, 3.0, 1.0);
    t.ui.root().child(sl.clone()).child(sp.clone());
    t.frame();
    sl.focus();
    t.key(Key::ArrowRight);
    assert_eq!(v.get(), 6.0);
    t.key(Key::End);
    t.key(Key::ArrowRight);
    assert_eq!(v.get(), 10.0, "clamped at max");
    t.key(Key::Home);
    assert_eq!(v.get(), 0.0);
    let plus = sp.child_elements().last().cloned();
    if let Some(p) = plus {
        for _ in 0..5 {
            t.click(&p);
        }
    }
    assert_eq!(st.get(), 3.0, "the stepper stops at its maximum");
}

#[test]
fn keyed_lists_keep_elements_and_virtual_lists_build_only_what_shows() {
    let mut t = T::new();
    let items = Signal::new(vec![1u32, 2, 3]);
    let built = Rc::new(Cell::new(0));
    let b2 = built.clone();
    let col = w::column(&t.ui);
    let row_ui = t.ui.clone();
    let col = w::for_each(
        &col,
        items,
        |i| *i,
        move |i| {
            b2.set(b2.get() + 1);
            w::text(&row_ui, format!("Row {i}"))
        },
    );
    t.ui.root().child(col.clone());
    items.set(vec![1, 2, 3]);
    t.frame();
    let ids: Vec<_> = col.child_elements().iter().map(El::id).collect();
    items.set(vec![3, 1, 2]);
    t.frame();
    let moved: Vec<_> = col.child_elements().iter().map(El::id).collect();
    let [a, b, c] = ids.as_slice() else { unreachable!("three rows") };
    assert_eq!(moved, vec![*c, *a, *b], "reordered, not rebuilt");
    items.set(vec![3, 4]);
    t.frame();
    assert_eq!(col.child_elements().len(), 2);
    assert_eq!(built.get(), 4, "one build per new key");

    let many = Signal::new((0..10_000u32).collect::<Vec<_>>());
    let ui = t.ui.clone();
    let vl = w::virtual_list(&t.ui, "Messages", many, 20.0, move |_, i| {
        w::text(&ui, format!("Message {i}")).height(20.0)
    });
    let _ = vl.clone().height(200.0);
    t.ui.root().child(vl.clone());
    t.frame();
    t.frame();
    let rows = |vl: &El| vl.child_elements().get(1).map_or(0, |c| c.child_elements().len());
    let shown = rows(&vl);
    assert!(shown > 5 && shown < 60, "only rows near the viewport exist: {shown}");
}

#[test]
fn selectable_lists_move_and_open_from_the_keyboard() {
    let mut t = T::new();
    let items = Signal::new(vec!["a", "b", "c"]);
    let sel = Signal::new(None::<&'static str>);
    let opened = Rc::new(Cell::new(""));
    let o2 = opened.clone();
    let ui = t.ui.clone();
    let l = w::list(&t.ui, "Notes", items, |s| *s, move |s| w::text(&ui, *s), sel, move |k| o2.set(k));
    t.ui.root().child(l.clone());
    t.frame();
    let first = l.child_elements().into_iter().next();
    if let Some(f) = first.clone() {
        t.click(&f);
    }
    assert_eq!(sel.get(), Some("a"));
    t.key(Key::ArrowDown);
    t.key(Key::ArrowDown);
    assert_eq!(sel.get(), Some("c"));
    t.key(Key::Enter);
    assert_eq!(opened.get(), "c");
}

#[test]
fn modals_trap_focus_close_on_escape_and_give_focus_back() {
    let mut t = T::new();
    let opener = w::button(&t.ui, "Delete…", || {});
    t.ui.root().child(opener.clone());
    t.frame();
    opener.focus();
    let deleted = Rc::new(Cell::new(false));
    let d2 = deleted.clone();
    let m = w::dialog(
        &t.ui,
        "Delete note?",
        "This cannot be undone.",
        vec![
            w::DialogButton::new("Cancel", "quiet", || {}),
            w::DialogButton::new("Delete", "danger_solid", move || d2.set(true)),
        ],
    );
    t.frame();
    assert!(m.is_open());
    assert!(!opener.is_focused(), "focus moved into the dialog");
    t.key(Key::Tab);
    t.key(Key::Tab);
    t.key(Key::Tab);
    assert!(!opener.is_focused(), "Tab stays in the dialog");
    t.key(Key::Escape);
    assert!(!m.is_open());
    assert!(opener.is_focused(), "focus came back");
    assert!(!deleted.get());
    let m = w::dialog(
        &t.ui,
        "Delete note?",
        "Sure?",
        vec![w::DialogButton::new("Delete", "danger_solid", {
            let d = deleted.clone();
            move || d.set(true)
        })],
    );
    t.frame();
    t.key(Key::Enter);
    assert!(deleted.get() && !m.is_open(), "the default button acts and closes");
}

#[test]
fn menus_open_choose_and_dismiss() {
    let mut t = T::new();
    let chosen = Rc::new(Cell::new(""));
    let (c1, c2) = (chosen.clone(), chosen.clone());
    let anchor = w::button(&t.ui, "More", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let layer = w::menu(
        &t.ui,
        &anchor,
        vec![
            w::MenuItem::new("Rename", move || c1.set("rename")),
            w::MenuItem::new("Delete", move || c2.set("delete")).danger(),
        ],
    );
    t.frame();
    t.key(Key::ArrowDown);
    t.key(Key::Enter);
    assert_eq!(chosen.get(), "delete");
    assert!(!t.ui.tree().contains(layer.id()), "choosing closes the menu");
    let layer = w::menu(&t.ui, &anchor, vec![w::MenuItem::new("Rename", || {})]);
    t.frame();
    t.ui.pointer(&ptr(PointerPhase::Down, Point::new(390.0, 290.0), Buttons::PRIMARY));
    t.frame();
    assert!(!t.ui.tree().contains(layer.id()), "a press outside dismisses");
}

#[test]
fn tooltips_wait_trusted_prompts_hold_their_buttons_and_toasts_expire() {
    let mut t = T::new();
    let b = w::icon_button(&t.ui, w::Icon::Trash, "Delete note", || {});
    t.ui.root().child(b.clone());
    t.frame();
    let c = b.bounds().unwrap_or_default().center();
    t.ui.pointer(&ptr(PointerPhase::Move, c, Buttons::default()));
    t.frame();
    let overlays_before = t.ui.tree().len();
    t.fire();
    assert!(t.ui.tree().len() > overlays_before, "the tooltip shows after the delay");
    t.ui.pointer(&ptr(PointerPhase::Move, Point::new(390.0, 290.0), Buttons::default()));
    t.frame();
    assert_eq!(t.ui.tree().len(), overlays_before, "and goes when the pointer leaves");

    let answer = Rc::new(Cell::new(None));
    let a2 = answer.clone();
    let _p = w::trusted_prompt(
        &t.ui,
        "Allow screen reading?",
        "An assistant wants to read this window.",
        move |yes| a2.set(Some(yes)),
    );
    t.frame();
    t.key(Key::Enter);
    assert_eq!(answer.get(), None, "nothing can be answered before the delay");
    t.fire();
    t.key(Key::Enter);
    assert_eq!(answer.get(), Some(false), "focus starts on Deny");

    let before = t.ui.tree().len();
    let _toast = w::toast(&t.ui, "Copied", Duration::from_secs(2));
    t.frame();
    assert!(t.ui.tree().len() > before);
    t.fire();
    assert_eq!(t.ui.tree().len(), before, "the toast went away by itself");
}

#[test]
fn the_lock_screen_hides_everything_below_it() {
    let mut t = T::new();
    t.ui.connect_accessibility();
    t.ui.root().child(w::text(&t.ui, "Top secret note"));
    t.frame();
    assert!(t.semantics().contains("Top secret note"));
    let pw = w::secret_input(&t.ui, "Master password");
    let lock = w::lock_screen(&t.ui, w::column(&t.ui).child(w::heading(&t.ui, 1, "Locked")).child(pw.el()));
    t.frame();
    let s = t.semantics();
    assert!(!s.contains("Top secret note"), "locked content leaves the accessibility tree");
    assert!(s.contains("Locked"));
    assert!(pw.el().is_focused(), "the unlock field has focus");
    lock.remove();
    t.frame();
    assert!(t.semantics().contains("Top secret note"), "unlocking brings it back");
}

#[test]
fn inputs_and_feedback_describe_themselves() {
    let mut t = T::new();
    t.ui.connect_accessibility();
    let name = w::text_input(&t.ui, "Note title");
    let pct = Signal::new(Some(0.25f32));
    t.ui.root()
        .child(w::field(&t.ui, "Title", name.el()))
        .child(w::progress(&t.ui, "Saving", pct))
        .child(w::spinner(&t.ui, "Unlocking"))
        .child(w::badge(&t.ui, "3"))
        .child(w::icon(&t.ui, w::Icon::Lock, 24.0))
        .child(w::avatar(&t.ui, "Remedy User", 32.0, remedy_ui_core::Color::rgb8(80, 120, 90)));
    t.frame();
    let s = t.semantics();
    for needle in ["Note title", "Progress", "Saving", "Unlocking", "Remedy User"] {
        assert!(s.contains(needle), "{needle} missing from {s}");
    }
    let bg = remedy_ui_themes::ThemeChoice::default().resolve(None, false).colors.bg.to_rgba8();
    let ink =
        t.frame.as_ref().map_or(0, |f| f.pixels.as_chunks::<4>().0.iter().filter(|p| **p != bg).count());
    assert!(ink > 500, "the widgets drew: {ink} pixels");
}

#[test]
fn reveal_shows_a_secret_without_exposing_it() {
    let mut t = T::new();
    t.ui.connect_accessibility();
    let pw = w::secret_input(&t.ui, "Master password");
    let eye = w::reveal_toggle(&t.ui, &pw);
    t.ui.root().child(w::row(&t.ui).child(pw.el()).child(eye.clone()));
    t.frame();
    pw.el().focus();
    for c in ["s", "3", "c"] {
        t.ui.key(&KeyEvent {
            key: Key::Character(c.into()),
            physical: String::new(),
            modifiers: Modifiers::NONE,
            pressed: true,
            repeat: false,
            composing: false,
            text: Some(c.into()),
        });
    }
    t.frame();
    let masked_width = pw.el().bounds().map(|b| b.width());
    t.click(&eye);
    assert!(pw.is_revealed());
    assert_eq!(pw.value(), "", "revealed is still not a plain value");
    let s = t.semantics();
    assert!(!s.contains("s3c"), "nor in the accessibility tree");
    t.click(&eye);
    assert!(!pw.is_revealed());
    assert_eq!(pw.el().bounds().map(|b| b.width()), masked_width);
}
