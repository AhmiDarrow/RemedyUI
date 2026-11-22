//! The tray on X11 desktops without a StatusNotifier host (older panels, plain window
//! managers with a system tray such as stalonetray or trayer): the freedesktop System Tray
//! Protocol over XEmbed. The icon is a small window the tray docks; pressing it reports
//! [`TrayEvent::Activate`], and the secondary button opens the menu as a pop-up window
//! with submenus cascading beside it (keyboard: arrows, Enter, Escape, and access keys: the
//! letter after `&` in a label, else its first letter) whose choices report
//! [`TrayEvent::Menu`].
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
const XK_LEFT: u32 = 0xff51;
const XK_RIGHT: u32 = 0xff53;

/// The keysym the keyboard's layout puts on `keycode` (unshifted).
fn keysym(conn: &RustConnection, keycode: u8) -> u32 {
    conn.get_keyboard_mapping(keycode, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.keysyms.first().copied())
        .unwrap_or(0)
}

/// A label's text with its `&` marks taken out, and its access key's index among the text's
/// characters: the character after a lone `&`, as on Windows. `&&` is a literal `&`.
fn mnemonic(label: &str) -> (String, Option<usize>) {
    let mut text = String::new();
    let mut key = None;
    let mut n = 0;
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        let c = match c {
            '&' => match chars.next() {
                Some('&') => '&',
                Some(k) => {
                    key = key.or(Some(n));
                    k
                }
                None => break,
            },
            c => c,
        };
        text.push(c);
        n += 1;
    }
    (text, key)
}

/// The key that picks an entry: its `&` letter, else its first character, lowercased.
fn access_key(item: &TrayItem) -> Option<char> {
    let (text, key) = mnemonic(item.label());
    let c = text.chars().nth(key.unwrap_or(0))?;
    c.to_lowercase().next()
}

/// A key the menu acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Char(char),
}

#[derive(Debug, PartialEq, Eq)]
enum MenuOutcome {
    Open,
    Chosen(u32),
    Closed,
}

/// One open level of the menu: its entries and the hot row.
struct Level {
    items: Vec<TrayItem>,
    hover: Option<usize>,
}

/// The menu and its open submenus, innermost last: what keys and the pointer do to them,
/// apart from the windows that show them. Never empty.
struct Nav {
    levels: Vec<Level>,
}

impl Nav {
    fn new(items: &[TrayItem]) -> Nav {
        Nav { levels: vec![Level { items: items.to_vec(), hover: None }] }
    }

    fn top(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }

    fn hover(&self, level: usize) -> Option<usize> {
        self.levels.get(level).and_then(|l| l.hover)
    }

    fn set_hover(&mut self, level: usize, row: Option<usize>) {
        if let Some(l) = self.levels.get_mut(level) {
            l.hover = row;
        }
    }

    /// Whether row `i` of `level` can be chosen, or opened (a submenu with entries).
    fn live(&self, level: usize, i: usize) -> bool {
        match self.levels.get(level).and_then(|l| l.items.get(i)) {
            Some(TrayItem::Submenu { items, .. }) => !items.is_empty(),
            Some(item) => item.chosen_id().is_some(),
            None => false,
        }
    }

    /// Opens the submenu on `level`'s hot row (closing deeper ones); `select` makes its first
    /// row hot, as opening it from the keyboard does.
    fn open_sub(&mut self, level: usize, select: bool) {
        self.levels.truncate(level + 1);
        let l = self.levels.get(level);
        let Some(TrayItem::Submenu { items, .. }) = l.and_then(|l| l.items.get(l.hover?)) else { return };
        if items.is_empty() {
            return;
        }
        let items = items.clone();
        self.levels.push(Level { items, hover: None });
        if select {
            self.step(level + 1, false);
        }
    }

    /// Makes the next live row of `level` hot (the previous with `up`), wrapping around.
    fn step(&mut self, level: usize, up: bool) {
        let n = self.levels.get(level).map_or(0, |l| l.items.len());
        if n == 0 {
            return;
        }
        let next = |i: usize| if up { (i + n - 1) % n } else { (i + 1) % n };
        let mut i = self.hover(level).map_or(if up { n - 1 } else { 0 }, next);
        for _ in 0..n {
            if self.live(level, i) {
                self.set_hover(level, Some(i));
                return;
            }
            i = next(i);
        }
    }

