//! The tray on X11 desktops without a StatusNotifier host (older panels, plain window
//! managers with a system tray such as stalonetray or trayer): the freedesktop System Tray
//! Protocol over XEmbed. The icon is a small window the tray docks; pressing it reports
//! [`TrayEvent::Activate`], and the secondary button opens the menu as a pop-up window
//! (keyboard: Up, Down, Enter, Escape) whose choices report [`TrayEvent::Menu`].
//!
//! It runs on a thread of its own with its own X connection, so it works whichever way the
//! app's windows reach the display (Wayland through XWayland too).

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use remedy_ui_host::{TrayEvent, TrayIcon, TrayItem, Unsupported};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ClientMessageEvent, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask,
    Font, Gcontext, GrabMode, ImageFormat, PropMode, Screen, Visualid, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const REQUEST_DOCK: u32 = 0;
/// The icon's side before the tray says otherwise.
const ICON: u16 = 24;
/// A menu row's height and the menu's side padding, in pixels.
const ROW: u16 = 22;
const PAD: i16 = 10;

/// What the app sends the tray thread.
pub(crate) enum Cmd {
    Set(TrayIcon),
    Quit,
}

/// A running XEmbed tray icon.
pub(crate) struct XTray {
    tx: Sender<Cmd>,
}

impl XTray {
    /// Docks `tray` in the X11 system tray; `report` gets what the person does with it.
    pub(crate) fn start(
        tray: &TrayIcon,
        report: impl Fn(TrayEvent) + Send + 'static,
    ) -> Result<XTray, Unsupported> {
        let display = std::env::var("DISPLAY").ok().filter(|d| !d.is_empty());
        XTray::start_on(display.as_deref(), tray, report)
    }

    /// [`XTray::start`] on the X display named `display`.
    fn start_on(
        display: Option<&str>,
        tray: &TrayIcon,
        report: impl Fn(TrayEvent) + Send + 'static,
    ) -> Result<XTray, Unsupported> {
        let err = |why: String| Unsupported::new("the tray", why);
        let display = display.ok_or_else(|| err("no X11 display for a system tray".into()))?;
        let (conn, screen_no) =
            x11rb::connect(Some(display)).map_err(|e| err(format!("the X server did not answer: {e}")))?;
        let manager =
            manager(&conn, screen_no).ok_or_else(|| err("the desktop has no system tray".into()))?;
        let (tx, rx) = channel();
        let first = tray.clone();
        std::thread::Builder::new()
            .name("remedy-ui-xtray".into())
            .spawn(move || {
                if let Err(e) = run(&conn, screen_no, manager, first, &rx, &report) {
                    eprintln!("remedy-ui: the X11 tray stopped: {e}");
                }
            })
            .map_err(|e| err(e.to_string()))?;
        Ok(XTray { tx })
    }

    /// Shows `tray` instead (its icon, tooltip and menu).
    pub(crate) fn set(&self, tray: &TrayIcon) {
        let _ = self.tx.send(Cmd::Set(tray.clone()));
    }
}

impl Drop for XTray {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

/// The system tray's manager window, if a tray runs on this screen.
fn manager(conn: &RustConnection, screen_no: usize) -> Option<Window> {
    let name = format!("_NET_SYSTEM_TRAY_S{screen_no}");
    let atom = conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom;
    let owner = conn.get_selection_owner(atom).ok()?.reply().ok()?.owner;
    (owner != x11rb::NONE).then_some(owner)
}

fn atom(conn: &RustConnection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes()).ok().and_then(|c| c.reply().ok()).map_or(0, |r| r.atom)
}

/// The visual the tray asks icons to use (`_NET_SYSTEM_TRAY_VISUAL`, usually 32-bit ARGB so
/// icons can be see-through), with its depth; the screen's own otherwise.
fn tray_visual(conn: &RustConnection, screen: &Screen, manager: Window) -> (Visualid, u8) {
    let prop = atom(conn, "_NET_SYSTEM_TRAY_VISUAL");
    let asked = conn
        .get_property(false, manager, prop, AtomEnum::VISUALID, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().and_then(|mut v| v.next()));
    if let Some(v) = asked {
        for d in &screen.allowed_depths {
            if d.visuals.iter().any(|vt| vt.visual_id == v) {
                return (v, d.depth);
            }
        }
    }
    (screen.root_visual, screen.root_depth)
}

