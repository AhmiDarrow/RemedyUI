//! The core widgets, driven by pointer and keyboard on a real view with the theme sheet.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Point, Size};
use remedy_ui_input::{Buttons, Key, KeyEvent, PointerEvent, PointerPhase, ScrollPhysics};
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
        let r = self.ui.key(&KeyEvent::press(k).with_text(None));
        self.frame();
        r
    }

    fn semantics(&self) -> String {
        format!("{:?}", self.ui.take_accessibility_update())
    }
}

fn ptr(phase: PointerPhase, p: Point, buttons: Buttons) -> PointerEvent {
    PointerEvent::new(phase, p).with_buttons(buttons).with_modifiers(remedy_ui_input::Modifiers::NONE)
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
fn an_unavailable_segment_cannot_be_chosen() {
    let mut t = T::new();
    let seg = Signal::new(0usize);
    let g = w::segmented_unavailable(
        &t.ui,
        "Who sees it",
        &["This room", "One person", "Everyone"],
        seg,
        &[(1, "Nobody else is here.".to_owned())],
    );
    t.ui.root().child(g.clone());
    t.frame();
    let parts = g.child_elements();
    let [room, one, everyone] = parts.as_slice() else { unreachable!("three options") };
    t.click(one);
    assert_eq!(seg.get(), 0, "a click on an unavailable option does nothing");
    assert!(one.states().contains(remedy_ui_style::States::DISABLED));
    room.focus();
    t.key(Key::ArrowRight);
    assert_eq!(seg.get(), 2, "the arrows step over it");
    t.key(Key::ArrowLeft);
    assert_eq!(seg.get(), 0);
    t.click(everyone);
    assert_eq!(seg.get(), 2, "the others still choose");
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
fn a_tooltip_in_the_corner_stays_inside_the_window() {
    let mut t = T::new();
    // A gear at the top right, its tooltip long: it must slide back, not run off the edge.
    let b = w::icon_button(&t.ui, w::Icon::Settings, "Settings and preferences for this window", || {});
    t.ui.root().child(w::row(&t.ui).fill_width().justify(remedy_ui_view::Justify::End).child(b.clone()));
    t.frame();
    let c = b.bounds().unwrap_or_default().center();
    t.ui.pointer(&ptr(PointerPhase::Move, c, Buttons::default()));
    t.frame();
    t.fire();
    t.frame();
    t.frame();
    let mut tips = Vec::new();
    for layer in t.ui.layers() {
        let mut stack = vec![layer];
        while let Some(e) = stack.pop() {
            if e.info().is_some_and(|i| i.widget == "tooltip") {
                tips.push(e.clone());
            }
            stack.extend(e.child_elements());
        }
    }
    let tip = tips.first().and_then(remedy_ui_view::El::bounds).unwrap_or_default();
    assert!(tip.width() > 50.0, "the tooltip shows: {tip:?}");
    assert!(tip.min.x >= 0.0 && tip.max.x <= 400.0 && tip.max.y <= 300.0, "inside the window: {tip:?}");
    assert!(
        tips.first().and_then(remedy_ui_view::El::info).is_some_and(|i| i.style.opacity > 0.99),
        "and visible once placed"
    );
    // The window narrows while it shows: placed again inside it.
    for _ in 0..3 {
        let _ = t.ui.frame(remedy_ui_core::Size::new(260.0, 300.0), 1.0);
        let _ = t.ui.take_requests();
    }
    let tip = tips.first().and_then(remedy_ui_view::El::bounds).unwrap_or_default();
    assert!(tip.max.x <= 260.0, "still inside a narrower window: {tip:?}");
    // Its button goes (a panel rebuilt): so does the tooltip.
    let tip_id = tips.first().map(remedy_ui_view::El::id);
    t.ui.root().clear();
    t.frame();
    assert!(tip_id.is_some_and(|id| !t.ui.tree().contains(id)), "the tooltip went with its button");
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
fn assistive_technology_can_dismiss_a_toast_as_a_click_does() {
    let mut t = T::new();
    t.ui.root().child(w::text(&t.ui, "Below"));
    t.frame();
    let before = t.ui.tree().len();
    let toast = w::toast(&t.ui, "Saved", Duration::from_secs(60));
    t.frame();
    assert!(t.ui.tree().len() > before);
    let s = toast.composed_semantics().unwrap_or_else(|| unreachable!("a toast has semantics"));
    assert!(s.actions().contains(&remedy_ui_a11y::Action::Click), "the click that dismisses it is offered");
    assert!(t.ui.accessibility_action(toast.node_id(), remedy_ui_a11y::Action::Click));
    t.frame();
    assert_eq!(t.ui.tree().len(), before, "dismissed before its time");
}

#[test]
fn toasts_showing_together_stack_in_the_corner_asked_for() {
    let mut t = T::new();
    t.frame();
    let before = t.ui.tree().len();
    let first = w::toast_at(&t.ui, "Shared Photos", Duration::from_secs(60), remedy_ui_layout::Align::End);
    let second = w::toast_at(&t.ui, "Invite copied", Duration::from_secs(60), remedy_ui_layout::Align::End);
    t.frame();
    let (a, b) = (first.bounds().unwrap_or_default(), second.bounds().unwrap_or_default());
    assert!(a.max.y <= b.min.y, "the newer toast sits under the older one, not on it: {a:?} {b:?}");
    assert!(a.max.x > 300.0 && b.max.x > 300.0, "both at the end of a 400 wide window: {a:?} {b:?}");
    assert!(b.max.y <= 300.0, "inside the window: {b:?}");
    // A dialog opened now is under the toasts showing, and under the next one.
    let top = |t: &T| {
        let [_, overlays, _] = t.ui.layers();
        overlays.child_elements().pop().and_then(|e| e.info()).map(|i| i.widget)
    };
    let m = w::modal(&t.ui, "Settings", w::text(&t.ui, "Body"), true);
    assert_eq!(top(&t), Some("toast_host"), "a dialog's scrim does not dim the toasts showing");
    let third = w::toast_at(&t.ui, "Saved", Duration::from_secs(60), remedy_ui_layout::Align::End);
    t.frame();
    assert_eq!(top(&t), Some("toast_host"), "toasts are the top overlay");
    m.close();
    for toast in [first, second, third] {
        assert!(t.ui.accessibility_action(toast.node_id(), remedy_ui_a11y::Action::Click));
    }
    t.frame();
    assert_eq!(t.ui.tree().len(), before, "the last toast to go takes the layer with it");
}

#[test]
fn a_fitted_modal_keeps_its_head_and_foot_while_its_middle_scrolls() {
    let mut t = T::new();
    t.frame();
    let (head, foot) = (w::text(&t.ui, "Share a folder"), w::button(&t.ui, "Save", || {}));
    let mut rows = w::column(&t.ui);
    for i in 0..60 {
        rows = rows.child(w::text(&t.ui, format!("Folder {i}")));
    }
    let middle = w::scroll(&t.ui).child(rows);
    let content = w::column(&t.ui).child(head.clone()).child(middle.clone()).child(foot.clone());
    let m = w::modal_fitted(&t.ui, "Share a folder", content, true);
    t.frame();
    let (d, h, f, mid) = (
        m.body().bounds().unwrap_or_default(),
        head.bounds().unwrap_or_default(),
        foot.bounds().unwrap_or_default(),
        middle.bounds().unwrap_or_default(),
    );
    assert!(d.height() <= 300.0 * 0.92 + 0.5, "the dialog fits the 300 high window: {d:?}");
    assert!(h.min.y >= d.min.y && f.max.y <= d.max.y, "head and foot are inside it: {h:?} {f:?} in {d:?}");
    assert!(mid.height() > 0.0 && mid.max.y <= f.min.y, "the middle took what was left and scrolls: {mid:?}");
    m.close();
}

#[test]
fn a_disclosure_opens_and_closes_from_its_head_and_says_which() {
    let mut t = T::new();
    let open = Signal::new(false);
    let body = w::text(&t.ui, "Paste a Hearth invite");
    let d = w::disclosure(&t.ui, "I have an invite", body.clone(), open);
    t.ui.root().child(d.clone());
    t.frame();
    assert!(body.bounds().is_none_or(|b| b.height() == 0.0), "closed: the body takes no room");
    let head =
        d.child_elements().into_iter().next().unwrap_or_else(|| unreachable!("a disclosure has a head"));
    let s = head.composed_semantics().unwrap_or_else(|| unreachable!("the head has semantics"));
    assert_eq!((s.name(), s.expanded_state()), (Some("I have an invite"), Some(false)));
    head.focus();
    t.key(Key::Enter);
    t.frame();
    assert_eq!(open.get_untracked().ok(), Some(true), "Enter on the head opens it");
    assert!(body.bounds().is_some_and(|b| b.height() > 0.0), "open: the body shows");
    let s = head.composed_semantics().unwrap_or_else(|| unreachable!("the head has semantics"));
    assert_eq!(s.expanded_state(), Some(true));
    let c = head.bounds().unwrap_or_default().center();
    t.ui.pointer(&ptr(PointerPhase::Down, c, Buttons::PRIMARY));
    t.ui.pointer(&ptr(PointerPhase::Up, c, Buttons::default()));
    t.frame();
    assert_eq!(open.get_untracked().ok(), Some(false), "a click closes it again");
}

#[test]
fn a_trusted_prompt_waits_from_when_it_is_seen() {
    let mut t = T::new();
    t.ui.root().child(w::text(&t.ui, "Below"));
    t.frame();
    // Opened while the window is covered (or minimized): its delay must not run out unseen.
    t.ui.set_occluded(true);
    let answer = Rc::new(Cell::new(None));
    let a2 = answer.clone();
    let _p = w::trusted_prompt(&t.ui, "Allow?", "Something wants access.", move |yes| a2.set(Some(yes)));
    t.frame();
    assert!(t.timers.is_empty(), "no delay runs while the prompt is not seen");
    t.fire();
    t.key(Key::Enter);
    assert_eq!(answer.get(), None, "unseen, its buttons stay disabled");
    // Shown: the delay starts now, and holds the buttons for its whole length.
    t.ui.set_occluded(false);
    t.frame();
    assert_eq!(t.timers.len(), 1, "the delay starts with the first frame that shows it");
    t.key(Key::Enter);
    assert_eq!(answer.get(), None, "shown but not yet pressable");
    // Covered again before it armed: it disarms, and starts over when seen.
    t.ui.set_occluded(true);
    t.frame();
    assert!(t.timers.is_empty(), "the pending delay is cancelled");
    t.ui.set_occluded(false);
    t.frame();
    t.key(Key::Enter);
    assert_eq!(answer.get(), None);
    t.fire();
    t.key(Key::Enter);
    assert_eq!(answer.get(), Some(false), "pressable once a full delay passed in view");
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
        t.ui.key(&KeyEvent::press(Key::Character(c.into())).with_text(Some(c.into())));
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

#[test]
fn drags_keep_going_when_the_pointer_leaves_the_widget() {
    let mut t = T::new();
    let v = Signal::new(0.5f64);
    let s = w::slider(&t.ui, "Volume", v, 0.0, 1.0, 0.0).width(200.0);
    t.ui.root().child(w::row(&t.ui).child(s.clone()));
    t.frame();
    let b = s.bounds().unwrap_or_default();
    let y = (b.min.y + b.max.y) / 2.0;
    t.ui.pointer(&ptr(PointerPhase::Down, Point::new(b.min.x + 100.0, y), Buttons::PRIMARY));
    // Far outside the slider, and below it.
    t.ui.pointer(&ptr(PointerPhase::Move, Point::new(b.max.x + 150.0, y + 80.0), Buttons::PRIMARY));
    t.frame();
    assert!((v.get() - 1.0).abs() < 1e-9, "the drag reached the end: {}", v.get());
    t.ui.pointer(&ptr(PointerPhase::Up, Point::new(b.max.x + 150.0, y + 80.0), Buttons::default()));
    t.ui.pointer(&ptr(PointerPhase::Move, Point::new(b.min.x + 10.0, y), Buttons::default()));
    assert!((v.get() - 1.0).abs() < 1e-9, "after the release, hovering does not drag");
}

#[test]
fn context_menus_learn_where_they_were_opened() {
    let mut t = T::new();
    let seen: Rc<Cell<Option<Point>>> = Rc::new(Cell::new(None));
    let asked = Rc::new(Cell::new(0));
    let (s2, a2) = (seen.clone(), asked.clone());
    let target = w::button(&t.ui, "Target", || {});
    w::context_menu_at(&target, move |at| {
        s2.set(at);
        a2.set(a2.get() + 1);
        vec![w::MenuItem::new("Copy", || {})]
    });
    t.ui.root().child(target.clone());
    t.frame();
    let at = target.bounds().unwrap_or_default().center();
    t.ui.pointer(&ptr(PointerPhase::Down, at, Buttons::SECONDARY));
    assert_eq!((seen.get(), asked.get()), (Some(at), 1), "a right press hands over its point");
}

#[test]
fn a_popover_keeps_presses_on_its_own_buttons() {
    let mut t = T::new();
    let pressed = Rc::new(Cell::new(0));
    let p2 = pressed.clone();
    let anchor = w::button(&t.ui, "Open", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let inside = w::button(&t.ui, "Apply", move || p2.set(p2.get() + 1));
    let layer = w::popover(&t.ui, &anchor, inside.clone());
    t.frame();
    t.click(&inside);
    assert_eq!(pressed.get(), 1, "the button inside took the press");
    assert!(t.ui.tree().contains(layer.id()), "pressing inside does not close it");
    t.ui.pointer(&ptr(PointerPhase::Down, Point::new(390.0, 290.0), Buttons::PRIMARY));
    t.frame();
    assert!(!t.ui.tree().contains(layer.id()), "a press outside closes it");
}

#[test]
fn escape_closes_popups_and_focus_goes_back_to_their_trigger() {
    let mut t = T::new();
    let anchor = w::button(&t.ui, "Open", || {});
    let sel = w::select(&t.ui, "Fruit", vec!["Apple".into(), "Pear".into()], Signal::new(0));
    t.ui.root().child(t.ui.el("col").column().child(anchor.clone()).child(sel.clone()));
    t.frame();

    // A popover.
    anchor.focus();
    let layer = w::popover(&t.ui, &anchor, w::button(&t.ui, "Apply", || {}));
    t.frame();
    assert!(!anchor.is_focused(), "focus moved inside");
    assert!(t.key(Key::Escape));
    t.frame();
    assert!(!t.ui.tree().contains(layer.id()), "Escape closes the popover");
    assert!(anchor.is_focused(), "focus goes back to its trigger");

    // A select's list box.
    sel.focus();
    assert!(t.key(Key::Enter));
    t.frame();
    assert!(!sel.is_focused());
    assert!(t.key(Key::Escape));
    t.frame();
    assert!(sel.is_focused(), "focus goes back to the select");

    // An app's own menu in an overlay, as Portal's Cast and Theme menus: Escape reaches the
    // list's key handler from the focused entry.
    anchor.focus();
    let slot: Rc<std::cell::RefCell<Option<El>>> = Rc::default();
    let entry = w::button(&t.ui, "The Ris", || {});
    let (s2, a2) = (slot.clone(), anchor.clone());
    let list = t.ui.el("col").column().child(entry.clone()).on_key(move |e| {
        if e.pressed && e.key == Key::Escape {
            if let Some(l) = s2.borrow_mut().take() {
                l.remove();
            }
            a2.focus();
            return true;
        }
        false
    });
    let layer = t.ui.overlay(list);
    *slot.borrow_mut() = Some(layer.clone());
    entry.focus();
    t.frame();
    assert!(t.key(Key::Escape));
    t.frame();
    assert!(!t.ui.tree().contains(layer.id()) && anchor.is_focused());
}

#[test]
fn a_virtual_list_fills_its_height_and_shows_changed_items() {
    let mut t = T::new();
    let items = Signal::new((0..5).map(|i| format!("row {i}")).collect::<Vec<_>>());
    let ui = t.ui.clone();
    let list = w::virtual_list(&t.ui, "Rows", items, 200.0, move |_, s: &String| w::text(&ui, s.clone()));
    t.ui.root().child(t.ui.el("col").column().height(280.0).fill_width().child(list.clone()));
    for _ in 0..3 {
        t.frame();
    }
    let shown = |l: &El| -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![l.clone()];
        while let Some(e) = stack.pop() {
            if let Some(text) = e.info().and_then(|i| i.text) {
                out.push(text);
            }
            let mut kids = e.child_elements();
            kids.reverse();
            stack.extend(kids);
        }
        out
    };
    // The estimate is far too tall; once laid out, every short row that fits is built.
    assert_eq!(shown(&list).len(), 5, "{:?}", shown(&list));
    // New items of the same count show, not the old rows.
    items.set((0..5).map(|i| format!("new {i}")).collect());
    t.frame();
    assert_eq!(shown(&list).first().map(String::as_str), Some("new 0"));
}

#[test]
fn keyed_views_apply_signal_vec_operations_without_rebuilding() {
    let mut t = T::new();
    let items = remedy_ui_reactive::SignalVec::new(vec![1u32, 2, 3]);
    let built = Rc::new(Cell::new(0));
    let b2 = built.clone();
    let row_ui = t.ui.clone();
    let col = w::for_each(
        &w::column(&t.ui),
        items,
        |i| *i,
        move |i| {
            b2.set(b2.get() + 1);
            w::text(&row_ui, format!("Row {i}"))
        },
    );
    t.ui.root().child(col.clone());
    t.frame();
    assert_eq!(built.get(), 3);
    let ids = |col: &El| col.child_elements().iter().map(El::id).collect::<Vec<_>>();
    let before = ids(&col);
    let _ = items.push(4);
    let _ = items.insert(0, 0);
    t.frame();
    assert_eq!(built.get(), 5, "a push and an insert mount one row each");
    let after = ids(&col);
    assert_eq!(after.get(1..4), Some(before.as_slice()), "the old rows kept their elements");
    let _ = items.move_item(4, 0);
    let _ = items.remove(1);
    t.frame();
    assert_eq!(built.get(), 5, "moves and removals build nothing");
    let texts: Vec<String> =
        col.child_elements().iter().filter_map(|c| c.info().and_then(|i| i.text)).collect();
    assert_eq!(texts, ["Row 4", "Row 1", "Row 2", "Row 3"]);
    let _ = items.set(0, 40);
    t.frame();
    assert_eq!(built.get(), 6, "a changed item rebuilds its own row");
    let _ = items.clear();
    t.frame();
    assert!(col.child_elements().is_empty());
}

#[test]
fn virtual_lists_over_a_signal_vec_build_only_new_rows_in_view() {
    let mut t = T::new();
    let items = remedy_ui_reactive::SignalVec::new((0..1000u32).collect());
    let built = Rc::new(Cell::new(0));
    let (b2, ui) = (built.clone(), t.ui.clone());
    let vl = w::virtual_list(&t.ui, "Messages", items, 20.0, move |_, i| {
        b2.set(b2.get() + 1);
        w::text(&ui, format!("Message {i}")).height(20.0)
    });
    let _ = vl.clone().height(200.0);
    t.ui.root().child(vl.clone());
    t.frame();
    t.frame();
    let first = built.get();
    assert!(first > 5 && first < 60, "only rows near the viewport: {first}");
    let _ = items.push(1000);
    t.frame();
    assert_eq!(built.get(), first, "a row added far below the viewport builds nothing");
    let _ = items.insert(0, 9999);
    t.frame();
    t.frame();
    assert_eq!(built.get(), first + 1, "one row added in view builds that row alone");
    let rows = vl.child_elements().get(1).map(El::child_elements).unwrap_or_default();
    let text = rows.first().and_then(El::info).and_then(|i| i.text);
    assert_eq!(text.as_deref(), Some("Message 9999"));
}

/// A list row as a screen reader meets it: name, place in the list, states and node id.
type ReaderRow = (Option<String>, Option<(usize, usize)>, Vec<&'static str>, u64);

fn list_rows(t: &T) -> Vec<ReaderRow> {
    t.ui.reader_view()
        .into_iter()
        .filter(|n| n.role == remedy_ui_a11y::Role::Option)
        .map(|n| (n.name, n.position, n.states, n.id))
        .collect()
}

#[test]
fn list_rows_tell_screen_readers_their_name_place_and_selection() {
    let mut t = T::new();
    t.ui.connect_accessibility();
    let items = remedy_ui_reactive::SignalVec::new(vec!["Alex", "Sam", "Kai"]);
    let sel = Signal::new(None::<&'static str>);
    let ui = t.ui.clone();
    let l = w::list(&t.ui, "People", items, |s| *s, move |s| w::text(&ui, *s), sel, |_| {});
    t.ui.root().child(l);
    t.frame();
    let seen = list_rows(&t);
    let names: Vec<_> = seen.iter().map(|r| (r.0.clone(), r.1)).collect();
    assert_eq!(
        names,
        [
            (Some("Alex".into()), Some((1, 3))),
            (Some("Sam".into()), Some((2, 3))),
            (Some("Kai".into()), Some((3, 3)))
        ]
    );
    // Assistive technology selects a row by clicking it.
    let sam = seen.get(1).map(|r| r.3).unwrap_or_default();
    assert!(t.ui.accessibility_action(sam, remedy_ui_a11y::Action::Click));
    t.frame();
    assert_eq!(sel.get(), Some("Sam"));
    let selected: Vec<_> =
        list_rows(&t).into_iter().filter(|r| r.2.contains(&"selected")).map(|r| r.0).collect();
    assert_eq!(selected, [Some("Sam".to_string())]);
    // Places follow inserts.
    let _ = items.insert(0, "Ann");
    t.frame();
    t.frame();
    let places: Vec<_> = list_rows(&t).into_iter().map(|r| r.1).collect();
    assert_eq!(places, [Some((1, 4)), Some((2, 4)), Some((3, 4)), Some((4, 4))]);
}

fn transitions() -> w::ListTransitions {
    let tween = remedy_ui_view::motion::Motion::Tween {
        duration: Duration::from_millis(100),
        easing: remedy_ui_view::motion::Easing::Linear,
    };
    w::ListTransitions::new()
        .enter(remedy_ui_view::Transition::fade(tween))
        .exit(remedy_ui_view::Transition::fade(tween))
        .moves(tween)
}

fn keyed_rows(t: &T, items: remedy_ui_reactive::SignalVec<u32>) -> El {
    let ui = t.ui.clone();
    let col = w::for_each_with(
        &w::column(&t.ui),
        items,
        |i| *i,
        move |i| w::text(&ui, format!("Row {i}")).height(20.0),
        transitions(),
    );
    t.ui.root().child(col.clone());
    col
}

fn row_named(col: &El, name: &str) -> Option<El> {
    col.child_elements().into_iter().find(|c| c.info().and_then(|i| i.text).as_deref() == Some(name))
}

#[test]
fn keyed_rows_fade_in_fade_out_and_glide_to_new_places() {
    let mut t = T::new();
    t.ui.set_frame_time(Duration::ZERO);
    let items = remedy_ui_reactive::SignalVec::new(vec![1u32, 2, 3]);
    let col = keyed_rows(&t, items);
    t.frame();
    let opacity = |el: &El| el.info().map_or(-1.0, |i| i.style.opacity);
    let first = row_named(&col, "Row 1").unwrap_or_else(|| unreachable!("row 1"));
    assert_eq!(opacity(&first), 1.0, "the rows there at the start do not fade in");

    // Inserting at the top: the new row fades in, the others glide down from where they were.
    let _ = items.insert(0, 0);
    t.ui.set_frame_time(Duration::from_millis(1));
    t.frame();
    let zero = row_named(&col, "Row 0").unwrap_or_else(|| unreachable!("row 0"));
    assert!(opacity(&zero) < 0.2, "it starts faded: {}", opacity(&zero));
    let (offset, _) = first.drawn_offset();
    assert!(offset.y < -10.0, "row 1 is drawn where it was, above its new place: {offset:?}");
    t.ui.set_frame_time(Duration::from_millis(200));
    t.frame();
    assert_eq!(opacity(&zero), 1.0);
    assert_eq!(first.drawn_offset().0, remedy_ui_core::Vec2::ZERO, "it arrived");

    // Removing: the row stays, inert and out of the accessibility tree, until it has faded.
    let _ = items.remove(2);
    t.ui.set_frame_time(Duration::from_millis(201));
    t.frame();
    let two = row_named(&col, "Row 2").unwrap_or_else(|| unreachable!("still there while it leaves"));
    let sem = two.info().and_then(|i| i.semantics).map(|s| s.is_hidden());
    assert_eq!(sem, Some(true));
    t.ui.set_frame_time(Duration::from_millis(400));
    t.frame();
    t.frame();
    assert!(row_named(&col, "Row 2").is_none(), "gone once it faded");
    let texts: Vec<String> =
        col.child_elements().iter().filter_map(|c| c.info().and_then(|i| i.text)).collect();
    assert_eq!(texts, ["Row 0", "Row 1", "Row 3"]);

    // Moving keeps order while rows leave; everything is instant under reduced motion.
    t.ui.set_reduced_motion(true);
    let _ = items.move_item(2, 0);
    let _ = items.remove(1);
    t.frame();
    t.frame();
    let texts: Vec<String> =
        col.child_elements().iter().filter_map(|c| c.info().and_then(|i| i.text)).collect();
    assert_eq!(texts, ["Row 3", "Row 1"], "the leaving row went at once");
    let three = row_named(&col, "Row 3").unwrap_or_else(|| unreachable!("row 3"));
    assert_eq!(three.drawn_offset().0, remedy_ui_core::Vec2::ZERO, "no glide under reduced motion");
}

#[test]
fn a_select_puts_its_value_where_justify_says() {
    let mut t = T::new();
    let options = vec!["Apple".to_owned(), "Banana".to_owned()];
    let centered = w::select(&t.ui, "Fruit", options.clone(), Signal::new(0usize)).width(240.0);
    let started = w::select(&t.ui, "Fruit", options, Signal::new(0usize))
        .width(240.0)
        .justify(remedy_ui_layout::Justify::SpaceBetween);
    t.ui.root().child(w::column(&t.ui).child(centered.clone()).child(started.clone()));
    t.frame();
    let value_x = |s: &El| {
        let kids = s.child_elements();
        let v = kids.first().and_then(|k| k.bounds()).unwrap_or_default();
        let chevron = kids.last().and_then(|k| k.bounds()).unwrap_or_default();
        (v.min.x - s.bounds().unwrap_or_default().min.x, s.bounds().unwrap_or_default().max.x - chevron.max.x)
    };
    let (c_start, _) = value_x(&centered);
    let (s_start, s_end) = value_x(&started);
    assert!(c_start > 40.0, "centered by default: {c_start}");
    assert!(s_start < 24.0, "the value at the start, past the padding only: {s_start}");
    assert!(s_end < 24.0, "the chevron at the end: {s_end}");
}

#[test]
fn choices_with_nothing_chosen_take_arrows_and_letters() {
    let mut t = T::new();
    let options = vec!["Apple".to_owned(), "Banana".to_owned(), "Cherry".to_owned()];
    // An index past the options: a stored value none of them is.
    let picked = Signal::new(usize::MAX);
    let sel = w::select(&t.ui, "Fruit", options, picked);
    let parts = Signal::new(usize::MAX);
    let seg = w::segmented(&t.ui, "Part", &["Left", "Right"], parts);
    t.ui.root().child(w::column(&t.ui).child(sel.clone()).child(seg.clone()));
    t.frame();
    // A combo box, as a page's `<select>`: named, collapsed until pressed.
    let said = || sel.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!((said().role(), said().name()), (remedy_ui_a11y::Role::ComboBox, Some("Fruit")));
    assert_eq!(said().expanded_state(), Some(false));
    sel.focus();
    assert!(t.key(Key::Enter));
    assert_eq!(said().expanded_state(), Some(true), "open while its list shows");
    // Its list is a list box of options, the chosen one selected.
    let reader = t.ui.reader_view();
    assert!(reader.iter().any(|n| n.role == remedy_ui_a11y::Role::ListBox), "a list box");
    assert_eq!(reader.iter().filter(|n| n.role == remedy_ui_a11y::Role::Option).count(), 3, "three options");
    assert!(t.key(Key::Escape));
    t.frame();
    assert_eq!(said().expanded_state(), Some(false), "closed again");
    sel.focus();
    assert!(t.key(Key::ArrowDown));
    assert_eq!(picked.get_untracked().ok(), Some(0), "down starts at the first option");
    picked.set(usize::MAX);
    t.ui.key(&KeyEvent::press(Key::Character("c".into())).with_text(Some("c".into())));
    t.frame();
    assert_eq!(picked.get_untracked().ok(), Some(2), "a letter finds its option");
    picked.set(usize::MAX);
    t.ui.key(&KeyEvent::press(Key::Character("a".into())).with_text(Some("a".into())));
    t.frame();
    assert_eq!(picked.get_untracked().ok(), Some(0), "from the first option when none is chosen");
    if let Some(first) = seg.child_elements().first() {
        first.focus();
    }
    assert!(t.key(Key::ArrowLeft));
    assert_eq!(parts.get_untracked().ok(), Some(1), "up or left starts at the last");
}

#[test]
fn a_lamp_shows_the_color_of_its_kind_of_status() {
    let mut t = T::new();
    let theme = t.ui.theme();
    let plain = w::lamp(&t.ui, "Connected", Signal::new(true));
    let problem = w::lamp(&t.ui, "Problem", Signal::new(true)).variant("danger");
    let news = w::lamp(&t.ui, "News", Signal::new(true)).variant("accent");
    let off = w::lamp(&t.ui, "Off", Signal::new(false)).variant("danger");
    t.ui.root().child(plain.clone()).child(problem.clone()).child(news.clone()).child(off.clone());
    t.frame();
    let at = |el: &El| {
        let b = el.bounds().unwrap_or_default();
        let (x, y) = ((b.min.x + b.width() / 2.0) as u32, (b.min.y + b.height() / 2.0) as u32);
        t.frame.as_ref().and_then(|f| f.pixel(x, y)).unwrap_or([0; 4])
    };
    assert_eq!(at(&plain), theme.colors.ok.to_rgba8(), "a lamp without a kind is the ok color");
    assert_eq!(at(&problem), theme.colors.danger.to_rgba8(), "a problem's lamp is the danger color");
    assert_eq!(at(&news), theme.colors.accent.to_rgba8());
    assert_eq!(at(&off), theme.derive().line.to_rgba8(), "off is off, whatever its kind");
}

#[test]
fn value_widgets_mirror_right_to_left() {
    let mut t = T::new();
    t.ui.set_locale("ar");
    let v = Signal::new(5.0f64);
    let sl = w::slider(&t.ui, "Volume", v, 0.0, 10.0, 1.0).width(200.0);
    let f = Signal::new(0.5f32);
    let split = w::split(
        &t.ui,
        remedy_ui_layout::Axis::Row,
        t.ui.el("pane").height(20.0),
        t.ui.el("pane").height(20.0),
        f,
    )
    .width(200.0)
    .height(20.0);
    t.ui.root().child(sl.clone()).child(split.clone());
    t.frame();
    sl.focus();
    // The thumb moves the way the arrow points: left is up, right to left.
    t.key(Key::ArrowLeft);
    assert_eq!(v.get(), 6.0);
    t.key(Key::ArrowRight);
    t.key(Key::ArrowRight);
    assert_eq!(v.get(), 4.0);
    // A press near the right edge is near the minimum.
    t.frame();
    let b = sl.bounds().unwrap_or_default();
    let at = Point::new(b.max.x - 2.0, (b.min.y + b.max.y) / 2.0);
    t.ui.pointer(&ptr(PointerPhase::Down, at, Buttons::PRIMARY));
    t.ui.pointer(&ptr(PointerPhase::Up, at, Buttons::default()));
    t.frame();
    assert!(v.get() <= 0.5, "the right end is the minimum: {}", v.get());
    // The split's first pane is at the right; Left moves the divider left, growing it.
    if let Some(handle) = split.child_elements().get(1) {
        handle.focus();
    }
    t.key(Key::ArrowLeft);
    assert!(f.get() > 0.5, "{}", f.get());
    // The same window left to right: arrows as before.
    t.ui.set_locale("en");
    t.frame();
    sl.focus();
    t.key(Key::ArrowLeft);
    assert!(v.get() <= 0.5);
    t.key(Key::ArrowRight);
    assert_eq!(v.get(), 1.0);
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&w::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn built_in_labels_follow_the_window_language() {
    let mut t = T::new();
    t.ui.connect_accessibility();
    let v = Signal::new(1.0f64);
    let st = w::stepper(&t.ui, "Copies", v, 0.0, 10.0, 1.0);
    let secret = w::secret_input(&t.ui, "PIN");
    let reveal = w::reveal_toggle(&t.ui, &secret);
    t.ui.root().child(st).child(secret.el()).child(reveal);
    t.frame();
    let names = |t: &T| -> Vec<String> {
        t.ui.reader_view()
            .into_iter()
            .filter_map(|n| n.name)
            .map(|n| n.replace(['\u{2068}', '\u{2069}'], ""))
            .collect()
    };
    let en = names(&t);
    for want in ["Decrease Copies", "Increase Copies", "Show password"] {
        assert!(en.iter().any(|n| n == want), "{want} in {en:?}");
    }
    t.ui.set_locale("de");
    t.frame();
    let de = names(&t);
    for want in ["Copies verringern", "Copies erhöhen", "Passwort anzeigen"] {
        assert!(de.iter().any(|n| n == want), "{want} in {de:?}");
    }
    assert_eq!(w::Icon::Close.localized_name(&t.ui), "Schließen");
}

#[test]
fn a_modal_taller_than_the_window_fits_and_scrolls() {
    let mut t = T::new();
    let tall = (0..20).fold(t.ui.el("column").column(), |c, n| c.child(w::text(&t.ui, format!("Line {n}"))));
    let last = w::button(&t.ui, "Done", || {});
    let m = w::modal(&t.ui, "Long", tall.child(last.clone()), true);
    t.frame();
    let b = m.body().bounds().unwrap_or_default();
    assert!(b.min.y >= 0.0 && b.max.y <= 300.0, "the dialog fits the 300 px window: {b:?}");
    // Its last button can be scrolled to.
    let _ = t.comp.scroll_at(Point::new(200.0, 150.0), remedy_ui_core::Vec2::new(0.0, 1000.0));
    t.frame();
    t.frame();
    let l = last.bounds().unwrap_or_default();
    assert!(l.max.y <= b.max.y + 0.5 && l.min.y >= b.min.y, "scrolled into the dialog: {l:?} in {b:?}");
}
