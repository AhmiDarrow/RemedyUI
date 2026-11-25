//! Gamepads (the Gamepad API). With the standard mapping the d-pad and left stick arrive as
//! arrow keys, A as Enter and B as Back, so spatial navigation and the router work with a
//! controller as they do with a TV remote; a held direction repeats as a held arrow key does
//! (`GamepadKeys`, tuned by `Host::set_gamepad_tuning`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::host::HostEvent;
use remedy_ui_core::input::{GamepadKeys, GamepadTuning, PadButtons, PadState};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::dom::Listener;
use super::{Driver, driver};
use crate::convert;

/// How often connected pads are read, in milliseconds: the Gamepad API has no button events,
/// and a frame at 60 Hz keeps a press from feeling late. A sampling rate, not a cap.
const POLL_MS: i32 = 16;

thread_local! {
    /// The running poll's interval handle, and the callback it runs (kept for the page's life:
    /// the poll stops itself, and a closure may not be dropped while it runs).
    static POLL: Cell<Option<i32>> = const { Cell::new(None) };
    static CALLBACK: RefCell<Option<Closure<dyn FnMut()>>> = const { RefCell::new(None) };
    /// What the pads hold, turned into keys with repeats.
    static KEYS: RefCell<GamepadKeys> = RefCell::new(GamepadKeys::new());
    /// Which slots had a pad at the last poll.
    static CONNECTED: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// Starts reading pads when one connects (browsers announce one at its first press).
pub(crate) fn install(d: &Rc<Driver>, win: &web_sys::Window) {
    d.add_global(Listener::new(win, "gamepadconnected", |_| start()));
    // A pad that connected under an app that ran here before is not announced again; with
    // none, the first poll stops itself.
    start();
}

/// The app's tuning ([`remedy_ui_core::host::Host::set_gamepad_tuning`]).
pub(crate) fn set_tuning(tuning: GamepadTuning) {
    KEYS.with_borrow_mut(|k| k.set_tuning(tuning));
}

fn start() {
    if POLL.get().is_some() {
        return;
    }
    let Some(win) = web_sys::window() else { return };
    CALLBACK.with_borrow_mut(|cb| {
        let cb = cb.get_or_insert_with(|| Closure::<dyn FnMut()>::new(poll));
        if let Ok(h) =
            win.set_interval_with_callback_and_timeout_and_arguments_0(cb.as_ref().unchecked_ref(), POLL_MS)
        {
            POLL.set(Some(h));
        }
    });
}

/// A standard-mapping pad's reading.
fn reading(pad: &web_sys::Gamepad) -> PadState {
    let buttons = pad.buttons();
    let mut held = PadButtons::NONE;
    for b in convert::GAMEPAD_BUTTONS {
        let pressed = u32::try_from(b)
            .ok()
            .and_then(|n| buttons.get(n).dyn_into::<web_sys::GamepadButton>().ok())
            .is_some_and(|g| g.pressed());
        if pressed && let Some(p) = convert::gamepad_button(b) {
            held |= p;
        }
    }
    let axes = pad.axes();
    let axis = |i| axes.get(i).as_f64().unwrap_or(0.0) as f32;
    // The standard mapping's axes 0 and 1 are the left stick, y growing downward.
    PadState { buttons: held, stick: [axis(0), axis(1)] }
}

fn poll() {
    let Some(win) = web_sys::window() else { return };
    let Some(d) = driver() else {
        // The app ended: nobody takes the keys, and the next app starts with nothing held.
        if let Some(h) = POLL.take() {
            win.clear_interval_with_handle(h);
        }
        KEYS.with_borrow_mut(|k| *k = GamepadKeys::new());
        CONNECTED.with_borrow_mut(Vec::clear);
        return;
    };
    let now =
        win.performance().map_or(Duration::ZERO, |p| Duration::from_secs_f64(p.now().max(0.0) / 1000.0));
    let pads = win.navigator().get_gamepads().unwrap_or_default();
    let mut keys = Vec::new();
    let mut any = false;
    KEYS.with_borrow_mut(|k| {
        {
            let s = d.state.borrow();
            k.set_system_repeat(s.settings.key_repeat_delay, s.settings.key_repeat_interval);
        }
        CONNECTED.with_borrow_mut(|connected| {
            let n = (pads.length() as usize).max(connected.len());
            connected.resize(n, false);
            for (i, was) in connected.iter_mut().enumerate() {
                // Empty slots are null; pads without the standard mapping have no known d-pad.
                let pad = u32::try_from(i)
                    .ok()
                    .and_then(|i| pads.get(i).dyn_into::<web_sys::Gamepad>().ok())
                    .filter(|p| p.connected() && p.mapping() == web_sys::GamepadMappingType::Standard);
                let state = pad.as_ref().map(reading);
                any |= pad.is_some();
                if state.is_some() || *was {
                    keys.extend(k.update(i, state.as_ref(), now));
                }
                *was = state.is_some();
            }
        });
        keys.extend(k.tick(now));
    });
    if !any {
        // Every pad is gone: stop until one connects again.
        if let Some(h) = POLL.take() {
            win.clear_interval_with_handle(h);
        }
    }
    let windows = d.windows();
    let Some(w) = windows.iter().find(|w| w.focused.get()).or(windows.first()) else { return };
    for k in keys {
        d.post(HostEvent::Key(w.id, k));
    }
}
