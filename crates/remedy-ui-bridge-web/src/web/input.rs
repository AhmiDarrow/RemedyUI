//! Pointer, wheel, pinch, key, focus and drag input from the page.

use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::{Point, Size};
use remedy_ui_host::{DragData, DragEvent, DropEffect, HostEvent, WindowId};
use remedy_ui_input::{
    Buttons, KeyEvent, PinchEvent, PinchPhase, PointerEvent, PointerKind, PointerPhase, WheelEvent,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::dom::{self, Listener};
use super::window::Win;
use super::{Driver, driver, post};
use crate::convert::{self, ClipboardKey, DeltaMode};

/// A touchpad pinch has ended once its wheel events stop for this long (browsers send one a
/// frame while the fingers move).
const PINCH_IDLE: i32 = 100;

/// True when the page may do what needs a user's gesture now (open a picker, fill the
/// screen). Browsers without `navigator.userActivation` decide for themselves.
pub(crate) fn user_active() -> bool {
    let Some(w) = web_sys::window() else { return false };
    let act = dom::get(w.navigator().unchecked_ref(), "userActivation");
    if act.is_undefined() || act.is_null() {
        return true;
    }
    dom::get(&act, "isActive").as_bool().unwrap_or(true)
}

fn time(e: &web_sys::Event) -> Duration {
    Duration::from_secs_f64(e.time_stamp().max(0.0) / 1000.0)
}

/// An event's position in the window's logical pixels (CSS pixels from its content's corner).
fn position(w: &Win, e: &JsValue) -> Point {
    let r = w.surface.get_bounding_client_rect();
    let x = dom::get(e, "clientX").as_f64().unwrap_or(0.0);
    let y = dom::get(e, "clientY").as_f64().unwrap_or(0.0);
    Point::new((x - r.left()) as f32, (y - r.top()) as f32)
}

fn mods(e: &web_sys::MouseEvent) -> remedy_ui_input::Modifiers {
    convert::modifiers(e.shift_key(), e.ctrl_key(), e.alt_key(), e.meta_key())
}

/// The flag of the button a `MouseEvent.button` index names.
fn button_flag(button: i16) -> Buttons {
    match button {
        0 => Buttons::PRIMARY,
        1 => Buttons::MIDDLE,
        2 => Buttons::SECONDARY,
        3 => Buttons::BACK,
        4 => Buttons::FORWARD,
        _ => Buttons::default(),
    }
}

fn pointer(w: &Rc<Win>, e: &web_sys::PointerEvent, phase: PointerPhase) {
    let kind = convert::pointer_kind(&e.pointer_type());
    let buttons = convert::buttons(e.buttons());
    let phase = match phase {
        // A second button pressed or released while another is held arrives as a move.
        PointerPhase::Move if e.button() >= 0 => {
            if buttons.contains(button_flag(e.button())) {
                PointerPhase::Down
            } else {
                PointerPhase::Up
            }
        }
        p => p,
    };
    let coalesced = if phase == PointerPhase::Move {
        let all = e.get_coalesced_events();
        let n = all.length();
        (0..n.saturating_sub(1))
            .filter_map(|i| all.get(i).dyn_into::<web_sys::PointerEvent>().ok())
            .map(|c| (position(w, &c), time(&c)))
            .collect()
    } else {
        Vec::new()
    };
    let pressure = match kind {
        PointerKind::Mouse => 0.5,
        _ => {
            let p = e.pressure();
            if p > 0.0 { p } else { 0.5 }
        }
    };
    let ev = PointerEvent::new(phase, position(w, e))
        .with_id(convert::pointer_id(e.pointer_id(), kind))
        .with_kind(kind)
        .with_buttons(buttons)
        .with_pressure(pressure)
        .with_time(time(e))
        .with_coalesced(coalesced)
        .with_modifiers(mods(e));
    let down = ev.phase == PointerPhase::Down;
    w.consumed.set(false);
    // The app handles it now (the page's event is still being dispatched), and says whether
    // it took it.
    post(HostEvent::Pointer(w.id, ev));
    if down {
        w.press_taken.set(w.consumed.get());
    }
}

/// Makes the window the one the keyboard talks to, unless focus is already inside it (the
/// hidden text field keeps it while a field edits).
fn take_focus(w: &Win) {
    let inside = dom::active_element().is_some_and(|a| w.frame.contains(Some(&a)));
    if !inside {
        let target = w.aria.focused_element().unwrap_or_else(|| w.surface.clone());
        w.aria.moving_focus.set(true);
        dom::focus(&target);
        w.aria.moving_focus.set(false);
    }
}

pub(crate) fn end_pinch(w: &Win) {
    if w.pinch.take().is_some() {
        let e = PinchEvent::new(PinchPhase::End, w.pinch_at.get(), 1.0).with_time(super::now());
        post(HostEvent::Pinch(w.id, e));
    }
}

fn pinch(w: &Rc<Win>, at: Point, scale: f32, t: Duration) {
    let Some(win) = web_sys::window() else { return };
    w.pinch_at.set(at);
    match w.pinch.take() {
        Some(h) => {
            win.clear_timeout_with_handle(h);
            post(HostEvent::Pinch(w.id, PinchEvent::new(PinchPhase::Change, at, scale).with_time(t)));
        }
        None => {
            post(HostEvent::Pinch(w.id, PinchEvent::new(PinchPhase::Start, at, 1.0).with_time(t)));
            post(HostEvent::Pinch(w.id, PinchEvent::new(PinchPhase::Change, at, scale).with_time(t)));
        }
    }
    let h = win
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            w.pinch_end_cb.as_ref().unchecked_ref(),
            PINCH_IDLE,
        )
        .ok();
    w.pinch.set(h);
}