#[allow(clippy::too_many_lines)]
fn run(
    conn: &RustConnection,
    screen_no: usize,
    manager: Window,
    first: TrayIcon,
    rx: &Receiver<Cmd>,
    report: &dyn Fn(TrayEvent),
) -> Result<(), Box<dyn std::error::Error>> {
    let screen = conn.setup().roots.get(screen_no).cloned().ok_or("the X server has no such screen")?;
    let (visual, depth) = tray_visual(conn, &screen, manager);
    let colormap = if visual == screen.root_visual {
        screen.default_colormap
    } else {
        let cm = conn.generate_id()?;
        conn.create_colormap(x11rb::protocol::xproto::ColormapAlloc::NONE, cm, screen.root, visual)?;
        cm
    };
    let win = conn.generate_id()?;
    conn.create_window(
        depth,
        win,
        screen.root,
        0,
        0,
        ICON,
        ICON,
        0,
        WindowClass::INPUT_OUTPUT,
        visual,
        &CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY | EventMask::BUTTON_PRESS),
    )?;
    let mut tray = first;
    set_name(conn, win, &tray.tooltip)?;
    // XEmbed info: version 0, mapped.
    let xembed = atom(conn, "_XEMBED_INFO");
    conn.change_property32(PropMode::REPLACE, win, xembed, xembed, &[0, 1])?;
    let opcode = atom(conn, "_NET_SYSTEM_TRAY_OPCODE");
    let dock = ClientMessageEvent::new(32, manager, opcode, [x11rb::CURRENT_TIME, REQUEST_DOCK, win, 0, 0]);
    conn.send_event(false, manager, EventMask::NO_EVENT, dock)?;
    let gc = conn.generate_id()?;
    conn.create_gc(gc, win, &CreateGCAux::new())?;
    conn.flush()?;
    let mut size = (ICON, ICON);
    let mut menu: Option<Menu> = None;
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Set(t)) => {
                    tray = t;
                    set_name(conn, win, &tray.tooltip)?;
                    draw_icon(conn, win, gc, &tray, size, depth)?;
                }
                Ok(Cmd::Quit) | Err(TryRecvError::Disconnected) => {
                    conn.destroy_window(win)?;
                    conn.flush()?;
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        let mut any = false;
        while let Some(event) = conn.poll_for_event()? {
            any = true;
            match event {
                Event::Expose(e) if e.window == win => draw_icon(conn, win, gc, &tray, size, depth)?,
                Event::ConfigureNotify(e) if e.window == win => {
                    size = (e.width.max(1), e.height.max(1));
                    draw_icon(conn, win, gc, &tray, size, depth)?;
                }
                Event::ButtonPress(e) if e.event == win => match e.detail {
                    1 => report(TrayEvent::Activate),
                    3 if !tray.menu.is_empty() => menu = Some(Menu::open(conn, &screen, &tray.menu, e)?),
                    _ => {}
                },
                ref e => {
                    if let Some(m) = &mut menu {
                        match m.handle(conn, e)? {
                            MenuOutcome::Open => {}
                            MenuOutcome::Chosen(id) => {
                                m.close(conn)?;
                                menu = None;
                                report(TrayEvent::Menu(id));
                            }
                            MenuOutcome::Closed => {
                                m.close(conn)?;
                                menu = None;
                            }
                        }
                    }
                }
            }
        }
        conn.flush()?;
        if !any {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

fn set_name(conn: &RustConnection, win: Window, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let utf8 = atom(conn, "UTF8_STRING");
    let net_name = atom(conn, "_NET_WM_NAME");
    conn.change_property8(PropMode::REPLACE, win, net_name, utf8, name.as_bytes())?;
    conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, name.as_bytes())?;
    Ok(())
}

/// The icon at the window's size ([`TrayIcon::image_at`]: a drawn size as it is, pixel art by
/// nearest pixel), as the window's pixels: premultiplied ARGB in a 32-bit visual, over a
/// neutral grey in a 24-bit one.
fn draw_icon(
    conn: &RustConnection,
    win: Window,
    gc: Gcontext,
    tray: &TrayIcon,
    (w, h): (u16, u16),
    depth: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let side = u32::from(w.min(h));
    let image = tray.image_at(side);
    let src = image.size.max(1);
    let (ox, oy) = ((u32::from(w) - side) / 2, (u32::from(h) - side) / 2);
    let mut px = vec![0u8; usize::from(w) * usize::from(h) * 4];
    for y in 0..side {
        for x in 0..side {
            let (sx, sy) = (x * src / side, y * src / side);
            let i = ((sy * src + sx) * 4) as usize;
            let Some(&[r, g, b, a]) = image.rgba.get(i..i + 4).and_then(|s| <&[u8; 4]>::try_from(s).ok())
            else {
                continue;
            };
            let o = (((oy + y) * u32::from(w) + ox + x) * 4) as usize;
            let (r, g, b, a) = if depth == 32 {
                (r, g, b, a)
            } else {
                // Premultiplied over grey.
                let bg = 64u16 * (255 - u16::from(a)) / 255;
                (r.saturating_add(bg as u8), g.saturating_add(bg as u8), b.saturating_add(bg as u8), 255)
            };
            // Little-endian 32-bit pixels: blue, green, red, alpha.
            if let Some(d) = px.get_mut(o..o + 4) {
                d.copy_from_slice(&[b, g, r, a]);
            }
        }
    }
    conn.put_image(ImageFormat::Z_PIXMAP, win, gc, w, h, 0, 0, 0, depth, &px)?;
    Ok(())
}

const XK_ESCAPE: u32 = 0xff1b;
const XK_RETURN: u32 = 0xff0d;
const XK_KP_ENTER: u32 = 0xff8d;
const XK_UP: u32 = 0xff52;
const XK_DOWN: u32 = 0xff54;

/// The keysym the keyboard's layout puts on `keycode` (unshifted).
fn keysym(conn: &RustConnection, keycode: u8) -> u32 {
    conn.get_keyboard_mapping(keycode, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.keysyms.first().copied())
        .unwrap_or(0)
}

/// The menu's rows: a submenu is a header row (not chosen itself) with its entries indented
/// under it.
// shortcut: submenus open inline rather than as cascading pop-ups; this tray is the fallback for
// panels without StatusNotifier, so it stays one window with one grab.
fn rows(items: &[TrayItem], depth: i16) -> Vec<(i16, TrayItem)> {
    let mut out = Vec::new();
    for it in items {
        match it {
            TrayItem::Submenu { label, items } => {
                out.push((depth, TrayItem::submenu(label.clone(), Vec::new())));
                out.extend(rows(items, depth + 1));
            }
            other => out.push((depth, other.clone())),
        }
    }
    out
}

/// The tray's menu as a pop-up window.
struct Menu {
    win: Window,
    gc: Gcontext,
    font: Font,
    /// Rows, indented by depth: a submenu is a header row with its entries under it.
    items: Vec<(i16, TrayItem)>,
    hover: Option<usize>,
    width: u16,
}

enum MenuOutcome {
    Open,
    Chosen(u32),
    Closed,
}

impl Menu {
    fn open(
        conn: &RustConnection,
        screen: &Screen,
        items: &[TrayItem],
        at: ButtonPressEvent,
    ) -> Result<Menu, Box<dyn std::error::Error>> {
        let font = conn.generate_id()?;
        // A Unicode core font when the server has one, else the always-present "fixed".
        if conn.open_font(font, b"-misc-fixed-medium-r-normal--13-*-*-*-*-*-iso10646-1")?.check().is_err() {
            conn.open_font(font, b"fixed")?.check()?;
        }
        let items = rows(items, 0);
        // The check mark's column and the indent, then the label.
        let longest = items
            .iter()
            .map(|(depth, i)| i.label().chars().count() + 2 + 2 * *depth as usize)
            .max()
            .unwrap_or(8);
        let width = (longest as u16).saturating_mul(7).saturating_add(2 * PAD as u16).max(120);
        let height = (items.len() as u16).saturating_mul(ROW).max(ROW);
        // Above or below the icon, inside the screen.
        let x = at.root_x.min(screen.width_in_pixels as i16 - width as i16).max(0);
        let y = if at.root_y > screen.height_in_pixels as i16 / 2 {
            at.root_y - height as i16
        } else {
            at.root_y
        };
        let win = conn.generate_id()?;
        conn.create_window(
            screen.root_depth,
            win,
            screen.root,
            x,
            y.max(0),
            width,
            height,
            1,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0x00f4_f4f4)
                .border_pixel(0x0080_8080)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::POINTER_MOTION
                        | EventMask::KEY_PRESS,
                ),
        )?;
        let gc = conn.generate_id()?;
        conn.create_gc(
            gc,
            win,
            &CreateGCAux::new().font(font).foreground(0x0020_2020).background(0x00f4_f4f4),
        )?;
        conn.map_window(win)?;
        // Every click and key comes here until the menu closes.
        let _ = conn.grab_pointer(
            false,
            win,
            EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )?;
        let _ = conn.grab_keyboard(false, win, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)?;
        let m = Menu { win, gc, font, items, hover: None, width };
        m.draw(conn)?;
        Ok(m)
    }

    fn draw(&self, conn: &RustConnection) -> Result<(), Box<dyn std::error::Error>> {
        conn.clear_area(false, self.win, 0, 0, 0, 0)?;
        for (i, (depth, item)) in self.items.iter().enumerate() {
            let top = i as i16 * ROW as i16;
            match item {
                TrayItem::Separator => {
                    let mid = top + ROW as i16 / 2;
                    conn.poly_line(
                        x11rb::protocol::xproto::CoordMode::ORIGIN,
                        self.win,
                        self.gc,
                        &[
                            x11rb::protocol::xproto::Point { x: PAD, y: mid },
                            x11rb::protocol::xproto::Point { x: self.width as i16 - PAD, y: mid },
                        ],
                    )?;
                }
                _ => {
                    let enabled = item.chosen_id().is_some();
                    let hot = self.hover == Some(i) && enabled;
                    // A submenu's header reads as normal text, though it is not chosen itself.
                    let (fg, bg) = match (hot, enabled || matches!(item, TrayItem::Submenu { .. })) {
                        (true, _) => (0x00ff_ffff, 0x0035_6ac0),
                        (false, true) => (0x0020_2020, 0x00f4_f4f4),
                        (false, false) => (0x0090_9090, 0x00f4_f4f4),
                    };
                    conn.change_gc(self.gc, &x11rb::protocol::xproto::ChangeGCAux::new().foreground(bg))?;
                    conn.poly_fill_rectangle(
                        self.win,
                        self.gc,
                        &[x11rb::protocol::xproto::Rectangle {
                            x: 0,
                            y: top,
                            width: self.width,
                            height: ROW,
                        }],
                    )?;
                    conn.change_gc(
                        self.gc,
                        &x11rb::protocol::xproto::ChangeGCAux::new().foreground(fg).background(bg),
                    )?;
                    let mark = match item {
                        TrayItem::Check { checked: true, .. } => '\u{2713}',
                        _ => ' ',
                    };
                    let text: Vec<x11rb::protocol::xproto::Char2b> = [mark, ' ']
                        .into_iter()
                        .chain(std::iter::repeat_n(' ', 2 * *depth as usize))
                        .chain(item.label().chars())
                        .map(|c| {
                            let u = u32::from(c).min(0xffff) as u16;
                            x11rb::protocol::xproto::Char2b { byte1: (u >> 8) as u8, byte2: u as u8 }
                        })
                        .collect();
                    let base = top + ROW as i16 - 7;
                    conn.image_text16(self.win, self.gc, PAD, base, &text)?;
                    if matches!(item, TrayItem::Item { default: true, .. }) {
                        // Bold for the default: core fonts have no bold face, so the text again a
                        // pixel to the right.
                        conn.image_text16(self.win, self.gc, PAD + 1, base, &text)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn row_at(&self, y: i16) -> Option<usize> {
        (y >= 0).then(|| (y / ROW as i16) as usize).filter(|i| *i < self.items.len())
    }

    fn enabled(&self, i: usize) -> Option<u32> {
        self.items.get(i)?.1.chosen_id()
    }

    fn handle(
        &mut self,
        conn: &RustConnection,
        e: &Event,
    ) -> Result<MenuOutcome, Box<dyn std::error::Error>> {
        Ok(match e {
            Event::Expose(e) if e.window == self.win => {
                self.draw(conn)?;
                MenuOutcome::Open
            }
            Event::MotionNotify(e) => {
                let row = (e.event == self.win && e.event_x >= 0 && e.event_x < self.width as i16)
                    .then(|| self.row_at(e.event_y))
                    .flatten();
                if row != self.hover {
                    self.hover = row;
                    self.draw(conn)?;
                }
                MenuOutcome::Open
            }
            Event::ButtonPress(e) => {
                let inside = e.event == self.win && e.event_x >= 0 && e.event_x < self.width as i16;
                match inside.then(|| self.row_at(e.event_y)).flatten() {
                    Some(i) => self.enabled(i).map_or(MenuOutcome::Open, MenuOutcome::Chosen),
                    None => MenuOutcome::Closed,
                }
            }
            Event::KeyPress(k) => match keysym(conn, k.detail) {
                XK_ESCAPE => MenuOutcome::Closed,
                sym @ (XK_UP | XK_DOWN) => {
                    let up = sym == XK_UP;
                    let n = self.items.len();
                    let step = |i: usize| if up { (i + n - 1) % n } else { (i + 1) % n };
                    let mut i = self.hover.map_or(if up { n - 1 } else { 0 }, step);
                    for _ in 0..n {
                        if self.enabled(i).is_some() {
                            break;
                        }
                        i = step(i);
                    }
                    self.hover = Some(i);
                    self.draw(conn)?;
                    MenuOutcome::Open
                }
                XK_RETURN | XK_KP_ENTER => {
                    self.hover.and_then(|i| self.enabled(i)).map_or(MenuOutcome::Open, MenuOutcome::Chosen)
                }
                _ => MenuOutcome::Open,
            },
            _ => MenuOutcome::Open,
        })
    }

    fn close(&self, conn: &RustConnection) -> Result<(), Box<dyn std::error::Error>> {
        conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
        conn.ungrab_keyboard(x11rb::CURRENT_TIME)?;
        conn.destroy_window(self.win)?;
        conn.free_gc(self.gc)?;
        conn.close_font(self.font)?;
        conn.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tray() -> TrayIcon {
        TrayIcon::new(
            "App",
            remedy_ui_host::TrayImage { size: 2, rgba: vec![255; 16] },
            vec![
                TrayItem::Item { id: 1, label: "Open".into(), enabled: true, default: true },
                TrayItem::Separator,
                TrayItem::Item { id: 2, label: "Room".into(), enabled: false, default: false },
                TrayItem::item(3, "Quit"),
                TrayItem::submenu("Server", vec![TrayItem::check(4, "Logs", true)]),
            ],
        )
    }

    #[test]
    fn without_a_display_it_says_why() {
        let why = XTray::start_on(None, &tray(), |_| {}).err().map(|e| e.reason);
        assert_eq!(why.as_deref(), Some("no X11 display for a system tray"));
    }

    #[test]
    fn a_menu_picks_only_enabled_rows() {
        let m = Menu { win: 0, gc: 0, font: 0, items: rows(&tray().menu, 0), hover: None, width: 120 };
        assert_eq!(m.row_at(0), Some(0));
        assert_eq!(m.row_at(ROW as i16 * 3 + 1), Some(3));
        assert_eq!(m.row_at(ROW as i16 * 6), None);
        assert_eq!(m.enabled(0), Some(1));
        assert_eq!(m.enabled(1), None, "a separator");
        assert_eq!(m.enabled(2), None, "a disabled row");
        assert_eq!(m.enabled(3), Some(3));
        assert_eq!(m.enabled(4), None, "a submenu's header");
        assert_eq!(m.items.get(5).map(|r| r.0), Some(1), "its entries are indented under it");
        assert_eq!(m.enabled(5), Some(4), "and chosen like any check");
    }
}
