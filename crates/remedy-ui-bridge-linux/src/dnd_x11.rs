//! Dragging out to other apps on X11: the XDND protocol (version 5) from the source side, on a
//! connection of its own.
//!
//! The press that started the drag belongs to the app's window (winit's client holds its
//! implicit grab), so another client cannot grab the pointer: the drag follows it by asking
//! where it is, finds the XDND-aware window under it, talks XDND to that window from a hidden
//! window that owns `XdndSelection`, hands over the data when asked, and ends when the button
//! is released. It runs to the end inside the call, as `DoDragDrop` does on Windows.

use std::time::{Duration, Instant};

use remedy_ui_host::{DragData, DropEffect, Unsupported};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, KeyButMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::dnd::{Offer, offers, preferred};

/// The newest XDND version spoken.
const VERSION: u32 = 5;
/// How often the pointer is looked at.
const POLL: Duration = Duration::from_millis(8);

fn fail(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("dragging out on X11", e.to_string())
}

struct Atoms {
    aware: u32,
    proxy: u32,
    selection: u32,
    enter: u32,
    position: u32,
    status: u32,
    leave: u32,
    drop: u32,
    finished: u32,
    type_list: u32,
    copy: u32,
    moving: u32,
    link: u32,
    targets: u32,
    atom: u32,
    wm_pid: u32,
}

fn intern(c: &RustConnection, name: &str) -> Result<u32, Unsupported> {
    Ok(c.intern_atom(false, name.as_bytes()).map_err(fail)?.reply().map_err(fail)?.atom)
}

impl Atoms {
    fn new(c: &RustConnection) -> Result<Atoms, Unsupported> {
        Ok(Atoms {
            aware: intern(c, "XdndAware")?,
            proxy: intern(c, "XdndProxy")?,
            selection: intern(c, "XdndSelection")?,
            enter: intern(c, "XdndEnter")?,
            position: intern(c, "XdndPosition")?,
            status: intern(c, "XdndStatus")?,
            leave: intern(c, "XdndLeave")?,
            drop: intern(c, "XdndDrop")?,
            finished: intern(c, "XdndFinished")?,
            type_list: intern(c, "XdndTypeList")?,
            copy: intern(c, "XdndActionCopy")?,
            moving: intern(c, "XdndActionMove")?,
            link: intern(c, "XdndActionLink")?,
            targets: intern(c, "TARGETS")?,
            atom: u32::from(AtomEnum::ATOM),
            wm_pid: intern(c, "_NET_WM_PID")?,
        })
    }

    fn action(&self, e: DropEffect) -> u32 {
        match e {
            DropEffect::Move => self.moving,
            DropEffect::Link => self.link,
            _ => self.copy,
        }
    }

    fn effect(&self, action: u32) -> DropEffect {
        if action == self.moving {
            DropEffect::Move
        } else if action == self.link {
            DropEffect::Link
        } else if action == self.copy {
            DropEffect::Copy
        } else {
            DropEffect::None
        }
    }
}

/// A window under the pointer that takes XDND drops: where to send messages (its proxy, if
/// it names one), the window itself, and the version both sides speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    send_to: Window,
    window: Window,
    version: u32,
}

/// The first single 32-bit value of `prop` on `w`.
fn prop32(c: &RustConnection, w: Window, prop: u32, kind: impl Into<u32>) -> Option<u32> {
    let r = c.get_property(false, w, prop, kind.into(), 0, 1).ok()?.reply().ok()?;
    r.value32().and_then(|mut v| v.next())
}

/// The XDND-aware window under the pointer, from the top-level frame down; this process's
/// own windows are not targets (the app handles drops on itself while the drag is inside).
fn target_at(c: &RustConnection, a: &Atoms, root: Window) -> Option<Target> {
    let mut w = c.query_pointer(root).ok()?.reply().ok()?.child;
    let me = std::process::id();
    while w != x11rb::NONE {
        if let Some(v) = prop32(c, w, a.aware, AtomEnum::ATOM) {
            if prop32(c, w, a.wm_pid, AtomEnum::CARDINAL) == Some(me) {
                return None;
            }
            let proxy = prop32(c, w, a.proxy, AtomEnum::WINDOW).filter(|p| *p != x11rb::NONE);
            return Some(Target { send_to: proxy.unwrap_or(w), window: w, version: v.min(VERSION) });
        }
        w = c.query_pointer(w).ok()?.reply().ok()?.child;
    }
    None
}

fn send(c: &RustConnection, to: &Target, kind: u32, data: [u32; 5]) -> Result<(), Unsupported> {
    let ev = ClientMessageEvent::new(32, to.window, kind, data);
    c.send_event(false, to.send_to, EventMask::NO_EVENT, ev).map_err(fail)?;
    c.flush().map_err(fail)
}

/// Answers a request for the dragged data (or for what it can be read as).
fn serve(c: &RustConnection, a: &Atoms, req: &SelectionRequestEvent, offers: &[Offer], mimes: &[u32]) {
    let property = if req.property == x11rb::NONE { req.target } else { req.property };
    let wrote = if req.target == a.targets {
        let mut list = mimes.to_vec();
        list.push(a.targets);
        c.change_property32(PropMode::REPLACE, req.requestor, property, a.atom, &list).is_ok()
    } else {
        let wanted = mimes.iter().position(|m| *m == req.target).and_then(|i| offers.get(i));
        match wanted {
            Some(o) => {
                c.change_property8(PropMode::REPLACE, req.requestor, property, req.target, &o.bytes).is_ok()
            }
            None => false,
        }
    };
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if wrote { property } else { x11rb::NONE },
    };
    let _ = c.send_event(false, req.requestor, EventMask::NO_EVENT, notify);
    let _ = c.flush();
}

