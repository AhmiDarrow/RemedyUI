//! The bridge in a browser page: the event loop on the page's callbacks, and the host.

mod aria;
mod clipboard;
mod dialog;
mod dom;
pub(crate) mod fonts;
mod gamepad;
pub(crate) mod history;
mod host;
mod input;
pub(crate) mod instance;
mod notify;
mod print;
mod settings;
mod share;
mod text;
mod window;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::host::{App, HostEvent, SystemSettings, TimerId, WindowId};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::{Mount, RunError, WebConfig};
use dom::Listener;
use window::Win;

pub(crate) use dialog::download;

/// How long one turn of cooperative background work may run before the page gets its thread
/// back: half a frame at 60 Hz, so input and frames keep their rate between turns.
const WORK_SLICE: Duration = Duration::from_millis(8);

/// Everything the host knows between events.
pub(crate) struct State {
    pub(crate) config: WebConfig,
    pub(crate) windows: BTreeMap<WindowId, Rc<Win>>,
    pub(crate) next_window: u64,
    /// Pending timers: the browser's handle and when each is due.
    pub(crate) timers: HashMap<TimerId, (i32, Duration)>,
    pub(crate) next_timer: u64,
    pub(crate) settings: SystemSettings,
    pub(crate) clip: clipboard::Clip,
    pub(crate) next_dialog: u64,
    pub(crate) next_share: u64,
    /// The app asked to end; the loop tears down after the current event.
    pub(crate) exiting: bool,
    /// Ctrl is physically held (a Ctrl+wheel without it is a touchpad pinch).
    pub(crate) ctrl_down: bool,
    /// The stacking order of the next window raised.
    pub(crate) next_z: i32,
}

/// The event loop: events from the page's callbacks go through one queue to the app, one at a
/// time, so a callback the app's own calls set off (a focus change, a copy) never runs the app
/// inside itself.
pub(crate) struct Driver {
    app: RefCell<Option<Box<dyn App>>>,
    queue: RefCell<VecDeque<HostEvent>>,
    dispatching: Cell<bool>,
    pub(crate) state: RefCell<State>,
    wake_queued: Cell<bool>,
    timer_cb: Closure<dyn FnMut(JsValue)>,
    wake_cb: Closure<dyn FnMut()>,
    work_port: web_sys::MessagePort,
    _work_cb: Closure<dyn FnMut(web_sys::MessageEvent)>,
    globals: RefCell<Vec<Listener>>,
    pub(crate) settings_watch: RefCell<Option<settings::Watch>>,
}

thread_local! {
    static DRIVER: RefCell<Option<Rc<Driver>>> = const { RefCell::new(None) };
}

/// The running loop, if any.
pub(crate) fn driver() -> Option<Rc<Driver>> {
    DRIVER.with_borrow(Clone::clone)
}

/// Queues `event` for the app and delivers it unless the app is already handling one.
pub(crate) fn post(event: HostEvent) {
    if let Some(d) = driver() {
        d.post(event);
    }
}

pub(crate) fn web_window() -> Result<web_sys::Window, RunError> {
    web_sys::window()
        .ok_or_else(|| RunError::new("there is no browser window (a worker cannot show windows)"))
}

/// The page's clock: milliseconds since the page started, as a duration.
pub(crate) fn now() -> Duration {
    let ms = web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now());
    Duration::from_secs_f64(ms.max(0.0) / 1000.0)
}

impl Driver {
    pub(crate) fn post(self: &Rc<Self>, event: HostEvent) {
        self.queue.borrow_mut().push_back(event);
        self.pump();
    }

    fn pump(self: &Rc<Self>) {
        if self.dispatching.replace(true) {
            return;
        }
        loop {
            let Some(event) = self.queue.borrow_mut().pop_front() else { break };
            let mut app = self.app.borrow_mut();
            let Some(app) = app.as_mut() else { break };
            let mut host = host::WebHost { d: self };
            app.event(&mut host, event);
            if self.state.borrow().exiting {
                break;
            }
        }
        self.dispatching.set(false);
        if self.state.borrow().exiting {
            self.teardown();
        }
    }

    /// Wakes the loop from anywhere on the page's thread: a `Wake` event after the current
    /// task.
    pub(crate) fn wake(&self) {
        if self.wake_queued.replace(true) {
            return;
        }
        if let Ok(w) = web_window() {
            w.queue_microtask(self.wake_cb.as_ref().unchecked_ref());
        }
    }

    /// Ends the loop: windows leave the page, listeners go, and the app is dropped.
    fn teardown(self: &Rc<Self>) {
        let windows: Vec<Rc<Win>> =
            std::mem::take(&mut self.state.borrow_mut().windows).into_values().collect();
        for w in windows {
            w.remove();
        }
        let timers: Vec<i32> = self.state.borrow_mut().timers.drain().map(|(_, (h, _))| h).collect();
        if let Ok(win) = web_window() {
            for h in timers {
                win.clear_timeout_with_handle(h);
            }
        }
        self.globals.borrow_mut().clear();
        self.settings_watch.borrow_mut().take();
        self.queue.borrow_mut().clear();
        let app = self.app.borrow_mut().take();
        drop(app);
        DRIVER.with_borrow_mut(|d| *d = None);
    }

