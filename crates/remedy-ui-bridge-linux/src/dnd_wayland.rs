//! Drag and drop on Wayland, both ways, on winit's own connection (winit 0.30 has none).
//!
//! The bridge opens winit's `wl_display` as a foreign connection with an event queue of its
//! own, binds the seat and the data device manager, and keeps a pointer object of its own: the
//! compositor sends every pointer object of the client the same button presses, which gives
//! the serial a drag must start with. Out: a `wl_data_source` offering the data, started on
//! the window's surface; how it ended arrives later. In: offers entering the app's windows
//! are read (`text/uri-list`, then text) and reported as drag events; the app's answer sets
//! the offer's actions and acceptance; a drop is finished once the app has taken it.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use remedy_ui_bridge_winit::winit::window::WindowId;
use remedy_ui_core::Point;
use remedy_ui_host::{DragData, DragEvent, DropEffect, Unsupported};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, ButtonState, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};

use crate::dnd::{Offer, offers, preferred};

/// How long reading a dropped offer may take.
const READ: Duration = Duration::from_secs(2);

fn fail(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("drag and drop on Wayland", e.to_string())
}

fn action(e: DropEffect) -> DndAction {
    match e {
        DropEffect::Copy => DndAction::Copy,
        DropEffect::Move => DndAction::Move,
        DropEffect::Link => DndAction::Ask,
        DropEffect::None => DndAction::empty(),
    }
}

fn effect(a: DndAction) -> DropEffect {
    if a.contains(DndAction::Move) {
        DropEffect::Move
    } else if a.contains(DndAction::Copy) {
        DropEffect::Copy
    } else if a.contains(DndAction::Ask) {
        DropEffect::Link
    } else {
        DropEffect::None
    }
}

/// A drag out under way.
struct Out {
    source: WlDataSource,
    offers: Vec<Offer>,
    window: WindowId,
    data: DragData,
    action: DndAction,
    dropped: bool,
}

/// A drag in over one of the app's windows.
struct In {
    offer: Option<WlDataOffer>,
    serial: u32,
    window: WindowId,
    mime: Option<String>,
    data: DragData,
    at: Point,
    dropped: bool,
}

#[derive(Default)]
struct State {
    press: Option<u32>,
    windows: HashMap<ObjectId, WindowId>,
    offered: HashMap<ObjectId, Vec<String>>,
    out: Option<Out>,
    incoming: Option<In>,
    events: Vec<(WindowId, DragEvent)>,
    ended: Vec<(WindowId, DropEffect)>,
}

/// The bridge's Wayland drag and drop.
pub(crate) struct Wayland {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    _pointer: WlPointer,
}

impl std::fmt::Debug for Wayland {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wayland drag and drop")
    }
}

