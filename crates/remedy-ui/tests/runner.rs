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
        modifiers: remedy_ui::input::Modifiers::NONE,
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
    // Clearing on lock removes the app's secret at once, and only that.
    runner.app().copy_secret("third-secret", Duration::from_secs(30));
    host.run_frames(&mut runner, 1);
    runner.app().clear_secret();
    host.run_frames(&mut runner, 1);
    assert!(host.clipboard().is_none_or(|(t, _)| t.is_empty()), "the secret is gone: {:?}", host.clipboard());
    runner.app().copy_secret("fourth-secret", Duration::from_secs(30));
    host.run_frames(&mut runner, 1);
    let _ = host.set_clipboard_text("mine again", false);
    runner.app().clear_secret();
    host.run_frames(&mut runner, 1);
    assert_eq!(host.clipboard(), Some(("mine again", false)));
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
fn completions_from_any_thread_reach_the_ui_thread_even_when_started_inside_effects() {
    let got: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let g2 = got.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        let trigger = remedy_ui::reactive::Signal::new(0);
        let (a, g3) = (app.clone(), g2.clone());
        // Started inside an effect that runs again right away: nothing it started is lost.
        let _ = remedy_ui::reactive::Effect::new(move || {
            let n = trigger.get();
            if n > 1 {
                return;
            }
            let (a2, g4) = (a.clone(), g3.clone());
            let c = a.completion(move |v: String| {
                g4.borrow_mut().push(v);
                // A second request started from the first one's result.
                let g5 = g4.clone();
                let c2 = a2.completion(move |v: String| g5.borrow_mut().push(v));
                std::thread::spawn(move || c2.complete(format!("nested {n}")));
            });
            std::thread::spawn(move || c.complete(format!("first {n}")));
            remedy_ui::reactive::untrack(|| trigger.set(n + 1));
        });
        // Dropped unfinished: the callback is forgotten, never called.
        let g6 = g2.clone();
        drop(app.completion(move |v: String| g6.borrow_mut().push(v)));
    });
    let mut host = HeadlessHost::new();
    for _ in 0..400 {
        host.run_frames(&mut runner, 1);
        if got.borrow().len() >= 4 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    host.run_frames(&mut runner, 3);
    let mut v = got.borrow().clone();
    v.sort();
    assert_eq!(v, ["first 0", "first 1", "nested 0", "nested 1"]);
}

#[test]
fn an_app_language_outlasts_the_system_s_until_it_is_cleared() {
    let ui_slot: Rc<std::cell::RefCell<Option<remedy_ui::Ui>>> = Rc::default();
    let u2 = ui_slot.clone();
    let heard: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let h2 = heard.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let h3 = h2.clone();
        app.on_locale(move |l| h3.borrow_mut().push(l.to_owned()));
        let u3 = u2.clone();
        app.open(WindowSpec::new("Words").size(100.0, 60.0), move |ui, _| {
            *u3.borrow_mut() = Some(ui.clone());
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let locale = |slot: &Rc<std::cell::RefCell<Option<remedy_ui::Ui>>>| {
        slot.borrow().as_ref().map(remedy_ui::Ui::locale)
    };
    let mut s = remedy_ui::host::SystemSettings { locale: "de-DE".into(), ..Default::default() };
    host.send(HostEvent::Settings(s.clone()));
    host.run_frames(&mut runner, 1);
    assert_eq!(locale(&ui_slot).as_deref(), Some("de-DE"));
    runner.app().set_locale(Some("ja".into()));
    host.run_frames(&mut runner, 1);
    assert_eq!(locale(&ui_slot).as_deref(), Some("ja"));
    s.locale = "fr-FR".into();
    host.send(HostEvent::Settings(s));
    host.run_frames(&mut runner, 1);
    assert_eq!(locale(&ui_slot).as_deref(), Some("ja"), "the app's choice stays");
    runner.app().set_locale(None);
    host.run_frames(&mut runner, 1);
    assert_eq!(locale(&ui_slot).as_deref(), Some("fr-FR"), "back to the system's");
    // What has no window hears each change once.
    assert_eq!(
        heard.borrow().iter().skip_while(|l| *l != "de-DE").cloned().collect::<Vec<_>>(),
        ["de-DE", "ja", "fr-FR"]
    );
    assert_eq!(runner.app().locale().as_deref(), Some("fr-FR"));
}

#[test]
fn capture_protection_is_a_request_and_windows_open_without_it() {
    for supported in [true, false] {
        let slot: Rc<std::cell::RefCell<Option<remedy_ui::Window>>> = Rc::default();
        let s2 = slot.clone();
        let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
            let w = app.open(WindowSpec::new("Vault").size(100.0, 60.0).capture_protected(true), |_, _| {});
            *s2.borrow_mut() = Some(w);
        });
        let mut host = HeadlessHost::new();
        host.set_capture_supported(supported);
        host.run_frames(&mut runner, 2);
        let w = slot.borrow().clone().unwrap_or_else(|| unreachable!("opened"));
        assert!(w.is_open(), "the window opens either way");
        assert_eq!(w.open_error(), None);
        assert_eq!(host.window_ids().len(), 1);
        assert_eq!(w.is_capture_protected(), supported, "and the app learns whether it holds");
        w.set_capture_protected(false);
        host.run_frames(&mut runner, 1);
        assert!(!w.is_capture_protected());
    }
}

