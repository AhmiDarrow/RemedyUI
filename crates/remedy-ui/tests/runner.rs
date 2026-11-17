//! Apps driven through the host contract on the headless bridge.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::host::{Host, HostEvent, ResizeEdge, WindowId};
use remedy_ui::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerKind, PointerPhase};
use remedy_ui::layout::Length;
use remedy_ui::reactive::Signal;
use remedy_ui::style::{States, StylePatch, StyleSheet};
use remedy_ui::{App, AppConfig, FieldKind, GeometryStore, Runner, WindowSpec};
use remedy_ui_bridge_headless::HeadlessHost;
use remedy_ui_core::{Color, Point, Size};

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };

fn config() -> AppConfig {
    AppConfig::new("RemedyUI runner test").cpu_only().theme(remedy_ui::style::Theme::dark_forest(), |_| {
        let mut s = StyleSheet::new();
        s.rule("button", None, States::NONE, StylePatch { background: Some(RED), ..StylePatch::default() });
        s
    })
}

fn press(x: f32, y: f32, phase: PointerPhase) -> PointerEvent {
    PointerEvent {
        id: 0,
        kind: PointerKind::Mouse,
        phase,
        position: Point::new(x, y),
        buttons: if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() },
        pressure: 0.5,
        time: Duration::ZERO,
        coalesced: Vec::new(),
    }
}

fn px(host: &HeadlessHost, id: WindowId, x: usize, y: usize) -> [u8; 4] {
    let Some(w) = host.window(id) else { return [0; 4] };
    let width = w.size().0 as usize;
    let i = (y * width + x) * 4;
    let s = w.surface();
    [
        s.get(i).copied().unwrap_or(0),
        s.get(i + 1).copied().unwrap_or(0),
        s.get(i + 2).copied().unwrap_or(0),
        s.get(i + 3).copied().unwrap_or(0),
    ]
}

fn first(host: &HeadlessHost) -> WindowId {
    host.window_ids().first().copied().unwrap_or(WindowId(0))
}

