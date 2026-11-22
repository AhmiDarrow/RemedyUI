//! The tray's menu driven from the keyboard and the pointer on a private Xvfb server (keys and
//! the pointer are simulated there with XTEST; no real display is touched). Skipped where Xvfb
//! is not installed.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use x11rb::protocol::xtest::ConnectionExt as _;

use super::*;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The keycode the server's keymap puts `sym` on.
fn keycode(conn: &RustConnection, sym: u32) -> u8 {
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn.get_keyboard_mapping(min, max - min + 1).ok().and_then(|c| c.reply().ok());
    let Some(map) = map else { return 0 };
    let per = usize::from(map.keysyms_per_keycode.max(1));
    map.keysyms.iter().position(|s| *s == sym).map_or(0, |i| min + (i / per) as u8)
}

/// Hands the menu the events that come within a moment, until one ends it.
fn settle(conn: &RustConnection, m: &mut Menu) -> MenuOutcome {
    let _ = conn.flush();
    let mut quiet = Instant::now();
    while quiet.elapsed() < Duration::from_millis(300) {
        match conn.poll_for_event() {
            Ok(Some(e)) => {
                quiet = Instant::now();
                match m.handle(conn, &e) {
                    Ok(MenuOutcome::Open) => {}
                    Ok(out) => return out,
                    Err(e) => unreachable!("{e}"),
                }
                let _ = conn.flush();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => unreachable!("{e}"),
        }
    }
    MenuOutcome::Open
}

/// Where level `i`'s window is, and its width.
fn place(m: &Menu, i: usize) -> (i16, i16, u16) {
    m.shown.get(i).map_or((0, 0, 0), |s| (s.x, s.y, s.width))
}

#[test]
fn the_menu_and_its_submenus_answer_keys_and_the_pointer() {
    let display = ":88";
    let Ok(child) = Command::new("Xvfb")
        .args([display, "-screen", "0", "640x480x24", "-nolisten", "tcp"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        eprintln!("Xvfb is not installed: skipped");
        return;
    };
    let _server = Server(child);
    let start = Instant::now();
    let conn = loop {
        if let Ok((c, _)) = x11rb::connect(Some(display)) {
            break c;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "Xvfb did not start");
        std::thread::sleep(Duration::from_millis(50));
    };
    let Some(screen) = conn.setup().roots.first().cloned() else { unreachable!("a screen") };
    let items = vec![
        TrayItem::item(1, "&Open"),
        TrayItem::submenu("&Server", vec![TrayItem::check(4, "&Logs", false), TrayItem::item(5, "Restart")]),
        TrayItem::item(7, "Quit"),
    ];
    let at = ButtonPressEvent { root_x: 20, root_y: 20, ..ButtonPressEvent::default() };
    let open = || Menu::open(&conn, &screen, &items, at).unwrap_or_else(|e| unreachable!("{e}"));
    let key = |sym: u32| {
        let code = keycode(&conn, sym);
        assert_ne!(code, 0, "the keymap has {sym:#x}");
        let _ = conn.xtest_fake_input(2, code, 0, screen.root, 0, 0, 0);
        let _ = conn.xtest_fake_input(3, code, 0, screen.root, 0, 0, 0);
    };
    let pointer = |x: i16, y: i16| {
        let _ = conn.xtest_fake_input(6, 0, 0, screen.root, x, y, 0);
    };
    let click = || {
        let _ = conn.xtest_fake_input(4, 1, 0, screen.root, 0, 0, 0);
        let _ = conn.xtest_fake_input(5, 1, 0, screen.root, 0, 0, 0);
    };

    // Keys: the access key opens the submenu beside its row, arrows move, Enter chooses.
    let mut m = open();
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Open);
    key(u32::from('s'));
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Open);
    assert_eq!(m.shown.len(), 2, "the submenu opened");
    let ((ox, oy, ow), (ix, iy, _)) = (place(&m, 0), place(&m, 1));
    assert_eq!(ix, ox + ow as i16, "beside the menu");
    assert_eq!(iy, oy + ROW as i16, "level with its row");
    assert_eq!(m.nav.hover(1), Some(0));
    key(XK_DOWN);
    key(XK_ESCAPE);
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Open);
    assert_eq!(m.shown.len(), 1, "Escape closed the submenu");
    key(XK_RIGHT);
    key(XK_DOWN);
    key(XK_RETURN);
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Chosen(5));
    let _ = m.close(&conn);

    // The pointer: over the submenu's row it opens; a click in it chooses.
    let mut m = open();
    let _ = settle(&conn, &mut m);
    let (x, y, _) = place(&m, 0);
    pointer(x + 10, y + ROW as i16 + ROW as i16 / 2);
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Open);
    assert_eq!(m.shown.len(), 2, "hovering the row opened it");
    let (sx, sy, _) = place(&m, 1);
    pointer(sx + 10, sy + ROW as i16 / 2);
    click();
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Chosen(4));
    let _ = m.close(&conn);

    // A click outside, or Escape at the top, closes it.
    let mut m = open();
    let _ = settle(&conn, &mut m);
    pointer(600, 460);
    click();
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Closed);
    let _ = m.close(&conn);
    let mut m = open();
    let _ = settle(&conn, &mut m);
    key(XK_ESCAPE);
    assert_eq!(settle(&conn, &mut m), MenuOutcome::Closed);
    let _ = m.close(&conn);
}