#[test]
fn content_pops_out_to_its_own_window_and_comes_back_with_its_state() {
    type Slot<T> = Rc<std::cell::RefCell<Option<T>>>;
    let m_slot: Slot<remedy_ui::Movable> = Rc::default();
    let ui_slot: Slot<remedy_ui::Ui> = Rc::default();
    let count = remedy_ui::reactive::Signal::new(0u32);
    let (m2, u2) = (m_slot.clone(), ui_slot.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (m3, u3, a) = (m2.clone(), u2.clone(), app.clone());
        app.open(WindowSpec::new("Call").size(400.0, 300.0), move |ui, _| {
            let (el, m) = remedy_ui::movable(ui, &a, "Screen share", move |ui| {
                remedy_ui::widgets::label(ui, "", "").text_dyn(move || format!("frames {}", count.get()))
            });
            ui.root().child(el);
            *m3.borrow_mut() = Some(m);
            *u3.borrow_mut() = Some(ui.clone());
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let texts = |ui: &remedy_ui::Ui| -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![ui.root()];
        while let Some(e) = stack.pop() {
            if let Some(t) = e.info().and_then(|i| i.text) {
                out.push(t);
            }
            stack.extend(e.child_elements());
        }
        out
    };
    let main_ui = ui_slot.borrow().clone().unwrap_or_else(|| unreachable!("the window"));
    count.set(3);
    host.run_frames(&mut runner, 1);
    assert!(texts(&main_ui).contains(&"frames 3".to_owned()));
    let m = m_slot.borrow().clone().unwrap_or_else(|| unreachable!("movable"));
    m.pop_out();
    host.run_frames(&mut runner, 2);
    assert_eq!(host.window_ids().len(), 2, "a real window of its own");
    assert!(m.out.get_untracked().unwrap_or(false));
    let out_ui = m.window().and_then(|w| w.ui()).unwrap_or_else(|| unreachable!("its window"));
    count.set(4);
    host.run_frames(&mut runner, 1);
    assert!(texts(&out_ui).contains(&"frames 4".to_owned()), "the same state, shown out there");
    assert!(
        texts(&main_ui).iter().any(|t| t.contains("in its own window")),
        "the place it left says where it went"
    );
    // Closing its window brings it back, state and all.
    let w = m.window().unwrap_or_else(|| unreachable!());
    w.close();
    host.run_frames(&mut runner, 2);
    assert_eq!(host.window_ids().len(), 1);
    assert!(!m.out.get_untracked().unwrap_or(true));
    assert!(texts(&main_ui).contains(&"frames 4".to_owned()));
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

#[test]
fn keyboard_focus_and_scroll_into_view_move_scroll_containers() {
    let items = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (i2, list_el) = (items.clone(), std::rc::Rc::new(std::cell::RefCell::new(None)));
    let l2 = list_el.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (i3, l3) = (i2.clone(), l2.clone());
        app.open(WindowSpec::new("List").size(200.0, 200.0), move |ui, _| {
            let mut list = ui.el("scroll").scroll().column().layout(|l| {
                l.grow = 1.0;
                l.min_height = Length::Px(0.0);
            });
            for n in 0..30 {
                let b = ui
                    .el("button")
                    .height(40.0)
                    .text(remedy_ui::view::RichText::plain(format!("Item {n}")))
                    .on_click(|| {});
                i3.borrow_mut().push(b.clone());
                list = list.child(b);
            }
            *l3.borrow_mut() = Some(list.clone());
            ui.root().child(list);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let ui = runner.app().windows().first().and_then(|w| w.ui()).unwrap_or_else(|| unreachable!("a window"));
    let list = list_el.borrow().clone().unwrap_or_else(|| unreachable!("the list"));
    let item = |n: usize| items.borrow().get(n).cloned().unwrap_or_else(|| unreachable!("item {n}"));
    // An element far down scrolls into view, just enough.
    item(20).scroll_into_view();
    host.run_frames(&mut runner, 2);
    let off = ui.scroll_offset(list.id());
    let b = item(20).bounds().unwrap_or_default();
    assert!(off.y > 600.0 && b.max.y <= 200.5 && b.min.y >= 0.0, "offset {off:?}, item at {b:?}");
    // Tabbing to the next one keeps it in view.
    item(20).focus();
    let id = first(&host);
    let tab = KeyEvent {
        key: Key::Tab,
        physical: "Tab".into(),
        modifiers: Modifiers::NONE,
        pressed: true,
        repeat: false,
        composing: false,
        text: None,
    };
    for _ in 0..3 {
        host.send(HostEvent::Key(id, tab.clone()));
        host.run_frames(&mut runner, 2);
    }
    let b = item(23).bounds().unwrap_or_default();
    assert!(item(23).is_focused() && b.max.y <= 200.5 && b.min.y >= 0.0, "focused item at {b:?}");
}

#[test]
fn drags_cross_between_apps_through_the_host() {
    use remedy_ui::host::{DragData, DragEvent, DropEffect};
    use remedy_ui::view::Drag;
    let dropped: Rc<std::cell::RefCell<Vec<std::path::PathBuf>>> = Rc::default();
    let ended = Rc::new(Cell::new(None));
    let (d2, e2) = (dropped.clone(), ended.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (d3, e3) = (d2.clone(), e2.clone());
        app.open(WindowSpec::new("Drops").size(200.0, 100.0), move |ui, _| {
            let (d4, e4) = (d3.clone(), e3.clone());
            let item = ui
                .el("button")
                .layout(|l| {
                    l.width = Length::Px(40.0);
                    l.height = Length::Px(20.0);
                })
                .draggable(|| Some(Drag::new(DragData::text("note"))), move |e| e4.set(Some(e)));
            let bin = ui
                .el("box")
                .layout(|l| {
                    l.width = Length::Px(200.0);
                    l.height = Length::Px(60.0);
                })
                .on_drop(
                    |d, _| if d.data.files.is_empty() { DropEffect::None } else { DropEffect::Copy },
                    move |d, _, _| d4.borrow_mut().extend(d.data.files.iter().cloned()),
                );
            ui.root().child(item).child(bin);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    // Files from another app, over the bin: the host is told a drop would copy.
    let data = DragData::files(vec!["C:/in.txt".into()]);
    host.send(HostEvent::Drag(
        id,
        DragEvent::Enter { position: Some(Point::new(50.0, 50.0)), data: data.clone() },
    ));
    host.run_frames(&mut runner, 1);
    assert_eq!(host.drop_effect(id), DropEffect::Copy);
    host.send(HostEvent::Drag(id, DragEvent::Drop { position: Some(Point::new(50.0, 50.0)), data }));
    host.run_frames(&mut runner, 1);
    assert_eq!(*dropped.borrow(), vec![std::path::PathBuf::from("C:/in.txt")]);
    // Our item dragged out of the window: the host carries it, and another app copies it.
    host.answer_drag(DropEffect::Copy);
    let mv = |x: f32, y: f32, phase: PointerPhase, down: bool| {
        let mut e = press(x, y, phase);
        e.buttons = if down { Buttons::PRIMARY } else { Buttons::default() };
        e
    };
    host.send(HostEvent::Pointer(id, mv(10.0, 10.0, PointerPhase::Down, true)));
    host.send(HostEvent::Pointer(id, mv(30.0, 10.0, PointerPhase::Move, true)));
    host.send(HostEvent::Pointer(id, mv(300.0, 10.0, PointerPhase::Move, true)));
    host.run_frames(&mut runner, 1);
    assert_eq!(
        host.drags_out().iter().map(|(_, d)| d.text.clone()).collect::<Vec<_>>(),
        vec![Some("note".to_owned())]
    );
    assert_eq!(ended.get(), Some(DropEffect::Copy));
}

#[test]
fn links_open_through_the_host_and_only_web_and_mail_links() {
    let results: Rc<std::cell::RefCell<Vec<Result<(), String>>>> = Rc::default();
    let r2 = results.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        for url in [
            "https://example.com/docs",
            "mailto:someone@example.com",
            "file:///C:/Windows/notepad.exe",
            "https://a.example/ x",
        ] {
            let r3 = r2.clone();
            app.open_url(url, move |r| r3.borrow_mut().push(r));
        }
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert_eq!(host.opened_urls(), ["https://example.com/docs", "mailto:someone@example.com"]);
    let ok: Vec<bool> = results.borrow().iter().map(Result::is_ok).collect();
    assert_eq!(ok, vec![true, true, false, false]);
}

#[test]
fn work_posted_from_other_threads_reaches_the_app() {
    let got = Signal::new(0u32);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |_app: &App| {
        let waker =
            remedy_ui::reactive::waker().unwrap_or_else(|e| unreachable!("a waker on the UI thread: {e}"));
        std::thread::spawn(move || {
            let _ = waker.post(move || got.set(42));
        });
    });
    let mut host = HeadlessHost::new();
    for _ in 0..200 {
        host.run_frames(&mut runner, 1);
        if got.get_untracked().ok() == Some(42) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(got.get_untracked().ok(), Some(42), "the post ran without the app calling anything");
}

#[test]
fn themes_change_live_and_apps_hear_the_system_appearance() {
    type Seen = Rc<std::cell::RefCell<Vec<(Option<bool>, bool)>>>;
    let seen: Seen = Rc::default();
    let ui_slot: Rc<std::cell::RefCell<Option<remedy_ui::Ui>>> = Rc::default();
    let (s2, u2) = (seen.clone(), ui_slot.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let s3 = s2.clone();
        app.on_appearance(move |dark, hc| s3.borrow_mut().push((dark, hc)));
        let u3 = u2.clone();
        app.open(WindowSpec::new("Themes").size(100.0, 60.0), move |ui, _| {
            *u3.borrow_mut() = Some(ui.clone());
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let mut s = remedy_ui::host::SystemSettings { dark: Some(false), ..Default::default() };
    host.send(HostEvent::Settings(s.clone()));
    host.run_frames(&mut runner, 1);
    s.dark = Some(true);
    s.high_contrast = true;
    host.send(HostEvent::Settings(s.clone()));
    host.send(HostEvent::Settings(s));
    host.run_frames(&mut runner, 1);
    let got = seen.borrow().clone();
    assert_eq!(got.last(), Some(&(Some(true), true)));
    assert!(got.contains(&(Some(false), false)));
    assert_eq!(got.iter().filter(|x| **x == (Some(true), true)).count(), 1, "only changes are told");
    assert_eq!(runner.app().appearance(), Some((Some(true), true)), "and it can be read back");
    let mut light = remedy_ui::style::Theme::dark_forest();
    light.name = "Test light".into();
    runner.app().set_theme(light);
    host.run_frames(&mut runner, 1);
    assert_eq!(ui_slot.borrow().as_ref().map(|u| u.theme().name.clone()), Some("Test light".to_owned()));
}

#[test]
fn a_covered_or_flattened_window_tells_its_elements_they_are_not_seen() {
    let seen: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let s2 = seen.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let s2 = s2.clone();
        app.open(WindowSpec::new("Watched").size(200.0, 100.0), move |ui, _| {
            let watcher =
                ui.el("watcher").width(50.0).height(20.0).on_visibility(move |v| s2.borrow_mut().push(v));
            ui.root().child(watcher);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    assert_eq!(*seen.borrow(), vec![true]);
    host.send(HostEvent::Occluded(id, true));
    host.run_frames(&mut runner, 1);
    host.send(HostEvent::Occluded(id, false));
    host.run_frames(&mut runner, 1);
    assert_eq!(*seen.borrow(), vec![true, false, true], "covered, then uncovered");
    // Windows that lose their area (minimized on Windows) show nothing until they have it again.
    host.send(HostEvent::Resized { window: id, width: 0, height: 0, scale: 1.0 });
    host.run_frames(&mut runner, 1);
    assert_eq!(seen.borrow().last(), Some(&false));
    host.send(HostEvent::Resized { window: id, width: 200, height: 100, scale: 1.0 });
    host.run_frames(&mut runner, 1);
    assert_eq!(seen.borrow().last(), Some(&true));
}