    /// Row `i` of `level` as Enter or a click takes it: a command is chosen, a submenu opens.
    fn activate(&mut self, level: usize, i: usize, keyboard: bool) -> MenuOutcome {
        if !self.live(level, i) {
            return MenuOutcome::Open;
        }
        self.set_hover(level, Some(i));
        match self.levels.get(level).and_then(|l| l.items.get(i)) {
            Some(TrayItem::Submenu { .. }) => {
                self.open_sub(level, keyboard);
                MenuOutcome::Open
            }
            Some(item) => item.chosen_id().map_or(MenuOutcome::Open, MenuOutcome::Chosen),
            None => MenuOutcome::Open,
        }
    }

    /// A key, which acts on the innermost open level.
    fn key(&mut self, k: NavKey) -> MenuOutcome {
        let top = self.top();
        match k {
            NavKey::Escape if top == 0 => MenuOutcome::Closed,
            NavKey::Escape | NavKey::Left => {
                if top > 0 {
                    self.levels.pop();
                }
                MenuOutcome::Open
            }
            NavKey::Up | NavKey::Down => {
                self.step(top, k == NavKey::Up);
                MenuOutcome::Open
            }
            NavKey::Right => {
                self.open_sub(top, true);
                MenuOutcome::Open
            }
            NavKey::Enter => match self.hover(top) {
                Some(i) => self.activate(top, i, true),
                None => MenuOutcome::Open,
            },
            NavKey::Char(c) => {
                let c = c.to_lowercase().next().unwrap_or(c);
                let n = self.levels.get(top).map_or(0, |l| l.items.len());
                let hits: Vec<usize> = (0..n)
                    .filter(|&i| {
                        self.live(top, i)
                            && self.levels.get(top).and_then(|l| l.items.get(i)).and_then(access_key)
                                == Some(c)
                    })
                    .collect();
                match hits.as_slice() {
                    [] => MenuOutcome::Open,
                    [one] => self.activate(top, *one, true),
                    // Several share the key: each press moves to the next, as Windows menus do.
                    [first, ..] => {
                        let after = self.hover(top).and_then(|h| hits.iter().find(|&&i| i > h));
                        self.set_hover(top, Some(*after.unwrap_or(first)));
                        MenuOutcome::Open
                    }
                }
            }
        }
    }

    /// The pointer is over `row` of `level` (`None`: off its rows); true when that changed
    /// anything. Over a submenu, the submenu opens.
    fn point(&mut self, level: usize, row: Option<usize>) -> bool {
        if level >= self.levels.len() {
            return false;
        }
        let row = row.filter(|&i| self.live(level, i));
        if row.is_none() {
            // Off the rows: an outer level keeps the row its open submenu came from.
            if level == self.top() && self.hover(level).is_some() {
                self.set_hover(level, None);
                return true;
            }
            return false;
        }
        let opened = self.levels.len() > level + 1;
        let is_sub = matches!(
            row.and_then(|i| self.levels.get(level).and_then(|l| l.items.get(i))),
            Some(TrayItem::Submenu { .. })
        );
        if self.hover(level) == row && (opened || !is_sub) {
            return false;
        }
        self.set_hover(level, row);
        self.levels.truncate(level + 1);
        if is_sub {
            self.open_sub(level, false);
        }
        true
    }

    /// A press on `row` of `level`; `None` is outside every level, which closes the menu.
    fn click(&mut self, at: Option<(usize, Option<usize>)>) -> MenuOutcome {
        match at {
            None => MenuOutcome::Closed,
            Some((level, Some(i))) => self.activate(level, i, false),
            Some(_) => MenuOutcome::Open,
        }
    }
}