fn wheel(w: &Rc<Win>, e: &web_sys::WheelEvent) {
    e.prevent_default();
    let at = position(w, e);
    let mode = DeltaMode::from_dom(e.delta_mode());
    let ctrl_held = driver().is_some_and(|d| d.state.borrow().ctrl_down);
    if e.ctrl_key() && !ctrl_held {
        // Ctrl without the Ctrl key: the browser's report of a touchpad pinch.
        pinch(w, at, convert::pinch_scale(e.delta_y(), mode), time(e));
        return;
    }
    let (_, _, scale) = w.size.get();
    let (pw, ph, _) = w.size.get();
    let page = Size::new(pw as f32 / scale, ph as f32 / scale);
    let (delta, precise) = convert::wheel_delta(e.delta_x(), e.delta_y(), mode, page);
    let ev = WheelEvent::new(at, delta).with_precise(precise).with_time(time(e)).with_modifiers(mods(e));
    post(HostEvent::Wheel(w.id, ev));
}

/// Safari's own pinch events (`gesturestart`, `gesturechange`, `gestureend`), whose `scale`
/// is the total since the start.
fn gesture(w: &Rc<Win>, e: &web_sys::Event, phase: PinchPhase) {
    e.prevent_default();
    let total = dom::get(e, "scale").as_f64().filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    let at = position(w, e);
    let scale = match phase {
        PinchPhase::Start => {
            w.gesture_scale.set(1.0);
            1.0
        }
        _ => {
            let step = total / w.gesture_scale.get();
            w.gesture_scale.set(total);
            step as f32
        }
    };
    post(HostEvent::Pinch(w.id, PinchEvent::new(phase, at, scale).with_time(time(e))));
}

