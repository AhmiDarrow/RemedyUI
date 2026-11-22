//! Gamepads (the Gamepad API). With the standard mapping the d-pad, A and B arrive as arrow
//! keys, Enter and Back, so spatial navigation and the router work with a controller as they
//! do with a TV remote.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_host::HostEvent;
use remedy_ui_input::KeyEvent;
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
    /// The buttons each pad had down at the last poll, a bit per standard button.
    static DOWN: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Starts reading pads when one connects (browsers announce one at its first press).
pub(crate) fn install(d: &Rc<Driver>, win: &web_sys::Window) {
    d.add_global(Listener::new(win, "gamepadconnected", |_| start()));
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

fn poll() {
    let Some(win) = web_sys::window() else { return };
    let pads = win.navigator().get_gamepads().unwrap_or_default();
    let mut any = false;
    let mut keys = Vec::new();
    DOWN.with_borrow_mut(|down| {
        for (i, pad) in pads.iter().enumerate() {
            // Empty slots are null.
            let Ok(pad) = pad.dyn_into::<web_sys::Gamepad>() else { continue };
            if !pad.connected() {
                continue;
            }
            any = true;
            if down.len() <= i {
                down.resize(i + 1, 0);
            }
            if pad.mapping() != web_sys::GamepadMappingType::Standard {
                continue;
            }
            let buttons = pad.buttons();
            let mut now = 0u64;
            for b in convert::GAMEPAD_BUTTONS {
                let pressed = u32::try_from(b)
                    .ok()
                    .and_then(|n| buttons.get(n).dyn_into::<web_sys::GamepadButton>().ok())
                    .is_some_and(|g| g.pressed());
                let bit = 1u64 << b;
                if pressed {
                    now |= bit;
                }
                if pressed != (down.get(i).is_some_and(|d| d & bit != 0))
                    && let Some(k) = convert::gamepad_key(b)
                {
                    keys.push(KeyEvent::press(k).with_pressed(pressed));
                }
            }
            if let Some(d) = down.get_mut(i) {
                *d = now;
            }
        }
        if !any {
            down.clear();
        }
    });
    if !any {
        // Every pad is gone: stop until one connects again.
        if let Some(h) = POLL.take() {
            win.clear_interval_with_handle(h);
        }
        return;
    }
    let Some(d) = driver() else { return };
    let windows = d.windows();
    let Some(w) = windows.iter().find(|w| w.focused.get()).or(windows.first()) else { return };
    for k in keys {
        d.post(HostEvent::Key(w.id, k));
    }
}