impl Wayland {
    /// Joins winit's connection (`display` is its `wl_display`).
    pub(crate) fn new(conn: Connection) -> Result<Wayland, Unsupported> {
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(fail)?;
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=7, ()).map_err(fail)?;
        let manager: WlDataDeviceManager = globals.bind(&qh, 3..=3, ()).map_err(fail)?;
        let pointer = seat.get_pointer(&qh, ());
        let device = manager.get_data_device(&seat, &qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state).map_err(fail)?;
        Ok(Wayland { conn, queue, qh, state, manager, device, _pointer: pointer })
    }

    /// The connection, for objects winit made.
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Learns a window: drops on `surface` are `window`'s.
    pub(crate) fn add_window(&mut self, surface: &WlSurface, window: WindowId) {
        self.state.windows.insert(surface.id(), window);
    }

    /// Handles what the compositor sent since the last call (winit reads the socket).
    pub(crate) fn dispatch(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.state);
        let _ = self.conn.flush();
    }

    /// Starts dragging `data` out of `window` (from its `surface`); its end is reported by
    /// [`Wayland::take_ended`].
    pub(crate) fn start(
        &mut self,
        surface: &WlSurface,
        window: WindowId,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<(), Unsupported> {
        self.dispatch();
        let serial = self.state.press.ok_or_else(|| fail("no button press to start from"))?;
        let offers = offers(data);
        if offers.is_empty() {
            return Err(fail("nothing other apps can read"));
        }
        let source = self.manager.create_data_source(&self.qh, ());
        for o in &offers {
            source.offer(o.mime.to_owned());
        }
        let mut actions = DndAction::empty();
        for e in allowed {
            actions |= action(*e);
        }
        source.set_actions(actions);
        self.device.start_drag(Some(&source), surface, None, serial);
        self.conn.flush().map_err(fail)?;
        self.state.out = Some(Out {
            source,
            offers,
            window,
            data: data.clone(),
            action: action(preferred(allowed)),
            dropped: false,
        });
        Ok(())
    }

    /// The app's answer to the drag over `window`: what a drop there would do; after a drop,
    /// whether it was taken.
    pub(crate) fn set_effect(&mut self, window: WindowId, e: DropEffect) {
        let Some(inc) = self.state.incoming.as_mut().filter(|i| i.window == window) else { return };
        let Some(offer) = inc.offer.clone() else { return };
        if inc.dropped {
            if e != DropEffect::None {
                offer.set_actions(action(e), action(e));
                offer.finish();
            }
            offer.destroy();
            self.state.incoming = None;
        } else {
            let mime = (e != DropEffect::None).then(|| inc.mime.clone()).flatten();
            offer.accept(inc.serial, mime);
            offer.set_actions(action(e), action(e));
        }
        let _ = self.conn.flush();
    }

    /// Drags over the app's windows since the last call.
    pub(crate) fn take_events(&mut self) -> Vec<(WindowId, DragEvent)> {
        self.dispatch();
        std::mem::take(&mut self.state.events)
    }

    /// Drags out that ended since the last call.
    pub(crate) fn take_ended(&mut self) -> Vec<(WindowId, DropEffect)> {
        self.dispatch();
        std::mem::take(&mut self.state.ended)
    }
}

/// Files and links in a `text/uri-list` (RFC 2483: comments start with `#`).
pub(crate) fn parse_uri_list(text: &str) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut uris = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        match line.strip_prefix("file://") {
            // `file:///path` or `file://localhost/path`.
            Some(rest) => {
                let path = rest.strip_prefix("localhost").unwrap_or(rest);
                if path.starts_with('/') {
                    files.push(PathBuf::from(std::ffi::OsString::from(
                        <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&percent_decode(path)),
                    )));
                }
            }
            None => uris.push(line.to_owned()),
        }
    }
    (files, uris)
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |k: usize| b.get(k).and_then(|h| char::from(*h).to_digit(16));
        if c == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Reads `mime` from `offer` through a pipe, giving up after [`READ`].
fn read_offer(conn: &Connection, offer: &WlDataOffer, mime: &str) -> Option<Vec<u8>> {
    let (mut reader, writer) = std::io::pipe().ok()?;
    offer.receive(mime.to_owned(), std::os::fd::AsFd::as_fd(&writer));
    drop(writer);
    conn.flush().ok()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("remedy-ui drop".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = tx.send(reader.read_to_end(&mut buf).map(|_| buf));
        })
        .ok()?;
    rx.recv_timeout(READ).ok()?.ok()
}