fn key(w: &Rc<Win>, e: &web_sys::KeyboardEvent, pressed: bool) {
    let dom_key = e.key();
    if w.text.composing.get() || convert::is_ime_key(&dom_key, e.key_code(), e.is_composing()) {
        // The input method has it; what it makes arrives as composition.
        return;
    }
    if dom_key == "Dead" {
        // The next key, or the input method, brings the composed character.
        return;
    }
    let Some(d) = driver() else { return };
    if dom_key == "Control" {
        d.state.borrow_mut().ctrl_down = pressed;
    }
    let platform = d.state.borrow().settings.platform;
    let modifiers = convert::modifiers(e.shift_key(), e.ctrl_key(), e.alt_key(), e.meta_key());
    let k = convert::key(&dom_key);
    let ev = KeyEvent::press(k)
        .with_physical(e.code())
        .with_modifiers(modifiers)
        .with_pressed(pressed)
        .with_repeat(e.repeat())
        .with_text(if pressed { convert::typed_text(&dom_key, modifiers) } else { None });
    match (pressed, convert::clipboard_key(&ev.key, modifiers, platform)) {
        // The browser's `paste` event brings the text; the key goes with it.
        (true, Some(ClipboardKey::Paste)) => {
            *w.deferred_paste.borrow_mut() = Some(ev);
            schedule_paste_fallback(w.id);
        }
        // The app copies first; the browser's `copy`/`cut` event then carries it out.
        (true, Some(ClipboardKey::Copy | ClipboardKey::Cut)) => {
            w.clipboard_key.set(true);
            d.post(HostEvent::Key(w.id, ev));
        }
        (_, clip) => {
            let keep = convert::browser_keeps(&ev.key, modifiers, platform) || clip.is_some();
            d.post(HostEvent::Key(w.id, ev));
            if !keep {
                e.prevent_default();
            }
        }
    }
}

/// A paste shortcut the browser raised no `paste` event for (focus outside anything that
/// takes one) still reaches the app, after the event would have come.
fn schedule_paste_fallback(window: WindowId) {
    let cb = Closure::once_into_js(move || {
        let Some(w) = driver().and_then(|d| d.window(window)) else { return };
        let key = w.deferred_paste.borrow_mut().take();
        if let Some(k) = key {
            post(HostEvent::Key(window, k));
        }
    });
    if let Some(win) = web_sys::window() {
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 0);
    }
}

fn set_focused(w: &Win, on: bool) {
    if w.focused.replace(on) != on {
        post(HostEvent::Focused(w.id, on));
    }
}

fn drop_effect(e: DropEffect) -> &'static str {
    match e {
        DropEffect::None => "none",
        DropEffect::Copy => "copy",
        DropEffect::Move => "move",
        DropEffect::Link => "link",
    }
}