/// Drags `data` out until the button is released; what the drop did.
pub(crate) fn drag(data: &DragData, allowed: &[DropEffect]) -> Result<DropEffect, Unsupported> {
    drag_on(None, data, allowed)
}

/// [`drag`] on the X server `display` names (`None`: `DISPLAY`'s).
fn drag_on(
    display: Option<&str>,
    data: &DragData,
    allowed: &[DropEffect],
) -> Result<DropEffect, Unsupported> {
    let offers = offers(data);
    if offers.is_empty() {
        return Ok(DropEffect::None);
    }
    let want = preferred(allowed);
    let (c, screen) = x11rb::connect(display).map_err(fail)?;
    let root = c.setup().roots.get(screen).map(|s| s.root).ok_or_else(|| fail("no screen"))?;
    let a = Atoms::new(&c)?;
    let mimes: Vec<u32> = offers.iter().map(|o| intern(&c, o.mime)).collect::<Result<_, _>>()?;
    // The hidden window that speaks for the drag and owns the data.
    let me = c.generate_id().map_err(fail)?;
    c.create_window(0, me, root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())
        .map_err(fail)?;
    c.change_property32(PropMode::REPLACE, me, a.type_list, a.atom, &mimes).map_err(fail)?;
    c.set_selection_owner(me, a.selection, x11rb::CURRENT_TIME).map_err(fail)?;
    c.flush().map_err(fail)?;
    let action = a.action(want);
    let more_types = u32::from(mimes.len() > 3);
    let first = |i: usize| mimes.get(i).copied().unwrap_or(x11rb::NONE);

    let mut current: Option<Target> = None;
    let mut accepted = false;
    let mut answered = true;
    let mut last_at: Option<(i16, i16)> = None;
    let mut status_action = action;
    let result = loop {
        // What the target and data requests said.
        while let Some(ev) = c.poll_for_event().map_err(fail)? {
            match ev {
                Event::ClientMessage(m) if m.type_ == a.status => {
                    let d = m.data.as_data32();
                    if current.is_some_and(|t| d.first() == Some(&t.window)) {
                        accepted = d.get(1).is_some_and(|f| f & 1 == 1);
                        status_action = d.get(4).copied().filter(|x| *x != x11rb::NONE).unwrap_or(action);
                        answered = true;
                    }
                }
                Event::SelectionRequest(r) if r.selection == a.selection => {
                    serve(&c, &a, &r, &offers, &mimes)
                }
                _ => {}
            }
        }
        let p = c.query_pointer(root).map_err(fail)?.reply().map_err(fail)?;
        let held = p.mask.contains(KeyButMask::BUTTON1);
        let now_target = target_at(&c, &a, root);
        if now_target.map(|t| t.window) != current.map(|t| t.window) {
            if let Some(old) = current.take() {
                send(&c, &old, a.leave, [me, 0, 0, 0, 0])?;
            }
            if let Some(t) = now_target {
                send(&c, &t, a.enter, [me, (t.version << 24) | more_types, first(0), first(1), first(2)])?;
            }
            current = now_target;
            accepted = false;
            answered = true;
            last_at = None;
        }
        if !held {
            break current;
        }
        // A position when the pointer moved and the last one was answered.
        let at = (p.root_x, p.root_y);
        if let Some(t) = current
            && answered
            && last_at != Some(at)
        {
            let xy = (u32::from(at.0 as u16) << 16) | u32::from(at.1 as u16);
            send(&c, &t, a.position, [me, 0, xy, x11rb::CURRENT_TIME, action])?;
            answered = false;
            last_at = Some(at);
        }
        std::thread::sleep(POLL);
    };
    let effect = match result {
        Some(t) if accepted => {
            send(&c, &t, a.drop, [me, 0, x11rb::CURRENT_TIME, 0, 0])?;
            // Serve the data until the target says it is done.
            let start = Instant::now();
            let finish = crate::dnd::drop_waits().1;
            let mut effect = a.effect(status_action);
            loop {
                if start.elapsed() > finish {
                    break;
                }
                match c.poll_for_event().map_err(fail)? {
                    Some(Event::SelectionRequest(r)) if r.selection == a.selection => {
                        serve(&c, &a, &r, &offers, &mimes);
                    }
                    Some(Event::ClientMessage(m)) if m.type_ == a.finished => {
                        let d = m.data.as_data32();
                        let ok = d.get(1).is_some_and(|f| f & 1 == 1);
                        // Version 5 says what was done; older targets do not.
                        let done = d.get(2).copied().filter(|x| *x != x11rb::NONE).unwrap_or(status_action);
                        effect = if t.version >= 5 && !ok { DropEffect::None } else { a.effect(done) };
                        break;
                    }
                    Some(_) => {}
                    None => std::thread::sleep(POLL),
                }
            }
            effect
        }
        Some(t) => {
            send(&c, &t, a.leave, [me, 0, 0, 0, 0])?;
            DropEffect::None
        }
        None => DropEffect::None,
    };
    let _ = c.destroy_window(me);
    let _ = c.flush();
    Ok(if allowed.contains(&effect) { effect } else { DropEffect::None })
}

#[cfg(test)]
mod e2e;