    pub(crate) fn window(&self, id: WindowId) -> Option<Rc<Win>> {
        self.state.borrow().windows.get(&id).cloned()
    }

    pub(crate) fn windows(&self) -> Vec<Rc<Win>> {
        self.state.borrow().windows.values().cloned().collect()
    }

    pub(crate) fn add_global(&self, l: Listener) {
        self.globals.borrow_mut().push(l);
    }

    /// Arms the browser timer for `id`, due at `at` on the page's clock. Timers further away
    /// than a browser can wait in one go are armed again when that wait ends.
    pub(crate) fn arm_timer(&self, id: TimerId, at: Duration) -> Option<i32> {
        let win = web_window().ok()?;
        let ms = at.saturating_sub(now()).as_secs_f64() * 1000.0;
        let delay = ms.ceil().clamp(0.0, f64::from(i32::MAX)) as i32;
        win.set_timeout_with_callback_and_timeout_and_arguments_1(
            self.timer_cb.as_ref().unchecked_ref(),
            delay,
            &JsValue::from_f64(id.0 as f64),
        )
        .ok()
    }

    fn timer_fired(self: &Rc<Self>, id: TimerId) {
        let due = self.state.borrow_mut().timers.remove(&id).map(|(_, at)| at);
        let Some(at) = due else { return };
        if now() < at {
            // A wait longer than the browser's longest: keep waiting.
            if let Some(h) = self.arm_timer(id, at) {
                self.state.borrow_mut().timers.insert(id, (h, at));
            }
            return;
        }
        self.post(HostEvent::Timer(id));
    }
}

/// Starts the loop with `app` and returns; the browser runs it from then on.
pub(crate) fn start(config: WebConfig, app: Box<dyn App>) -> Result<(), RunError> {
    if driver().is_some() {
        return Err(RunError::new("a RemedyUI app already runs in this page"));
    }
    let win = web_window()?;
    let document = win.document().ok_or_else(|| RunError::new("the page has no document"))?;
    if let Mount::Element(id) = &config.mount
        && document.get_element_by_id(id).is_none()
    {
        return Err(RunError::new(format!("the page has no element with id `{id}` to show the app in")));
    }
    let channel =
        web_sys::MessageChannel::new().map_err(|e| RunError::new(format!("no message channel: {e:?}")))?;
    let page = settings::read(&win);
    let d = Rc::new_cyclic(|me: &Weak<Driver>| {
        let (m1, m2, m3) = (me.clone(), me.clone(), me.clone());
        let timer_cb = Closure::<dyn FnMut(JsValue)>::new(move |id: JsValue| {
            if let (Some(d), Some(id)) = (m1.upgrade(), id.as_f64()) {
                d.timer_fired(TimerId(id as u64));
            }
        });
        let wake_cb = Closure::<dyn FnMut()>::new(move || {
            if let Some(d) = m2.upgrade() {
                d.wake_queued.set(false);
                d.post(HostEvent::Wake);
            }
        });
        let work_cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |_| {
            if m3.upgrade().is_some() {
                // Asks for another turn itself when work is left.
                let _ = remedy_ui_core::exec::run_pending(WORK_SLICE);
            }
        });
        channel.port2().set_onmessage(Some(work_cb.as_ref().unchecked_ref()));
        Driver {
            app: RefCell::new(Some(app)),
            queue: RefCell::new(VecDeque::new()),
            dispatching: Cell::new(false),
            state: RefCell::new(State {
                config,
                windows: BTreeMap::new(),
                next_window: 1,
                timers: HashMap::new(),
                next_timer: 1,
                settings: crate::convert::settings(&page),
                clip: clipboard::Clip::default(),
                next_dialog: 1,
                next_share: 1,
                exiting: false,
                ctrl_down: false,
                next_z: 1,
            }),
            wake_queued: Cell::new(false),
            timer_cb,
            wake_cb,
            work_port: channel.port1(),
            _work_cb: work_cb,
            globals: RefCell::new(Vec::new()),
            settings_watch: RefCell::new(None),
        }
    });
    DRIVER.with_borrow_mut(|slot| *slot = Some(d.clone()));
    // Work the engine runs off the UI thread runs here in slices, each its own task.
    remedy_ui_core::exec::set_scheduler(Arc::new(|| {
        if let Some(d) = driver() {
            let _ = d.work_port.post_message(&JsValue::NULL);
        }
    }));
    // Work that waits for a time (a player's next picture) gets its turn from a browser timer.
    remedy_ui_core::exec::set_timer(Arc::new(|d: Duration| {
        let turn = Closure::once_into_js(|| {
            let _ = remedy_ui_core::exec::run_pending(WORK_SLICE);
        });
        if let Ok(w) = web_window() {
            let ms = (d.as_secs_f64() * 1000.0).ceil().clamp(0.0, f64::from(i32::MAX)) as i32;
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(turn.unchecked_ref(), ms);
        }
    }));
    input::install_globals(&d, &win, &document);
    clipboard::install(&d, &win, &document);
    gamepad::install(&d, &win);
    *d.settings_watch.borrow_mut() = Some(settings::watch(&win));
    let s = d.state.borrow().settings.clone();
    d.post(HostEvent::Settings(s));
    d.post(HostEvent::Resumed);
    Ok(())
}