fn drag_data(dt: Option<web_sys::DataTransfer>) -> (Vec<web_sys::File>, Option<String>, Vec<String>) {
    let Some(dt) = dt else { return (Vec::new(), None, Vec::new()) };
    let files = dt.files().map(|l| (0..l.length()).filter_map(|i| l.get(i)).collect()).unwrap_or_default();
    let text = dt.get_data("text/plain").ok().filter(|t| !t.is_empty());
    let uris = dt
        .get_data("text/uri-list")
        .ok()
        .map(|l| {
            l.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    (files, text, uris)
}

/// Every listener of one window.
pub(crate) fn attach(_d: &Rc<Driver>, w: &Rc<Win>) {
    w.text.attach(w.id);
    w.aria.attach();
    let surface: web_sys::EventTarget = w.surface.clone().into();
    let frame: web_sys::EventTarget = w.frame.clone().into();
    let mut ls: Vec<Listener> = Vec::new();
    let on = |target: &web_sys::EventTarget, kind: &'static str, f: fn(&Rc<Win>, web_sys::Event)| {
        let me = Rc::downgrade(w);
        Listener::new(target, kind, move |e| {
            if let Some(w) = me.upgrade() {
                f(&w, e);
            }
        })
    };
    ls.push(on(&surface, "pointerdown", |w, e| {
        let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() else { return };
        // The page neither selects nor scrolls; the app has the pointer until it is released.
        p.prevent_default();
        let _ = w.surface.set_pointer_capture(p.pointer_id());
        w.last_down.set(Some((f64::from(p.client_x()), f64::from(p.client_y()))));
        w.raise();
        take_focus(w);
        pointer(w, &p, PointerPhase::Down);
    }));
    ls.push(on(&surface, "pointermove", |w, e| {
        if let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() {
            pointer(w, &p, PointerPhase::Move);
        }
    }));
    ls.push(on(&surface, "pointerup", |w, e| {
        if let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() {
            pointer(w, &p, PointerPhase::Up);
        }
    }));
    ls.push(on(&surface, "pointercancel", |w, e| {
        if let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() {
            pointer(w, &p, PointerPhase::Cancel);
        }
    }));
    ls.push(on(&surface, "pointerleave", |w, e| {
        if let Ok(p) = e.dyn_into::<web_sys::PointerEvent>()
            && p.pointer_type() == "mouse"
            && p.buttons() == 0
        {
            pointer(w, &p, PointerPhase::Cancel);
        }
    }));
    // A press must not move focus away from the hidden text field (or out of the window).
    ls.push(on(&surface, "mousedown", |_, e| e.prevent_default()));
    // The mouse's back and forward buttons reach the app (its router goes back once); the
    // browser's own history move on their release is cancelled so it is not a second one.
    for kind in ["mouseup", "auxclick"] {
        ls.push(on(&surface, kind, |_, e| {
            if e.dyn_ref::<web_sys::MouseEvent>().is_some_and(|m| matches!(m.button(), 3 | 4)) {
                e.prevent_default();
            }
        }));
    }
    // The browser's own menu shows unless the app took the press that opens it (its own
    // context menu, or a handler that uses the secondary button).
    ls.push(on(&surface, "contextmenu", |w, e| {
        if w.press_taken.get() {
            e.prevent_default();
        }
    }));
    ls.push(on(&surface, "wheel", |w, e| {
        if let Ok(we) = e.dyn_into::<web_sys::WheelEvent>() {
            wheel(w, &we);
        }
    }));
    ls.push(on(&surface, "gesturestart", |w, e| gesture(w, &e, PinchPhase::Start)));
    ls.push(on(&surface, "gesturechange", |w, e| gesture(w, &e, PinchPhase::Change)));
    ls.push(on(&surface, "gestureend", |w, e| gesture(w, &e, PinchPhase::End)));
    ls.push(on(&surface, "keydown", |w, e| {
        if let Ok(k) = e.dyn_into::<web_sys::KeyboardEvent>() {
            key(w, &k, true);
        }
    }));
    ls.push(on(&surface, "keyup", |w, e| {
        if let Ok(k) = e.dyn_into::<web_sys::KeyboardEvent>() {
            key(w, &k, false);
        }
    }));
    ls.push(on(&frame, "focusin", |w, _| set_focused(w, true)));
    ls.push(on(&frame, "focusout", |w, e| {
        let next = e.dyn_ref::<web_sys::FocusEvent>().and_then(web_sys::FocusEvent::related_target);
        let stays =
            next.and_then(|t| t.dyn_into::<web_sys::Node>().ok()).is_some_and(|n| w.frame.contains(Some(&n)));
        if !stays {
            set_focused(w, false);
        }
    }));
    ls.push(on(&surface, "dragenter", |w, e| {
        let Ok(de) = e.dyn_into::<web_sys::DragEvent>() else { return };
        de.prevent_default();
        // Browsers reveal what a drag carries only when it is dropped.
        let ev = DragEvent::Enter { position: Some(position(w, &de)), data: DragData::default() };
        post(HostEvent::Drag(w.id, ev));
    }));
    ls.push(on(&surface, "dragover", |w, e| {
        let Ok(de) = e.dyn_into::<web_sys::DragEvent>() else { return };
        de.prevent_default();
        if let Some(dt) = de.data_transfer() {
            dt.set_drop_effect(drop_effect(w.drop_effect.get()));
        }
        post(HostEvent::Drag(w.id, DragEvent::Over { position: position(w, &de) }));
    }));
    ls.push(on(&surface, "dragleave", |w, e| {
        let Ok(de) = e.dyn_into::<web_sys::DragEvent>() else { return };
        let inside = de
            .related_target()
            .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
            .is_some_and(|n| w.surface.contains(Some(&n)));
        if !inside {
            post(HostEvent::Drag(w.id, DragEvent::Leave));
        }
    }));
    ls.push(on(&surface, "drop", |w, e| {
        let Ok(de) = e.dyn_into::<web_sys::DragEvent>() else { return };
        de.prevent_default();
        let (files, text, uris) = drag_data(de.data_transfer());
        super::dialog::dropped(w.id, Some(position(w, &de)), files, text, uris);
    }));
    w.listeners.borrow_mut().extend(ls);
}

/// Listeners for the page as a whole: the physical Ctrl key, focus, visibility, the page
/// lifecycle, full screen and the visual viewport.
pub(crate) fn install_globals(d: &Rc<Driver>, win: &web_sys::Window, doc: &web_sys::Document) {
    let ctrl = |pressed: bool| {
        move |e: web_sys::Event| {
            if e.dyn_ref::<web_sys::KeyboardEvent>().is_some_and(|k| k.key() == "Control")
                && let Some(d) = driver()
            {
                d.state.borrow_mut().ctrl_down = pressed;
            }
        }
    };
    d.add_global(Listener::with_capture(win, "keydown", true, ctrl(true)));
    d.add_global(Listener::with_capture(win, "keyup", true, ctrl(false)));
    d.add_global(Listener::new(win, "blur", |_| {
        let Some(d) = driver() else { return };
        d.state.borrow_mut().ctrl_down = false;
        for w in d.windows() {
            set_focused(&w, false);
        }
    }));
    d.add_global(Listener::new(win, "focus", |_| {
        let Some(d) = driver() else { return };
        let active = dom::active_element();
        for w in d.windows() {
            let inside = active.as_ref().is_some_and(|a| w.frame.contains(Some(a)));
            set_focused(&w, inside);
        }
    }));
    d.add_global(Listener::new(doc, "visibilitychange", |_| {
        let Some(d) = driver() else { return };
        let hidden = dom::document().is_some_and(|doc| doc.hidden());
        for w in d.windows() {
            post(HostEvent::Occluded(w.id, hidden));
            if !hidden {
                w.request_frame();
            }
        }
    }));
    // The browser freezes background pages and keeps left pages in its back-forward cache:
    // the app lets go of its surfaces there and gets them back when the page returns.
    d.add_global(Listener::new(doc, "freeze", |_| post(HostEvent::Suspended)));
    d.add_global(Listener::new(doc, "resume", |_| post(HostEvent::Resumed)));
    d.add_global(Listener::new(win, "pagehide", |e| {
        if dom::get(&e, "persisted").as_bool() == Some(true) {
            post(HostEvent::Suspended);
        }
    }));
    d.add_global(Listener::new(win, "pageshow", |e| {
        if dom::get(&e, "persisted").as_bool() == Some(true) {
            post(HostEvent::Resumed);
        }
    }));
    d.add_global(Listener::new(doc, "fullscreenchange", |_| {
        if let Some(d) = driver() {
            for w in d.windows() {
                w.measure();
            }
        }
    }));
    let insets = |_: web_sys::Event| {
        if let Some(d) = driver() {
            for w in d.windows() {
                w.update_safe_area();
            }
        }
    };
    d.add_global(Listener::new(win, "resize", insets));
    if let Some(vv) = win.visual_viewport() {
        d.add_global(Listener::new(&vv, "resize", insets));
    }
}
