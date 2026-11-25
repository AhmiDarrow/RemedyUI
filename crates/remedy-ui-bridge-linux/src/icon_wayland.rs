//! Window icons on Wayland: the `xdg-toplevel-icon-v1` protocol, on winit's own connection
//! (winit 0.30 has none).
//!
//! Each icon is handed over as pixel buffers in shared memory: the icon's own size, and every
//! size the compositor said it prefers. Once set on the window the compositor keeps its own
//! copy, so the icon object and its buffers are released at once. Compositors without the
//! protocol show the icon of the app's desktop entry for every window, and the bridge says so.

use std::collections::HashSet;
use std::os::fd::AsFd;

use remedy_ui_core::host::{Unsupported, WindowIcon};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::{self, WlShmPool};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::{
    self, XdgToplevelIconManagerV1,
};
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::{self, XdgToplevelIconV1};

fn fail(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("window icons on Wayland", e.to_string())
}

#[derive(Default)]
struct State {
    /// Sizes the compositor prefers, in surface pixels.
    sizes: HashSet<i32>,
}

/// The icon manager on winit's connection.
pub(crate) struct Icons {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    manager: XdgToplevelIconManagerV1,
    shm: WlShm,
}

impl std::fmt::Debug for Icons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wayland window icons")
    }
}

impl Icons {
    /// Joins winit's connection; `Unsupported` when the compositor has no toplevel icons.
    pub(crate) fn new(conn: Connection) -> Result<Icons, Unsupported> {
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(fail)?;
        let qh = queue.handle();
        let manager: XdgToplevelIconManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| {
            fail("the compositor has no xdg-toplevel-icon; it shows the app's desktop-entry icon")
        })?;
        let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(fail)?;
        let mut state = State::default();
        // The preferred sizes arrive right after the manager is bound.
        queue.roundtrip(&mut state).map_err(fail)?;
        Ok(Icons { conn, queue, qh, state, manager, shm })
    }

    /// The connection, for objects winit made.
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Sets `icon` on the toplevel (winit's `xdg_toplevel` at `toplevel`), or resets it to the
    /// app's with `None`. `surface` is committed so the change shows now (the icon is part of
    /// the surface's double-buffered state).
    pub(crate) fn set(
        &mut self,
        toplevel: std::ptr::NonNull<std::ffi::c_void>,
        surface: &WlSurface,
        icon: Option<&WindowIcon>,
    ) -> Result<(), Unsupported> {
        let _ = self.queue.dispatch_pending(&mut self.state);
        let toplevel = crate::os::wayland_object::<XdgToplevel>(&self.conn, toplevel)
            .ok_or_else(|| fail("the window's toplevel is gone"))?;
        let Some(icon) = icon else {
            self.manager.set_icon(&toplevel, None);
            surface.commit();
            return self.conn.flush().map_err(fail);
        };
        let mut sizes: Vec<u32> = self.state.sizes.iter().filter_map(|s| u32::try_from(*s).ok()).collect();
        sizes.extend(icon.drawn_sizes());
        sizes.sort_unstable();
        sizes.dedup();
        let made = self.manager.create_icon(&self.qh, ());
        let mut buffers = Vec::with_capacity(sizes.len());
        for size in sizes.into_iter().filter(|s| *s > 0) {
            let scaled = icon.scaled(size);
            match self.buffer(&scaled) {
                Ok(b) => {
                    made.add_buffer(&b, 1);
                    buffers.push(b);
                }
                Err(e) => {
                    made.destroy();
                    for b in buffers {
                        b.destroy();
                    }
                    return Err(e);
                }
            }
        }
        self.manager.set_icon(&toplevel, Some(&made));
        // The compositor keeps its own copy once the icon is set; the buffers may go after
        // the icon object.
        made.destroy();
        for b in buffers {
            b.destroy();
        }
        surface.commit();
        self.conn.flush().map_err(fail)
    }

    /// A shared-memory buffer holding `icon` as premultiplied ARGB8888 (little-endian BGRA),
    /// the format every compositor takes.
    fn buffer(&self, icon: &WindowIcon) -> Result<WlBuffer, Unsupported> {
        let side = i32::try_from(icon.size()).map_err(fail)?;
        let bgra: Vec<u8> =
            icon.rgba().as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [b, g, r, a]).collect();
        let len = i32::try_from(bgra.len()).map_err(fail)?;
        let file = crate::os::sealed_memory("remedy-ui-window-icon", &bgra).map_err(fail)?;
        let pool = self.shm.create_pool(file.as_fd(), len, &self.qh, ());
        let buffer = pool.create_buffer(0, side, side, side * 4, wl_shm::Format::Argb8888, &self.qh, ());
        // The pool's memory stays mapped for the buffer; the pool object is no longer needed.
        pool.destroy();
        Ok(buffer)
    }
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

impl Dispatch<XdgToplevelIconManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgToplevelIconManagerV1,
        event: xdg_toplevel_icon_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel_icon_manager_v1::Event::IconSize { size } = event
            && size > 0
        {
            state.sizes.insert(size);
        }
    }
}

impl Dispatch<XdgToplevelIconV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgToplevelIconV1,
        _: xdg_toplevel_icon_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for State {
    fn event(_: &mut Self, _: &WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlShmPool, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(_: &mut Self, _: &WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
    }
}