/// What an offer with `mimes` carries: files and links from `text/uri-list`, else text.
fn read_data(conn: &Connection, offer: &WlDataOffer, mimes: &[String]) -> (DragData, Option<String>) {
    if mimes.iter().any(|m| m == "text/uri-list")
        && let Some(bytes) = read_offer(conn, offer, "text/uri-list")
    {
        let (files, uris) = parse_uri_list(&String::from_utf8_lossy(&bytes));
        return (DragData { files, uris, text: None }, Some("text/uri-list".into()));
    }
    for m in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"] {
        if mimes.iter().any(|x| x == m)
            && let Some(bytes) = read_offer(conn, offer, m)
        {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            return (DragData { text: Some(text), ..DragData::default() }, Some(m.into()));
        }
    }
    (DragData::default(), None)
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        s: &mut Self,
        _: &WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button { serial, state: WEnum::Value(ButtonState::Pressed), .. } = e {
            s.press = Some(serial);
        }
    }
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        s: &mut Self,
        offer: &WlDataOffer,
        e: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = e {
            s.offered.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        s: &mut Self,
        _: &WlDataDevice,
        e: wl_data_device::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_device::Event::DataOffer { id } => {
                s.offered.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Enter { serial, surface, x, y, id } => {
                let Some(&window) = s.windows.get(&surface.id()) else {
                    if let Some(o) = id {
                        o.destroy();
                    }
                    return;
                };
                let at = Point::new(x as f32, y as f32);
                let mimes = id.as_ref().and_then(|o| s.offered.remove(&o.id())).unwrap_or_default();
                // This app's own drag coming back: its data is known, and reading it here would
                // wait on this same thread to write it.
                let (data, mime) = match (&s.out, &id) {
                    (Some(out), _) => (out.data.clone(), out.offers.first().map(|o| o.mime.to_owned())),
                    (None, Some(o)) => read_data(conn, o, &mimes),
                    (None, None) => (DragData::default(), None),
                };
                s.incoming =
                    Some(In { offer: id, serial, window, mime, data: data.clone(), at, dropped: false });
                s.events.push((window, DragEvent::Enter { position: Some(at), data }));
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(inc) = s.incoming.as_mut() {
                    inc.at = Point::new(x as f32, y as f32);
                    s.events.push((inc.window, DragEvent::Over { position: inc.at }));
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(inc) = s.incoming.take()
                    && !inc.dropped
                {
                    if let Some(o) = inc.offer {
                        o.destroy();
                    }
                    s.events.push((inc.window, DragEvent::Leave));
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(inc) = s.incoming.as_mut() {
                    inc.dropped = true;
                    s.events.push((
                        inc.window,
                        DragEvent::Drop { position: Some(inc.at), data: inc.data.clone() },
                    ));
                }
            }
            // The clipboard is arboard's; this device only drags.
            wl_data_device::Event::Selection { id: Some(o) } => {
                s.offered.remove(&o.id());
                o.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        s: &mut Self,
        source: &WlDataSource,
        e: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(out) = s.out.as_mut().filter(|o| o.source == *source) else { return };
        match e {
            wl_data_source::Event::Send { mime_type, fd } => {
                if let Some(o) = out.offers.iter().find(|o| o.mime == mime_type) {
                    let mut f = std::fs::File::from(fd);
                    let _ = std::io::Write::write_all(&mut f, &o.bytes);
                }
            }
            wl_data_source::Event::Action { dnd_action: WEnum::Value(a) } => out.action = a,
            wl_data_source::Event::DndDropPerformed => out.dropped = true,
            wl_data_source::Event::DndFinished => {
                let done = effect(out.action);
                let window = out.window;
                source.destroy();
                s.out = None;
                s.ended.push((window, done));
            }
            wl_data_source::Event::Cancelled => {
                let window = out.window;
                source.destroy();
                s.out = None;
                s.ended.push((window, DropEffect::None));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::parse_uri_list;

    #[test]
    fn uri_lists_give_files_and_links() {
        let (files, uris) = parse_uri_list(
            "# from a file manager\r\nfile:///home/sam/beach%20day.jpg\r\nfile://localhost/tmp/%C3%A4.txt\r\nhttps://example.com/x\r\n",
        );
        assert_eq!(files, [PathBuf::from("/home/sam/beach day.jpg"), PathBuf::from("/tmp/ä.txt")]);
        assert_eq!(uris, ["https://example.com/x"]);
        let (files, _) = parse_uri_list("file://otherhost/share/a.txt\n");
        assert!(files.is_empty(), "another host's file is not a local path");
    }
}