/// A keysym as a menu key: arrows, Enter, Escape, or the character it types.
fn nav_key(sym: u32) -> Option<NavKey> {
    Some(match sym {
        XK_ESCAPE => NavKey::Escape,
        XK_RETURN | XK_KP_ENTER => NavKey::Enter,
        XK_UP => NavKey::Up,
        XK_DOWN => NavKey::Down,
        XK_LEFT => NavKey::Left,
        XK_RIGHT => NavKey::Right,
        // Latin-1 keysyms are their characters; others are 0x0100_0000 plus the code point.
        0x20..=0x7e | 0xa0..=0xff => NavKey::Char(char::from_u32(sym)?),
        0x0100_0000..=0x0110_ffff => NavKey::Char(char::from_u32(sym - 0x0100_0000)?),
        _ => return None,
    })
}

/// A level's window and where it is on the screen.
struct Shown {
    win: Window,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    /// The row of the level outside it that it opened from.
    from: Option<usize>,
}

/// The tray's menu as pop-up windows: the menu, and each open submenu beside the row it
/// opened from.
struct Menu {
    gc: Gcontext,
    font: Font,
    /// The font's character width.
    char_w: u16,
    screen: Screen,
    /// Where the menu was opened (the press on the icon), in root coordinates.
    at: (i16, i16),
    nav: Nav,
    shown: Vec<Shown>,
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
        let char_w = conn
            .query_font(font)?
            .reply()
            .ok()
            .and_then(|f| u16::try_from(f.max_bounds.character_width).ok())
            .filter(|w| *w > 0)
            .unwrap_or(7);
        let gc = conn.generate_id()?;
        conn.create_gc(
            gc,
            screen.root,
            &CreateGCAux::new().font(font).foreground(0x0020_2020).background(0x00f4_f4f4),
        )?;
        let mut m = Menu {
            gc,
            font,
            char_w,
            screen: screen.clone(),
            at: (at.root_x, at.root_y),
            nav: Nav::new(items),
            shown: Vec::new(),
        };
        m.sync(conn)?;
        let Some(first) = m.shown.first().map(|s| s.win) else { return Ok(m) };
        // Every click and key comes here until the menu closes; presses on a submenu's window
        // go to it (owner events), presses elsewhere to the menu, outside its rows.
        let _ = conn.grab_pointer(
            true,
            first,
            EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )?;
        let _ = conn.grab_keyboard(false, first, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)?;
        Ok(m)
    }

    /// Windows follow the open levels: closed ones go, new ones open, and all are drawn.
    fn sync(&mut self, conn: &RustConnection) -> Result<(), Box<dyn std::error::Error>> {
        let levels = &self.nav.levels;
        let keep = self
            .shown
            .iter()
            .enumerate()
            .take_while(|(i, s)| {
                *i < levels.len()
                    && (*i == 0 || i.checked_sub(1).and_then(|p| levels.get(p)?.hover) == s.from)
            })
            .count();
        for s in self.shown.drain(keep..) {
            conn.destroy_window(s.win)?;
        }
        for i in self.shown.len()..self.nav.levels.len() {
            let Some(level) = self.nav.levels.get(i) else { break };
            let longest =
                level.items.iter().map(|it| mnemonic(it.label()).0.chars().count() + 4).max().unwrap_or(8);
            let width = (longest as u16).saturating_mul(self.char_w).saturating_add(2 * PAD as u16).max(120);
            let height = (level.items.len() as u16).saturating_mul(ROW).max(ROW);
            let (sw, sh) = (self.screen.width_in_pixels as i16, self.screen.height_in_pixels as i16);
            let from = i.checked_sub(1).and_then(|p| self.nav.hover(p));
            let (x, y) = match (i.checked_sub(1).and_then(|p| self.shown.get(p)), from) {
                // Beside its row: to the right, or the left where the screen ends.
                (Some(parent), Some(row)) => {
                    let right = parent.x + parent.width as i16;
                    let x = if right + width as i16 <= sw { right } else { parent.x - width as i16 };
                    (x, parent.y + row as i16 * ROW as i16)
                }
                // Above or below the icon.
                _ => {
                    let (ax, ay) = self.at;
                    (ax, if ay > sh / 2 { ay - height as i16 } else { ay })
                }
            };
            let x = x.min(sw - width as i16).max(0);
            let y = y.min(sh - height as i16).max(0);
            let win = conn.generate_id()?;
            conn.create_window(
                self.screen.root_depth,
                win,
                self.screen.root,
                x,
                y,
                width,
                height,
                1,
                WindowClass::INPUT_OUTPUT,
                self.screen.root_visual,
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
            conn.map_window(win)?;
            self.shown.push(Shown { win, x, y, width, height, from });
        }
        for i in 0..self.shown.len() {
            self.draw(conn, i)?;
        }
        Ok(())
    }

    fn draw(&self, conn: &RustConnection, level: usize) -> Result<(), Box<dyn std::error::Error>> {
        use x11rb::protocol::xproto::{ChangeGCAux, Char2b, CoordMode, Point, Rectangle};
        let (Some(s), Some(l)) = (self.shown.get(level), self.nav.levels.get(level)) else { return Ok(()) };
        conn.clear_area(false, s.win, 0, 0, 0, 0)?;
        let cw = self.char_w as i16;
        for (i, item) in l.items.iter().enumerate() {
            let top = i as i16 * ROW as i16;
            if matches!(item, TrayItem::Separator) {
                let mid = top + ROW as i16 / 2;
                conn.poly_line(
                    CoordMode::ORIGIN,
                    s.win,
                    self.gc,
                    &[Point { x: PAD, y: mid }, Point { x: s.width as i16 - PAD, y: mid }],
                )?;
                continue;
            }
            let live = self.nav.live(level, i);
            let hot = l.hover == Some(i) && live;
            let (fg, bg) = match (hot, live) {
                (true, _) => (0x00ff_ffff, 0x0035_6ac0),
                (false, true) => (0x0020_2020, 0x00f4_f4f4),
                (false, false) => (0x0090_9090, 0x00f4_f4f4),
            };
            conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bg))?;
            conn.poly_fill_rectangle(
                s.win,
                self.gc,
                &[Rectangle { x: 0, y: top, width: s.width, height: ROW }],
            )?;
            conn.change_gc(self.gc, &ChangeGCAux::new().foreground(fg).background(bg))?;
            let mark = match item {
                TrayItem::Check { checked: true, .. } => '\u{2713}',
                _ => ' ',
            };
            let (label, key) = mnemonic(item.label());
            let glyphs = |s: &str| -> Vec<Char2b> {
                s.chars()
                    .map(|c| {
                        let u = u32::from(c).min(0xffff) as u16;
                        Char2b { byte1: (u >> 8) as u8, byte2: u as u8 }
                    })
                    .collect()
            };
            let text = glyphs(&format!("{mark} {label}"));
            let base = top + ROW as i16 - 7;
            conn.image_text16(s.win, self.gc, PAD, base, &text)?;
            if matches!(item, TrayItem::Item { default: true, .. }) {
                // Bold for the default: core fonts have no bold face, so the text again a pixel
                // to the right.
                conn.image_text16(s.win, self.gc, PAD + 1, base, &text)?;
            }
            if let Some(k) = key {
                // The access key, underlined (after the mark and its space).
                let x = PAD + (k as i16 + 2) * cw;
                conn.poly_line(
                    CoordMode::ORIGIN,
                    s.win,
                    self.gc,
                    &[Point { x, y: base + 2 }, Point { x: x + cw - 1, y: base + 2 }],
                )?;
            }
            if matches!(item, TrayItem::Submenu { .. }) {
                conn.image_text16(s.win, self.gc, s.width as i16 - PAD - cw, base, &glyphs(">"))?;
            }
        }
        Ok(())
    }

    /// The level whose window `win` is, and the row at `(x, y)` in it.
    fn at(&self, win: Window, x: i16, y: i16) -> Option<(usize, Option<usize>)> {
        let level = self.shown.iter().position(|s| s.win == win)?;
        let s = self.shown.get(level)?;
        if x < 0 || y < 0 || x >= s.width as i16 || y >= s.height as i16 {
            return None;
        }
        let n = self.nav.levels.get(level).map_or(0, |l| l.items.len());
        Some((level, Some((y / ROW as i16) as usize).filter(|i| *i < n)))
    }

    fn handle(
        &mut self,
        conn: &RustConnection,
        e: &Event,
    ) -> Result<MenuOutcome, Box<dyn std::error::Error>> {
        let out = match e {
            Event::Expose(e) => {
                if let Some(i) = self.shown.iter().position(|s| s.win == e.window) {
                    self.draw(conn, i)?;
                }
                return Ok(MenuOutcome::Open);
            }
            Event::MotionNotify(e) => {
                let changed = match self.at(e.event, e.event_x, e.event_y) {
                    Some((level, row)) => self.nav.point(level, row),
                    None => self.nav.point(self.nav.top(), None),
                };
                if !changed {
                    return Ok(MenuOutcome::Open);
                }
                MenuOutcome::Open
            }
            Event::ButtonPress(e) => self.nav.click(self.at(e.event, e.event_x, e.event_y)),
            Event::KeyPress(k) => match nav_key(keysym(conn, k.detail)) {
                Some(key) => self.nav.key(key),
                None => return Ok(MenuOutcome::Open),
            },
            _ => return Ok(MenuOutcome::Open),
        };
        if out == MenuOutcome::Open {
            self.sync(conn)?;
        }
        Ok(out)
    }

    fn close(&self, conn: &RustConnection) -> Result<(), Box<dyn std::error::Error>> {
        conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
        conn.ungrab_keyboard(x11rb::CURRENT_TIME)?;
        for s in self.shown.iter().rev() {
            conn.destroy_window(s.win)?;
        }
        conn.free_gc(self.gc)?;
        conn.close_font(self.font)?;
        conn.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod e2e;

#[cfg(test)]
mod tests {
    use super::*;

    fn tray() -> TrayIcon {
        TrayIcon::new(
            "App",
            remedy_ui_host::TrayImage { size: 2, rgba: vec![255; 16] },
            vec![
                TrayItem::Item { id: 1, label: "&Open".into(), enabled: true, default: true },
                TrayItem::Separator,
                TrayItem::Item { id: 2, label: "Room".into(), enabled: false, default: false },
                TrayItem::item(3, "&Quit"),
                TrayItem::submenu(
                    "&Server",
                    vec![
                        TrayItem::check(4, "&Logs", true),
                        TrayItem::item(5, "Restart"),
                        TrayItem::item(6, "Re&load"),
                        TrayItem::submenu("Empty", Vec::new()),
                    ],
                ),
                TrayItem::item(7, "Settings"),
            ],
        )
    }

    #[test]
    fn without_a_display_it_says_why() {
        let why = XTray::start_on(None, &tray(), |_| {}).err().map(|e| e.reason);
        assert_eq!(why.as_deref(), Some("no X11 display for a system tray"));
    }

    #[test]
    fn labels_mark_their_access_key_with_an_ampersand() {
        assert_eq!(mnemonic("&Open"), ("Open".into(), Some(0)));
        assert_eq!(mnemonic("Re&load"), ("Reload".into(), Some(2)));
        assert_eq!(mnemonic("Salt && &Pepper"), ("Salt & Pepper".into(), Some(7)));
        assert_eq!(mnemonic("Plain"), ("Plain".into(), None));
        assert_eq!(mnemonic("End&"), ("End".into(), None));
        assert_eq!(access_key(&TrayItem::item(1, "Re&load")), Some('l'));
        assert_eq!(access_key(&TrayItem::item(1, "Restart")), Some('r'), "else the first letter");
    }

    #[test]
    fn arrows_skip_what_cannot_be_chosen_and_enter_chooses() {
        let mut n = Nav::new(&tray().menu);
        assert_eq!(n.key(NavKey::Down), MenuOutcome::Open);
        assert_eq!(n.hover(0), Some(0));
        n.key(NavKey::Down);
        assert_eq!(n.hover(0), Some(3), "past the separator and the disabled row");
        n.key(NavKey::Up);
        assert_eq!(n.hover(0), Some(0));
        n.key(NavKey::Up);
        assert_eq!(n.hover(0), Some(5), "wrapping around");
        assert_eq!(n.key(NavKey::Enter), MenuOutcome::Chosen(7));
    }

    #[test]
    fn submenus_open_and_close_from_the_keyboard() {
        let mut n = Nav::new(&tray().menu);
        n.key(NavKey::Up);
        n.key(NavKey::Up);
        assert_eq!(n.hover(0), Some(4), "the submenu's row");
        n.key(NavKey::Right);
        assert_eq!((n.levels.len(), n.hover(1)), (2, Some(0)), "open, its first row hot");
        n.key(NavKey::Down);
        n.key(NavKey::Down);
        n.key(NavKey::Down);
        assert_eq!(n.hover(1), Some(0), "an empty submenu cannot be entered, so it wraps");
        n.key(NavKey::Left);
        assert_eq!((n.levels.len(), n.hover(0)), (1, Some(4)), "back out, on its row");
        n.key(NavKey::Enter);
        assert_eq!(n.levels.len(), 2, "Enter opens it too");
        assert_eq!(n.key(NavKey::Escape), MenuOutcome::Open, "Escape closes the submenu");
        assert_eq!(n.levels.len(), 1);
        assert_eq!(n.key(NavKey::Escape), MenuOutcome::Closed, "then the menu");
    }

    #[test]
    fn access_keys_choose_in_the_open_level() {
        let mut n = Nav::new(&tray().menu);
        assert_eq!(n.key(NavKey::Char('S')), MenuOutcome::Open, "S is Server's and Settings' first letter");
        assert_eq!(n.hover(0), Some(4));
        n.key(NavKey::Char('s'));
        assert_eq!(n.hover(0), Some(5), "each press moves to the next");
        let mut n = Nav::new(&tray().menu);
        n.key(NavKey::Down);
        n.key(NavKey::Down);
        n.key(NavKey::Right);
        assert_eq!(n.levels.len(), 1, "Quit has no submenu");
        let mut n = Nav::new(&tray().menu);
        n.set_hover(0, Some(4));
        n.key(NavKey::Right);
        assert_eq!(n.key(NavKey::Char('l')), MenuOutcome::Open, "Logs and Reload share L");
        assert_eq!(n.hover(1), Some(2));
        assert_eq!(
            n.key(NavKey::Char('r')),
            MenuOutcome::Chosen(5),
            "Reload's key is its L, so R is Restart's"
        );
        assert_eq!(n.key(NavKey::Char('q')), MenuOutcome::Open, "keys act on the submenu");
        let mut n = Nav::new(&tray().menu);
        assert_eq!(n.key(NavKey::Char('q')), MenuOutcome::Chosen(3));
        assert_eq!(n.key(NavKey::Char('x')), MenuOutcome::Open);
    }

    #[test]
    fn the_pointer_opens_submenus_and_clicks_choose() {
        let mut n = Nav::new(&tray().menu);
        assert!(n.point(0, Some(4)));
        assert_eq!(n.levels.len(), 2, "over the submenu's row, it opens");
        assert_eq!(n.hover(1), None);
        assert!(!n.point(0, Some(4)), "staying put changes nothing");
        assert!(n.point(1, Some(0)));
        assert!(!n.point(0, None), "leaving the outer menu keeps the submenu open");
        assert_eq!(n.click(Some((1, Some(0)))), MenuOutcome::Chosen(4));
        assert!(n.point(0, Some(0)));
        assert_eq!(n.levels.len(), 1, "another row closes it");
        assert_eq!(n.click(Some((0, Some(2)))), MenuOutcome::Open, "a disabled row");
        assert_eq!(n.click(None), MenuOutcome::Closed, "outside the menu");
    }

    #[test]
    fn keysyms_become_menu_keys() {
        assert_eq!(nav_key(XK_RIGHT), Some(NavKey::Right));
        assert_eq!(nav_key(XK_KP_ENTER), Some(NavKey::Enter));
        assert_eq!(nav_key(u32::from('q')), Some(NavKey::Char('q')));
        assert_eq!(nav_key(0xe9), Some(NavKey::Char('\u{e9}')));
        assert_eq!(nav_key(0x0100_0436), Some(NavKey::Char('\u{436}')));
        assert_eq!(nav_key(0xffe1), None, "Shift");
    }
}
