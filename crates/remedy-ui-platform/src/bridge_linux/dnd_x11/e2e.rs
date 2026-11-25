//! Dragging out on X11 (an end-to-end test of [`super::drag`]) against a real XDND target, on a private Xvfb server (the pointer and
//! button are simulated there with XTEST; no real display or mouse is touched). Skipped
//! where Xvfb is not installed.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn atom(c: &RustConnection, n: &str) -> u32 {
    c.intern_atom(false, n.as_bytes()).ok().and_then(|r| r.reply().ok()).map_or(0, |r| r.atom)
}

#[test]
fn files_dragged_out_reach_an_xdnd_target_as_a_uri_list() {
    let display = ":87";
    let Ok(child) = Command::new("Xvfb")
        .args([display, "-screen", "0", "400x300x24", "-nolisten", "tcp"])
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
    let root = conn.setup().roots.first().map(|s| s.root).unwrap_or_default();
    // The target: an XDND-aware window covering the screen.
    let target = conn.generate_id().unwrap_or_default();
    let _ = conn.create_window(
        24,
        target,
        root,
        0,
        0,
        400,
        300,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    );
    let (aware, status, position, drop_, enter, finished, selection, uri_list, copy) = (
        atom(&conn, "XdndAware"),
        atom(&conn, "XdndStatus"),
        atom(&conn, "XdndPosition"),
        atom(&conn, "XdndDrop"),
        atom(&conn, "XdndEnter"),
        atom(&conn, "XdndFinished"),
        atom(&conn, "XdndSelection"),
        atom(&conn, "text/uri-list"),
        atom(&conn, "XdndActionCopy"),
    );
    let got = atom(&conn, "REMEDY_UI_TEST_DROP");
    let _ = conn.change_property32(PropMode::REPLACE, target, aware, AtomEnum::ATOM, &[5]);
    let _ = conn.map_window(target);
    // The button goes down over the target, as when a drag leaves the app's window.
    let _ = conn.xtest_fake_input(6, 0, 0, root, 200, 150, 0); // MotionNotify
    let _ = conn.xtest_fake_input(4, 1, 0, root, 0, 0, 0); // ButtonPress 1
    let _ = conn.flush();

    let worker = std::thread::spawn(move || {
        let data = remedy_ui_core::host::DragData {
            files: vec!["/home/sam/beach day.jpg".into()],
            ..remedy_ui_core::host::DragData::default()
        };
        super::drag_on(Some(display), &data, &[remedy_ui_core::host::DropEffect::Copy])
    });

    // Play the target: accept positions, release the button after a few, take the drop.
    let mut source = 0;
    let mut positions = 0;
    let mut entered = false;
    let mut received = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    'run: while Instant::now() < deadline {
        while let Ok(Some(ev)) = conn.poll_for_event() {
            match ev {
                Event::ClientMessage(m) if m.type_ == enter => {
                    entered = true;
                    source = m.data.as_data32().first().copied().unwrap_or_default();
                }
                Event::ClientMessage(m) if m.type_ == position => {
                    positions += 1;
                    let ev = ClientMessageEvent::new(32, source, status, [target, 1, 0, 0, copy]);
                    let _ = conn.send_event(false, source, EventMask::NO_EVENT, ev);
                    if positions == 2 {
                        let _ = conn.xtest_fake_input(5, 1, 0, root, 0, 0, 0); // ButtonRelease 1
                    } else {
                        let _ = conn.xtest_fake_input(6, 0, 0, root, 200 + positions, 150, 0);
                    }
                    let _ = conn.flush();
                }
                Event::ClientMessage(m) if m.type_ == drop_ => {
                    let _ = conn.convert_selection(target, selection, uri_list, got, x11rb::CURRENT_TIME);
                    let _ = conn.flush();
                }
                Event::SelectionNotify(n) if n.property == got => {
                    if let Ok(r) =
                        conn.get_property(true, target, got, AtomEnum::ANY, 0, 4096).map(|c| c.reply())
                        && let Ok(r) = r
                    {
                        received = r.value;
                    }
                    let ev = ClientMessageEvent::new(32, source, finished, [target, 1, copy, 0, 0]);
                    let _ = conn.send_event(false, source, EventMask::NO_EVENT, ev);
                    let _ = conn.flush();
                    break 'run;
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let effect = worker.join().unwrap_or_else(|_| unreachable!("the drag thread"));
    assert!(entered, "the target heard XdndEnter");
    assert!(positions >= 2, "and positions");
    assert_eq!(String::from_utf8_lossy(&received), "file:///home/sam/beach%20day.jpg\r\n");
    assert_eq!(effect.ok(), Some(remedy_ui_core::host::DropEffect::Copy));
}