#[test]
fn a_window_renders_and_clicks_repaint_it() {
    let count = Signal::new(0u32);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Counter").size(200.0, 100.0), move |ui, _| {
            let label = ui.el("label").text_dyn(move || format!("Clicked {}", count.get()));
            let button = ui
                .el("button")
                .layout(|l| {
                    l.width = Length::Px(60.0);
                    l.height = Length::Px(30.0);
                })
                .on_click(move || count.update(|c| *c += 1));
            ui.root().child(button).child(label);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    assert_eq!(host.window(id).map(|w| w.options().title.clone()), Some("Counter".into()));
    assert_eq!(px(&host, id, 10, 10), [255, 0, 0, 255], "the button is on screen");
    let presents = host.window(id).map_or(0, |w| w.presents());
    host.run_frames(&mut runner, 2);
    assert_eq!(host.window(id).map(|w| w.presents()), Some(presents), "idle frames present nothing");
    host.send(HostEvent::Pointer(id, press(10.0, 10.0, PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, press(10.0, 10.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert_eq!(count.get(), 1);
    assert!(host.window(id).is_some_and(|w| w.presents() > presents), "the label change was presented");
}

#[test]
fn hide_on_close_windows_survive_and_the_last_real_close_quits() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.open(WindowSpec::new("Manager").hide_on_close(), |_, _| {});
        app.open(WindowSpec::new("Note"), |_, _| {});
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let ids = host.window_ids();
    let [manager, note] = ids.as_slice() else { unreachable!("two windows") };
    let (manager, note) = (*manager, *note);
    host.send(HostEvent::CloseRequested(manager));
    host.run_frames(&mut runner, 1);
    assert!(host.window(manager).is_some_and(|w| !w.options().visible), "hidden, not closed");
    assert_eq!(runner.app().windows().len(), 2);
    host.send(HostEvent::CloseRequested(note));
    host.run_frames(&mut runner, 1);
    assert!(host.window(note).is_none());
    assert!(!host.exited(), "the hidden manager still counts as open");
    let manager_window = runner.app().windows().into_iter().next();
    if let Some(w) = manager_window {
        w.close();
    }
    host.run_frames(&mut runner, 1);
    assert!(host.exited(), "the last window closed: quit");
}

#[test]
fn keep_running_tray_apps_outlive_their_windows_and_timers_fire() {
    let fired = Rc::new(Cell::new(false));
    let f2 = fired.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        let w = app.open(WindowSpec::new("Only"), |_, _| {});
        let app2 = app.clone();
        app.after(Duration::from_millis(100), move || {
            f2.set(true);
            w.close();
            let _ = &app2;
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert!(!fired.get());
    host.run_frames(&mut runner, 10);
    assert!(fired.get());
    assert!(host.window_ids().is_empty());
    assert!(!host.exited(), "a tray app keeps running with no windows");
}

#[test]
fn window_geometry_persists_across_runs() {
    let dir = std::env::temp_dir().join(format!("remedy-ui-geometry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("windows.txt");
    let spec = || WindowSpec::new("Note").size(300.0, 200.0).persist_geometry("note 7");
    {
        let mut runner =
            Runner::with_geometry(config(), GeometryStore::open(file.clone()), move |app: &App| {
                app.open(spec(), |_, _| {});
            });
        let mut host = HeadlessHost::new();
        host.run_frames(&mut runner, 1);
        let id = first(&host);
        let _ = host.set_position(id, Point::new(40.0, 50.0));
        host.user_resize(id, Size::new(345.0, 280.0));
        host.run_frames(&mut runner, 40);
        assert_eq!(runner.geometry().get("note 7"), Some((Point::new(40.0, 50.0), Size::new(345.0, 280.0))));
    }
    assert!(file.exists(), "saved after moves settle");
    let mut runner = Runner::with_geometry(config(), GeometryStore::open(file), move |app: &App| {
        app.open(spec(), |_, _| {});
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    assert_eq!(
        host.window(id).map(|w| (w.options().position, w.options().size)),
        Some((Some(Point::new(40.0, 50.0)), Size::new(345.0, 280.0)))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fields_paste_from_the_host_clipboard_and_window_commands_reach_the_host() {
    let field_value = Rc::new(std::cell::RefCell::new(String::new()));
    let fv = field_value.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Fields").frameless(), move |ui, window| {
            let field = ui.text_field("text_input", FieldKind::Single);
            let f2 = field.clone();
            let field = field.on_change(move || *fv.borrow_mut() = f2.value());
            ui.root().child(field.el().layout(|l| l.width = Length::Px(150.0)));
            field.el().focus();
            window.set_always_on_top(true);
            window.set_title("Renamed");
            window.begin_resize(ResizeEdge::BottomRight);
            window.app().copy("from the app", true);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    assert_eq!(host.clipboard(), Some(("from the app", true)));
    assert!(host.window(id).is_some_and(|w| w.options().always_on_top
        && w.options().title == "Renamed"
        && !w.options().decorated));
    assert_eq!(host.take_interactions(), vec![(id, Some(ResizeEdge::BottomRight))]);
    assert!(
        host.window(id).is_some_and(|w| w.text_input().is_some()),
        "the focused field opened a text session"
    );
    let ctrl_v = KeyEvent {
        key: Key::Character("v".into()),
        physical: "KeyV".into(),
        modifiers: Modifiers { ctrl: true, ..Modifiers::NONE },
        pressed: true,
        repeat: false,
        composing: false,
        text: None,
    };
    host.send(HostEvent::Key(id, ctrl_v));
    host.run_frames(&mut runner, 1);
    assert_eq!(*field_value.borrow(), "from the app");
}

#[test]
fn idle_lock_waits_for_real_input_and_secret_copies_clear_only_themselves() {
    let locked = Rc::new(Cell::new(0));
    let l2 = locked.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Vault"), |_, _| {});
        app.idle_lock(Some(Duration::from_secs(60)), move || l2.set(l2.get() + 1));
        app.copy_secret("recovery-key-123", Duration::from_secs(30));
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    assert_eq!(host.clipboard(), Some(("recovery-key-123", true)), "copied as sensitive");
    // Activity at 40 s pushes the lock to 100 s.
    host.advance(Duration::from_secs(40));
    host.send(HostEvent::Pointer(id, press(5.0, 5.0, PointerPhase::Move)));
    host.run_frames(&mut runner, 1);
    assert_eq!(host.clipboard(), None, "the secret was cleared after 30 s");
    host.advance(Duration::from_secs(30));
    host.run_frames(&mut runner, 1);
    assert_eq!(locked.get(), 0, "70 s: input at 40 s keeps it unlocked");
    host.advance(Duration::from_secs(40));
    host.run_frames(&mut runner, 1);
    assert_eq!(locked.get(), 1, "110 s: locked");
    host.advance(Duration::from_secs(10));
    host.run_frames(&mut runner, 1);
    assert_eq!(locked.get(), 1, "once per idle stretch");
    // A secret is not cleared if the user copied something else meanwhile.
    runner.app().copy_secret("second-secret", Duration::from_secs(30));
    host.run_frames(&mut runner, 1);
    let _ = host.set_clipboard_text("my own text", false);
    host.advance(Duration::from_secs(31));
    host.run_frames(&mut runner, 1);
    assert_eq!(host.clipboard(), Some(("my own text", false)), "the user's copy survives");
}

#[test]
fn privacy_cover_hides_content_while_unfocused() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.open(WindowSpec::new("Secrets").size(200.0, 100.0).privacy_cover(true), |ui, _| {
            ui.root().child(ui.el("button").layout(|l| {
                l.width = Length::Px(60.0);
                l.height = Length::Px(30.0);
            }));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    assert_eq!(px(&host, id, 10, 10), [255, 0, 0, 255]);
    host.send(HostEvent::Focused(id, false));
    host.run_frames(&mut runner, 1);
    assert_ne!(px(&host, id, 10, 10), [255, 0, 0, 255], "covered while unfocused");
    host.send(HostEvent::Focused(id, true));
    host.run_frames(&mut runner, 1);
    assert_eq!(px(&host, id, 10, 10), [255, 0, 0, 255], "shown again");
}

#[test]
fn tray_dialogs_and_second_launches_reach_the_app() {
    use remedy_ui::host::{FileDialog, TrayEvent, TrayIcon, TrayImage, TrayItem};
    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let launches = Rc::new(std::cell::RefCell::new(Vec::new()));
    let picked = Rc::new(std::cell::RefCell::new(Vec::new()));
    let (e2, l2, p2) = (events.clone(), launches.clone(), picked.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        app.set_tray(Some(TrayIcon {
            tooltip: "SecretSticky".into(),
            icon: TrayImage { size: 2, rgba: vec![255; 16] },
            menu: vec![
                TrayItem::Item { id: 7, label: "New note".into(), enabled: true },
                TrayItem::Separator,
            ],
        }));
        app.on_tray(move |e| e2.borrow_mut().push(e));
        app.on_second_instance(move |a| l2.borrow_mut().push(a));
        let p3 = p2.clone();
        app.file_dialog(
            None,
            FileDialog { title: "Export".into(), save: true, ..FileDialog::default() },
            move |p| *p3.borrow_mut() = p,
        );
    });
    let mut host = HeadlessHost::new();
    host.answer_dialog(vec![std::path::PathBuf::from("notes.txt")]);
    host.run_frames(&mut runner, 1);
    assert_eq!(host.tray().map(|t| t.tooltip.as_str()), Some("SecretSticky"));
    assert_eq!(host.dialogs().first().map(|d| (d.title.as_str(), d.save)), Some(("Export", true)));
    assert_eq!(*picked.borrow(), vec![std::path::PathBuf::from("notes.txt")]);
    host.send(HostEvent::Tray(TrayEvent::Menu(7)));
    host.send(HostEvent::Tray(TrayEvent::Activate));
    host.send(HostEvent::SecondInstance(vec!["--show".into()]));
    host.run_frames(&mut runner, 1);
    assert_eq!(*events.borrow(), vec![TrayEvent::Menu(7), TrayEvent::Activate]);
    assert_eq!(*launches.borrow(), vec![vec!["--show".to_owned()]]);
    runner.app().set_tray(None);
    host.run_frames(&mut runner, 1);
    assert!(host.tray().is_none());
}

#[test]
fn screen_readers_get_the_tree_and_can_click_and_focus() {
    use remedy_ui::host::AccessibilityEvent;
    use remedy_ui::host::accesskit::{Action, ActionRequest, NodeId, TreeId};
    let count = Signal::new(0u32);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("A11y"), move |ui, _| {
            let b = ui
                .el("button")
                .text(remedy_ui::view::RichText::plain("Add note"))
                .on_click(move || count.update(|c| *c += 1));
            ui.root().child(b);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    assert!(host.take_accessibility(id).is_empty(), "nothing is sent until a reader connects");
    host.send(HostEvent::Accessibility(id, AccessibilityEvent::Requested));
    host.run_frames(&mut runner, 2);
    let updates = host.take_accessibility(id);
    let tree = format!("{updates:?}");
    assert!(tree.contains("Add note") && tree.contains("Click") && tree.contains("Focus"), "{tree}");
    // The node that carries the Click action is the button.
    let button = updates
        .iter()
        .flat_map(|u| u.nodes.iter())
        .find(|(_, n)| n.supports_action(Action::Click))
        .map(|(nid, _)| *nid);
    let Some(NodeId(target)) = button else { unreachable!("a clickable node: {tree}") };
    let request =
        |action| ActionRequest { action, target_tree: TreeId::ROOT, target_node: NodeId(target), data: None };
    host.send(HostEvent::Accessibility(id, AccessibilityEvent::Action(request(Action::Click))));
    host.send(HostEvent::Accessibility(id, AccessibilityEvent::Action(request(Action::Focus))));
    host.run_frames(&mut runner, 1);
    assert_eq!(count.get(), 1, "the reader's click pressed the button");
    let after = format!("{:?}", host.take_accessibility(id));
    assert!(after.contains(&format!("focus: #{target}")), "focus moved to it: {after}");
}

#[test]
fn background_work_returns_to_the_ui_thread() {
    let result = Rc::new(std::cell::RefCell::new(None));
    let r2 = result.clone();
    let ui_thread = std::thread::current().id();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        app.background(
            || (std::thread::current().id(), (1..=10u64).product::<u64>()),
            move |(worker, v)| {
                assert_ne!(worker, std::thread::current().id(), "the work ran elsewhere");
                *r2.borrow_mut() = Some((std::thread::current().id(), v));
            },
        );
    });
    let mut host = HeadlessHost::new();
    for _ in 0..200 {
        host.run_frames(&mut runner, 1);
        if result.borrow().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*result.borrow(), Some((ui_thread, 3_628_800)), "done ran on the UI thread with the result");
}

#[test]
fn windows_report_geometry_and_closing_and_title_bars_drag() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let closed = Rc::new(Cell::new(0u32));
    let (s2, c2) = (seen.clone(), closed.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (s3, c3) = (s2.clone(), c2.clone());
        let win = app.open(WindowSpec::new("Note").size(300.0, 200.0).frameless(), move |ui, w| {
            let w2 = w.clone();
            let bar = ui.el("titlebar").height(40.0).fill_width().drag_region(move || w2.begin_move());
            let bar = bar.child(ui.el("button").width(40.0).height(40.0).on_click(|| {}));
            ui.root().child(bar);
        });
        win.on_geometry(move |p, s| s3.borrow_mut().push((p, s)));
        win.on_closed(move || c3.set(c3.get() + 1));
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    // A press on the button keeps the press; one on the bar beside it moves the window.
    host.send(HostEvent::Pointer(id, press(20.0, 20.0, PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, press(20.0, 20.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert!(host.take_interactions().is_empty(), "buttons in the bar are not drag handles");
    host.send(HostEvent::Pointer(id, press(150.0, 20.0, PointerPhase::Down)));
    host.run_frames(&mut runner, 1);
    assert_eq!(host.take_interactions(), vec![(id, None)], "the bar moves the window");
    host.send(HostEvent::Pointer(id, press(150.0, 20.0, PointerPhase::Up)));
    host.send(HostEvent::Pointer(id, press(150.0, 120.0, PointerPhase::Down)));
    host.run_frames(&mut runner, 1);
    assert!(host.take_interactions().is_empty(), "the body below the bar is not a handle");

    let _ = host.set_position(id, Point::new(70.0, 80.0));
    host.user_resize(id, Size::new(345.0, 250.0));
    host.run_frames(&mut runner, 2);
    assert_eq!(seen.borrow().last().copied(), Some((Point::new(70.0, 80.0), Size::new(345.0, 250.0))));
    host.send(HostEvent::CloseRequested(id));
    host.run_frames(&mut runner, 2);
    assert_eq!(closed.get(), 1, "on_closed ran once");
}

#[test]
fn activity_is_reported_at_most_once_a_second() {
    let n = Rc::new(Cell::new(0u32));
    let n2 = n.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.on_activity(move || n2.set(n2.get() + 1));
        app.open(WindowSpec::new("A").size(100.0, 100.0), |_, _| {});
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    for _ in 0..5 {
        host.send(HostEvent::Pointer(id, press(10.0, 10.0, PointerPhase::Move)));
        host.run_frames(&mut runner, 1);
    }
    assert_eq!(n.get(), 1, "bursts are reported once");
    host.advance(Duration::from_secs(2));
    host.send(HostEvent::Pointer(id, press(10.0, 10.0, PointerPhase::Move)));
    host.run_frames(&mut runner, 1);
    assert_eq!(n.get(), 2);
}

#[cfg(feature = "devtools")]
#[test]
fn f12_opens_one_inspector_per_window_and_closing_it_detaches() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("App").size(300.0, 200.0), |ui, _| {
            ui.root().child(ui.el("button").text(remedy_ui::view::RichText::plain("Hi")).on_click(|| {}));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let app_win = first(&host);
    let f12 = KeyEvent {
        key: Key::F(12),
        physical: "F12".into(),
        modifiers: Modifiers::NONE,
        pressed: true,
        repeat: false,
        composing: false,
        text: None,
    };
    host.send(HostEvent::Key(app_win, f12.clone()));
    host.run_frames(&mut runner, 3);
    assert_eq!(host.window_ids().len(), 2, "the inspector opened");
    let inspector = host.window_ids().into_iter().find(|w| *w != app_win).unwrap_or(app_win);
    assert!(host.window(inspector).is_some_and(|w| w.options().title == "Inspector · App"));
    host.send(HostEvent::Key(app_win, f12));
    host.run_frames(&mut runner, 2);
    assert_eq!(host.window_ids().len(), 2, "a second F12 does not open another");
    host.send(HostEvent::CloseRequested(inspector));
    host.run_frames(&mut runner, 2);
    assert_eq!(host.window_ids(), vec![app_win]);
}

#[test]
fn windows_know_the_local_date_and_roll_over_at_midnight() {
    use remedy_ui::host::LocalTime;
    let seen = Signal::new(None::<LocalTime>);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Clock").size(100.0, 100.0), move |ui, _| {
            let u = ui.clone();
            ui.root().child(ui.el("label").bind(move || seen.set(Some(u.local_time()))));
        });
    });
    let mut host = HeadlessHost::new();
    // 2026-11-17 23:59:00 at UTC+1.
    host.set_local_time(1_794_956_340, 3600);
    host.run_frames(&mut runner, 2);
    let day = |t: Option<LocalTime>| t.map(|t| t.local_days());
    let first = day(seen.get());
    assert_eq!(seen.get().map(|t| t.utc_offset), Some(3600));
    // Idle for two minutes: the midnight timer moves the date on with no input.
    host.advance(Duration::from_secs(120));
    host.run_frames(&mut runner, 1);
    assert_eq!(day(seen.get()), first.map(|d| d + 1), "the next local day");
}
