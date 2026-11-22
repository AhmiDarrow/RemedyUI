//! Apps driven through the host contract on the headless bridge.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::host::{Host, HostEvent, ResizeEdge, WindowId};
use remedy_ui::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
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
        s.rule("button", None, States::NONE, StylePatch::default().background(RED));
        s
    })
}

#[test]
fn the_apps_edition_reaches_every_window() {
    let seen: Rc<Cell<Option<remedy_ui::Edition>>> = Rc::default();
    let s2 = seen.clone();
    let config = config().edition(remedy_ui::Edition::LATEST);
    let mut runner = Runner::with_geometry(config, GeometryStore::in_memory(), move |app: &App| {
        let s3 = s2.clone();
        app.open(WindowSpec::new("Edition"), move |ui, _| s3.set(Some(ui.edition())));
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert_eq!(seen.get(), Some(remedy_ui::Edition::LATEST));
    // An app that never names one keeps the first edition's behavior.
    assert_eq!(AppConfig::new("No edition").edition, remedy_ui::Edition::FIRST);
}

fn press(x: f32, y: f32, phase: PointerPhase) -> PointerEvent {
    PointerEvent::new(phase, Point::new(x, y))
        .with_buttons(if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() })
        .with_modifiers(remedy_ui::input::Modifiers::NONE)
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
            let label = ui.el("label").text(move || format!("Clicked {}", count.get()));
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
    assert!(!host.has_exited(), "the hidden manager still counts as open");
    let manager_window = runner.app().windows().into_iter().next();
    if let Some(w) = manager_window {
        w.close();
    }
    host.run_frames(&mut runner, 1);
    assert!(host.has_exited(), "the last window closed: quit");
}

#[test]
fn a_window_reports_being_hidden_by_its_close_button_and_shown_again() {
    let seen: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let s2 = seen.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        let w = app.open(WindowSpec::new("Tray app").hide_on_close(), |_, _| {});
        let s3 = s2.clone();
        w.on_visibility(move |on| s3.borrow_mut().push(on));
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = host.window_ids().first().copied().unwrap_or_else(|| unreachable!("a window"));
    host.send(HostEvent::CloseRequested(id));
    host.run_frames(&mut runner, 1);
    assert_eq!(*seen.borrow(), [false], "the close button hid it");
    let w = runner.app().windows().into_iter().next().unwrap_or_else(|| unreachable!("still open"));
    w.focus();
    w.set_visible(true);
    host.run_frames(&mut runner, 1);
    assert_eq!(*seen.borrow(), [false, true], "shown again, reported once");
    assert!(host.window(id).is_some_and(|w| w.options().visible));
}

#[test]
fn where_windows_cannot_hide_keeping_one_minimizes_it_and_says_it_is_still_shown() {
    let seen: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let s2 = seen.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        let w = app.open(WindowSpec::new("Tray app").hide_on_close(), |_, _| {});
        let s3 = s2.clone();
        w.on_visibility(move |on| s3.borrow_mut().push(on));
    });
    let mut host = HeadlessHost::new();
    host.set_hides_windows(false);
    host.run_frames(&mut runner, 1);
    let id = host.window_ids().first().copied().unwrap_or_else(|| unreachable!("a window"));
    let w = runner.app().windows().into_iter().next().unwrap_or_else(|| unreachable!("open"));
    assert!(!w.can_hide());
    host.send(HostEvent::CloseRequested(id));
    host.run_frames(&mut runner, 1);
    let shown = |host: &HeadlessHost| host.window(id).map(|w| (w.options().visible, w.is_minimized()));
    assert_eq!(shown(&host), Some((true, true)), "minimized, not hidden");
    host.set_minimized(id, false);
    w.set_visible(false);
    host.run_frames(&mut runner, 1);
    assert_eq!(shown(&host), Some((true, true)), "the app's hide minimizes too");
    assert!(seen.borrow().is_empty(), "never reported hidden: it is still on the screen");
}

#[test]
fn where_windows_cannot_hide_a_window_hidden_before_it_exists_opens_minimized() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.keep_running(true);
        app.open(WindowSpec::new("Opened hidden").hidden(), |_, _| {});
        let w = app.open(WindowSpec::new("Hidden at once"), |_, _| {});
        w.set_visible(false);
    });
    let mut host = HeadlessHost::new();
    host.set_hides_windows(false);
    host.run_frames(&mut runner, 2);
    for id in host.window_ids() {
        assert!(host.window(id).is_some_and(|w| w.is_minimized()), "minimized");
    }
    assert!(runner.app().windows().iter().all(|w| w.is_visible()), "and reported shown");
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
    assert!(!host.has_exited(), "a tray app keeps running with no windows");
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

/// How a browser page keeps geometry (it has no files): in the app's persist store, here a
/// folder one so the test runs natively.
#[test]
fn window_geometry_persists_in_a_persist_store_and_is_written_when_the_app_goes_out_of_sight() {
    use remedy_ui::reactive::{Persist, PersistStore, Signal};
    let dir = std::env::temp_dir().join(format!("remedy-ui-geometry-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let spec = || WindowSpec::new("Panel").size(300.0, 200.0).persist_geometry("panel");
    let draft: Rc<Cell<Option<Signal<String>>>> = Rc::default();
    {
        // A long debounce: only the flush on going out of sight writes in time.
        let Ok(store) = PersistStore::with_debounce(&dir, Duration::from_secs(600)) else {
            unreachable!("a store in a temporary folder")
        };
        let d2 = draft.clone();
        let mut runner = Runner::with_geometry(
            config().data_dir(dir.join("unused")),
            GeometryStore::persisted_in(&store),
            move |app: &App| {
                app.open(spec(), move |_, _| {
                    d2.set(Some(Signal::new(String::new()).persist("draft")));
                });
            },
        );
        let mut host = HeadlessHost::new();
        host.run_frames(&mut runner, 1);
        let id = first(&host);
        let _ = host.set_position(id, Point::new(12.0, 34.0));
        host.user_resize(id, Size::new(320.0, 240.0));
        host.run_frames(&mut runner, 40);
        let set_at = std::time::Instant::now();
        if let Some(d) = draft.get() {
            d.set("lye calculation".into());
        }
        host.run_frames(&mut runner, 2);
        let state = dir.join("unused").join("state");
        let before = state.join("draft.json").exists();
        host.send(HostEvent::Occluded(id, true));
        host.run_frames(&mut runner, 1);
        // Within the app store's debounce, only the write on going out of sight can save it.
        let in_debounce = set_at.elapsed() < remedy_ui::reactive::DEFAULT_DEBOUNCE;
        if in_debounce {
            assert!(!before, "the persisted draft waits for its debounce");
        }
        assert!(state.join("draft.json").exists(), "going out of sight wrote the app's persisted state");
        assert!(store.flush().is_ok(), "the geometry entry is written");
        assert!(store.path_of(remedy_ui::GEOMETRY_KEY).exists());
        use remedy_ui::reactive::JsonValue;
        let Ok(Some(saved)) = store.load::<JsonValue>(&Persist::new(remedy_ui::GEOMETRY_KEY)) else {
            unreachable!("the geometry entry reads back")
        };
        let row = ["panel".into(), 12.0.into(), 34.0.into(), 320.0.into(), 240.0.into()];
        assert_eq!(saved, JsonValue::Array(vec![JsonValue::Array(row.to_vec())]));
    }
    let Ok(store) = PersistStore::open(&dir) else { unreachable!("the store opens again") };
    let mut runner =
        Runner::with_geometry(config(), GeometryStore::persisted_in(&store), move |app: &App| {
            app.open(spec(), |_, _| {});
        });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    assert_eq!(
        host.window(id).map(|w| (w.options().position, w.options().size)),
        Some((Some(Point::new(12.0, 34.0)), Size::new(320.0, 240.0)))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quiet_windows_ask_the_host_for_no_focus_and_no_taskbar_button() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.open(WindowSpec::new("Quiet").activate(false).in_taskbar(false), |_, _| {});
        app.open(WindowSpec::new("Plain"), |_, _| {});
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let flags = |title: &str| {
        host.window_ids()
            .into_iter()
            .filter_map(|id| host.window(id))
            .find(|w| w.options().title == title)
            .map(|w| (w.options().activate, w.options().taskbar))
    };
    assert_eq!(flags("Quiet"), Some((false, false)));
    assert_eq!(flags("Plain"), Some((true, true)), "windows take focus and a taskbar button by default");
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
    let ctrl_v = KeyEvent::press(Key::Character("v".into()))
        .with_physical("KeyV")
        .with_modifiers(Modifiers { ctrl: true, ..Modifiers::NONE })
        .with_text(None);
    host.send(HostEvent::Key(id, ctrl_v));
    host.run_frames(&mut runner, 1);
    assert_eq!(*field_value.borrow(), "from the app");
}

fn typed(c: &str, ctrl: bool) -> KeyEvent {
    KeyEvent::press(Key::Character(c.into()))
        .with_modifiers(Modifiers { ctrl, ..Modifiers::NONE })
        .with_text(if ctrl { None } else { Some(c.into()) })
}

#[test]
fn a_pause_in_typing_starts_a_new_undo_step() {
    let field_value = Rc::new(std::cell::RefCell::new(String::new()));
    let fv = field_value.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Typing"), move |ui, _| {
            let field = ui.text_field("text_input", FieldKind::Single);
            let f2 = field.clone();
            let field = field.on_change(move || *fv.borrow_mut() = f2.value());
            ui.root().child(field.el().layout(|l| l.width = Length::Px(150.0)));
            field.el().focus();
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    for c in ["a", "b"] {
        host.send(HostEvent::Key(id, typed(c, false)));
        host.run_frames(&mut runner, 1);
    }
    host.advance(Duration::from_secs(3));
    host.send(HostEvent::Key(id, typed("c", false)));
    host.run_frames(&mut runner, 1);
    assert_eq!(*field_value.borrow(), "abc");
    let platform_undo = if cfg!(target_os = "macos") { (false, true) } else { (true, false) };
    let mut undo = typed("z", true);
    (undo.modifiers.ctrl, undo.modifiers.meta) = platform_undo;
    host.send(HostEvent::Key(id, undo));
    host.run_frames(&mut runner, 1);
    assert_eq!(*field_value.borrow(), "ab", "the typing after the pause is its own step");
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
fn windows_carry_icons_of_their_own() {
    use remedy_ui::host::WindowIcon;
    let amber = WindowIcon::new(2, [251, 191, 36, 255].repeat(4)).unwrap_or_else(|| unreachable!("2 × 2"));
    let green = WindowIcon::new(1, vec![74, 222, 128, 255]).unwrap_or_else(|| unreachable!("1 × 1"));
    let windows = Rc::new(std::cell::RefCell::new(Vec::new()));
    let (w2, a2) = (windows.clone(), amber.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        w2.borrow_mut().push(app.open(WindowSpec::new("Manager").icon(a2), |_, _| {}));
        w2.borrow_mut().push(app.open(WindowSpec::new("Plain"), |_, _| {}));
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let ids = host.window_ids();
    let icon_of = |host: &HeadlessHost, i: usize| {
        ids.get(i).and_then(|id| host.window(*id)).and_then(|w| w.options().icon.clone())
    };
    assert_eq!(icon_of(&host, 0), Some(amber), "opened with its own icon");
    assert_eq!(icon_of(&host, 1), None, "the app's icon");
    let Some(plain) = windows.borrow().get(1).cloned() else { unreachable!("opened") };
    plain.set_icon(Some(green.clone()));
    host.run_frames(&mut runner, 1);
    assert_eq!(icon_of(&host, 1), Some(green));
    plain.set_icon(None);
    host.run_frames(&mut runner, 1);
    assert_eq!(icon_of(&host, 1), None, "back to the app's");
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
fn snapshots_picture_the_whole_next_frame() {
    let got = Rc::new(std::cell::RefCell::new(Vec::new()));
    let win = Rc::new(std::cell::RefCell::new(None));
    let w2 = win.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let w = app.open(WindowSpec::new("Tour").size(200.0, 100.0), |ui, _| {
            ui.root().child(ui.el("button").layout(|l| {
                l.width = Length::Px(60.0);
                l.height = Length::Px(30.0);
            }));
        });
        *w2.borrow_mut() = Some(w);
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    // Asked after the window has drawn: the picture still holds all of it, not only what changed.
    for _ in 0..2 {
        let g = got.clone();
        if let Some(w) = win.borrow().as_ref() {
            w.snapshot(move |pic| g.borrow_mut().push(pic));
        }
    }
    host.run_frames(&mut runner, 1);
    let pics = got.borrow();
    assert_eq!(pics.len(), 2, "both answered by one frame");
    let Some(Some(pic)) = pics.first() else { unreachable!("a picture") };
    assert_eq!((pic.width, pic.height), (200, 100));
    let id = first(&host);
    let at = (10 * 200 + 10) * 4;
    assert_eq!(
        pic.pixels.get(at..at + 4),
        Some(&px(&host, id, 10, 10)[..]),
        "the button, as the window shows it"
    );
}

#[test]
fn windows_report_focus_changes_once_each() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let win = Rc::new(std::cell::RefCell::new(None));
    let (s2, w2) = (seen.clone(), win.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let w = app.open(WindowSpec::new("Chat"), |_, _| {});
        let s3 = s2.clone();
        w.on_focus(move |on| s3.borrow_mut().push(on));
        *w2.borrow_mut() = Some(w);
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    let focused = || win.borrow().as_ref().is_some_and(remedy_ui::Window::is_focused);
    host.send(HostEvent::Focused(id, true));
    host.run_frames(&mut runner, 1);
    assert!(focused());
    host.send(HostEvent::Focused(id, true));
    host.send(HostEvent::Focused(id, false));
    host.run_frames(&mut runner, 1);
    assert!(!focused(), "another window came in front");
    assert_eq!(*seen.borrow(), [true, false], "a repeat is not reported");
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
        app.set_tray(Some(TrayIcon::new(
            "SecretSticky",
            TrayImage { size: 2, rgba: vec![255; 16] },
            vec![TrayItem::Item { id: 7, label: "New note".into(), enabled: true }, TrayItem::Separator],
        )));
        app.on_tray(move |e| e2.borrow_mut().push(e));
        app.on_second_instance(move |a| l2.borrow_mut().push(a));
        let p3 = p2.clone();
        app.file_dialog(
            None,
            {
                let mut v_ = FileDialog::default();
                v_.title = "Export".into();
                v_.save = true;
                v_
            },
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
fn cancelled_background_work_stops_and_never_reports() {
    let reported = Rc::new(Cell::new(0u32));
    let r2 = reported.clone();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let (seen_tx, seen_rx) = std::sync::mpsc::channel::<bool>();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        let job = app.background_cancellable(
            move |cancelled| {
                // Waits until the UI has cancelled, as a long search would notice between steps.
                let _ = go_rx.recv();
                let _ = seen_tx.send(cancelled.is_cancelled());
                42u32
            },
            move |_| r2.set(r2.get() + 1),
        );
        job.cancel();
        assert!(job.is_cancelled());
        let _ = go_tx.send(());
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    // However busy the machine, the work finishes; then the UI gets every chance to hear it.
    let seen = seen_rx.recv_timeout(Duration::from_secs(30));
    assert_eq!(seen, Ok(true), "the work saw it was cancelled");
    for _ in 0..10 {
        host.run_frames(&mut runner, 1);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(reported.get(), 0, "a cancelled job's result never reaches the UI");
}

#[test]
fn the_tray_follows_what_it_reads() {
    use remedy_ui::host::{TrayIcon, TrayImage};
    let unread = Signal::new(1u32);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        app.tray_dyn(move || {
            let n = unread.get();
            Some(TrayIcon::new(format!("{n} unread"), TrayImage { size: 1, rgba: vec![255; 4] }, Vec::new()))
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    assert_eq!(host.tray().map(|t| t.tooltip.clone()), Some("1 unread".to_owned()));
    unread.set(5);
    host.run_frames(&mut runner, 2);
    assert_eq!(host.tray().map(|t| t.tooltip.clone()), Some("5 unread".to_owned()));
    runner.app().set_tray(None);
    unread.set(6);
    host.run_frames(&mut runner, 2);
    assert!(host.tray().is_none(), "set_tray replaces the dynamic tray, which stops following");
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
    let mut s = {
        let mut v_ = remedy_ui::host::SystemSettings::default();
        v_.locale = "de-DE".into();
        v_
    };
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
                remedy_ui::widgets::text(ui, "").text(move || format!("frames {}", count.get()))
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
    // In the window's language, which it follows as it changes.
    main_ui.set_locale("de");
    host.run_frames(&mut runner, 1);
    let de = texts(&main_ui);
    assert!(de.iter().any(|t| t.contains("ist in einem eigenen Fenster")), "{de:?}");
    assert!(de.iter().any(|t| t == "Zurückholen"), "{de:?}");
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
    let f12 = KeyEvent::press(Key::F(12)).with_physical("F12");
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
    let tab = KeyEvent::press(Key::Tab).with_physical("Tab");
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
    let results: Rc<std::cell::RefCell<Vec<Result<(), remedy_ui::host::Unsupported>>>> = Rc::default();
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
    let mut s = {
        let mut v_ = remedy_ui::host::SystemSettings::default();
        v_.dark = Some(false);
        v_
    };
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

fn red_block_app() -> Runner {
    Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.open(WindowSpec::new("Block").size(100.0, 100.0), |ui, _| {
            let block = ui
                .el("block")
                .layout(|l| {
                    l.width = Length::Px(40.0);
                    l.height = Length::Px(40.0);
                })
                .style(StylePatch::default().background(RED));
            ui.root().child(block);
        });
    })
}

#[test]
fn content_keeps_clear_of_the_safe_area() {
    let mut runner = red_block_app();
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    assert_eq!(px(&host, id, 2, 2), [255, 0, 0, 255], "without a safe area the block starts at the corner");
    let area = remedy_ui::host::SafeArea { top: 20.0, right: 0.0, bottom: 10.0, left: 8.0 };
    host.send(HostEvent::SafeArea(id, area));
    host.run_frames(&mut runner, 2);
    assert_ne!(px(&host, id, 2, 2), [255, 0, 0, 255], "the covered corner shows only the background");
    assert_eq!(px(&host, id, 10, 22), [255, 0, 0, 255], "the block starts inside the safe area");
    let ui = runner.app().windows().first().and_then(|w| w.ui());
    assert_eq!(ui.map(|u| u.safe_area()), Some(area));
}

#[test]
fn an_unclaimed_back_leaves_the_window() {
    let mut runner = red_block_app();
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    let back = |pressed| KeyEvent::press(Key::Back).with_physical("BrowserBack").with_pressed(pressed);
    host.send(HostEvent::Key(id, back(true)));
    host.send(HostEvent::Key(id, back(false)));
    host.run_frames(&mut runner, 1);
    assert!(host.window(id).is_none(), "nothing took Back, so the window closed");
}

#[test]
fn dialogs_that_answer_later_reach_the_app() {
    let picked: Rc<std::cell::RefCell<Option<Vec<std::path::PathBuf>>>> = Rc::default();
    let p2 = picked.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Dialogs").size(100.0, 100.0), |_, _| {});
        app.file_dialog(None, remedy_ui::host::FileDialog::default(), move |p| *p2.borrow_mut() = Some(p));
    });
    let mut host = HeadlessHost::new();
    host.dialogs_answer_later(true);
    host.run_frames(&mut runner, 1);
    assert_eq!(host.dialogs().len(), 1);
    assert!(picked.borrow().is_none(), "the picker is still open");
    let chosen = vec![std::path::PathBuf::from("/cache/documents/1/notes.txt")];
    host.send(HostEvent::FileDialog(remedy_ui::host::DialogId(1), chosen.clone()));
    host.run_frames(&mut runner, 1);
    assert_eq!(*picked.borrow(), Some(chosen));
}

#[test]
fn permission_answers_reach_the_app_now_or_later() {
    use remedy_ui::host::{Permission, PermissionId};
    let answers: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let a2 = answers.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Permissions").size(100.0, 100.0), |_, _| {});
        for _ in 0..3 {
            let a = a2.clone();
            app.request_permission(Permission::Microphone, move |g| a.borrow_mut().push(g));
        }
    });
    // A desktop platform answers at once; a scripted refusal is passed on as it is.
    let mut host = HeadlessHost::new();
    host.answer_permission(false);
    host.run_frames(&mut runner, 1);
    assert_eq!(host.permission_requests(), &[Permission::Microphone; 3]);
    assert_eq!(*answers.borrow(), vec![false, true, true]);

    // Android's prompt answers later, in any order.
    answers.borrow_mut().clear();
    let a3 = answers.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Permissions").size(100.0, 100.0), |_, _| {});
        for _ in 0..2 {
            let a = a3.clone();
            app.request_permission(Permission::Microphone, move |g| a.borrow_mut().push(g));
        }
    });
    let mut host = HeadlessHost::new();
    host.permissions_answer_later(true);
    host.run_frames(&mut runner, 1);
    assert!(answers.borrow().is_empty(), "the prompt is still up");
    host.send(HostEvent::Permission(PermissionId(2), true));
    host.send(HostEvent::Permission(PermissionId(1), false));
    host.send(HostEvent::Permission(PermissionId(1), true));
    host.run_frames(&mut runner, 1);
    assert_eq!(*answers.borrow(), vec![true, false], "each answer reaches its own request, once");
}

#[test]
fn notifications_carry_the_app_name_and_answers_reach_the_app() {
    use remedy_ui::host::{Notification, NotificationResponse};
    let heard: Rc<std::cell::RefCell<Vec<(u64, NotificationResponse)>>> = Rc::default();
    let outcomes: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let (h2, o2) = (heard.clone(), outcomes.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Notes").size(100.0, 100.0), |_, _| {});
        let h = h2.clone();
        app.on_notification(move |id, r| h.borrow_mut().push((id, r)));
        let o = o2.clone();
        app.notify(Notification::new(7, "Ada", "New message").action("reply", "Reply"), move |r| {
            o.borrow_mut().push(r.is_ok());
        });
        app.withdraw_notification(3);
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let shown = host.notifications();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown.first().map(|n| (n.id, n.title.as_str(), n.app.is_empty())), Some((7, "Ada", false)));
    assert_eq!(shown.first().map(|n| n.actions.len()), Some(1));
    assert_eq!(host.withdrawn_notifications(), &[3]);
    assert_eq!(*outcomes.borrow(), vec![true]);
    host.send(HostEvent::Notification(7, NotificationResponse::Action("reply".into())));
    host.send(HostEvent::Notification(7, NotificationResponse::Activated));
    host.run_frames(&mut runner, 1);
    assert_eq!(
        *heard.borrow(),
        vec![(7, NotificationResponse::Action("reply".into())), (7, NotificationResponse::Activated)]
    );

    // Turned off: the app hears that it was not shown.
    let o3 = outcomes.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Notes").size(100.0, 100.0), |_, _| {});
        let o = o3.clone();
        app.notify(Notification::new(1, "Ada", ""), move |r| o.borrow_mut().push(r.is_ok()));
    });
    let mut host = HeadlessHost::new();
    host.notifications_off(true);
    host.run_frames(&mut runner, 1);
    assert_eq!(*outcomes.borrow(), vec![true, false]);
}

#[test]
fn a_notification_press_that_started_the_app_reaches_it_after_setup() {
    use remedy_ui::host::NotificationResponse;
    let heard: Rc<std::cell::RefCell<Vec<(u64, NotificationResponse)>>> = Rc::default();
    let h2 = heard.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Notes").size(100.0, 100.0), |_, _| {});
        let h = h2.clone();
        app.on_notification(move |id, r| h.borrow_mut().push((id, r)));
    });
    let mut host = HeadlessHost::new();
    // Android reports the press that started the app before the app's setup runs.
    host.send(HostEvent::Notification(7, NotificationResponse::Activated));
    host.run_frames(&mut runner, 2);
    assert_eq!(*heard.borrow(), vec![(7, NotificationResponse::Activated)], "kept until the app listens");
    host.send(HostEvent::Notification(7, NotificationResponse::Dismissed));
    host.run_frames(&mut runner, 1);
    assert_eq!(heard.borrow().len(), 2, "later ones arrive directly, once");
}

#[test]
fn foreground_work_starts_stops_and_hears_when_the_platform_ends_it() {
    use remedy_ui::host::{ForegroundKind, ForegroundTask, Notification, NotificationResponse};
    let outcomes: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let stopped: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let heard: Rc<std::cell::RefCell<Vec<(u64, NotificationResponse)>>> = Rc::default();
    let (o2, s2, h2) = (outcomes.clone(), stopped.clone(), heard.clone());
    let app_slot: Rc<std::cell::RefCell<Option<App>>> = Rc::default();
    let a2 = app_slot.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Call").size(100.0, 100.0), |_, _| {});
        let s = s2.clone();
        app.on_foreground_stopped(move |why| s.borrow_mut().push(why));
        let h = h2.clone();
        app.on_notification(move |id, r| h.borrow_mut().push((id, r)));
        let task = ForegroundTask::new(
            ForegroundKind::Microphone,
            Notification::new(40, "In a call", "Ada").action("hang-up", "Hang up"),
        )
        .kind(ForegroundKind::MediaPlayback)
        .kind(ForegroundKind::Microphone);
        let o = o2.clone();
        app.start_foreground(task, move |r| o.borrow_mut().push(r.is_ok()));
        *a2.borrow_mut() = Some(app.clone());
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    assert_eq!(*outcomes.borrow(), vec![true]);
    let task = host.foreground().cloned().unwrap_or_else(|| unreachable!("started"));
    assert_eq!(task.kinds, vec![ForegroundKind::Microphone, ForegroundKind::MediaPlayback], "each kind once");
    assert!(!task.notification.app.is_empty(), "the engine names the app");
    assert_eq!(task.notification.actions.len(), 1);

    // Its notification's button reaches the app like any notification's.
    host.send(HostEvent::Notification(40, NotificationResponse::Action("hang-up".into())));
    host.run_frames(&mut runner, 1);
    assert_eq!(*heard.borrow(), vec![(40, NotificationResponse::Action("hang-up".into()))]);

    let app = app_slot.borrow().clone().unwrap_or_else(|| unreachable!("set up"));
    app.stop_foreground();
    host.run_frames(&mut runner, 1);
    assert!(host.foreground().is_none(), "stopped");

    // Refused (out of sight, a permission missing): the app hears it.
    host.refuse_foreground(Some("start microphone work while the app is on screen"));
    let o3 = outcomes.clone();
    app.start_foreground(
        ForegroundTask::new(ForegroundKind::Microphone, Notification::new(40, "In a call", "")),
        move |r| {
            o3.borrow_mut().push(r.is_ok());
        },
    );
    host.run_frames(&mut runner, 1);
    assert_eq!(*outcomes.borrow(), vec![true, false]);
    assert!(host.foreground().is_none());

    // Started, then ended by the platform: the app is told why.
    host.refuse_foreground(None);
    app.start_foreground(
        ForegroundTask::new(ForegroundKind::DataSync, Notification::new(41, "Uploading", "")),
        |_| {},
    );
    host.run_frames(&mut runner, 1);
    assert!(host.foreground().is_some());
    host.send(HostEvent::ForegroundStopped("the time limit for data sync ran out".into()));
    host.run_frames(&mut runner, 1);
    assert_eq!(*stopped.borrow(), vec!["the time limit for data sync ran out".to_owned()]);
    assert!(host.foreground().is_none());
}

#[test]
fn shares_reach_the_sheet_and_how_they_ended_reaches_the_app() {
    use remedy_ui::host::{ShareFile, ShareOptions, ShareOutcome, Unsupported};
    let outcomes: Rc<std::cell::RefCell<Vec<ShareOutcome>>> = Rc::default();
    let o2 = outcomes.clone();
    let recipe = ShareOptions::new()
        .title("Lavender")
        .text("Lavender soap: 500 g oils")
        .file(ShareFile::new("Lavender.alex-soap.json", "application/json", b"{}".to_vec()));
    let r2 = recipe.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let window = app.open(WindowSpec::new("Share").size(100.0, 100.0), |_, _| {});
        for share in [r2.clone(), r2.clone(), r2, ShareOptions::new()] {
            let o = o2.clone();
            app.share(Some(&window), share, move |outcome| o.borrow_mut().push(outcome));
        }
    });
    let mut host = HeadlessHost::new();
    host.answer_share(ShareOutcome::Cancelled);
    let refused = Unsupported::new("sharing", "scripted");
    host.answer_share(ShareOutcome::Unsupported(refused.clone()));
    host.run_frames(&mut runner, 1);
    let window = host.window_ids().first().copied();
    assert!(window.is_some());
    assert_eq!(host.shares().len(), 3, "an empty share never reaches the sheet");
    assert!(host.shares().iter().all(|(w, s)| *w == window && *s == recipe), "{:?}", host.shares());
    let got = outcomes.borrow().clone();
    assert_eq!(
        got.get(..3),
        Some(&[ShareOutcome::Cancelled, ShareOutcome::Unsupported(refused), ShareOutcome::Shared][..])
    );
    assert!(
        matches!(got.get(3), Some(ShareOutcome::Unsupported(u)) if u.reason.contains("nothing to share")),
        "{got:?}"
    );

    // Android's chooser closes later, in any order.
    outcomes.borrow_mut().clear();
    let o3 = outcomes.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Share").size(100.0, 100.0), |_, _| {});
        for n in 0..2 {
            let o = o3.clone();
            app.share(None, ShareOptions::new().text(format!("share {n}")), move |outcome| {
                o.borrow_mut().push(outcome)
            });
        }
    });
    let mut host = HeadlessHost::new();
    host.shares_answer_later(true);
    host.run_frames(&mut runner, 1);
    assert!(outcomes.borrow().is_empty(), "the sheets are still open");
    assert_eq!(host.shares().iter().map(|(w, _)| *w).collect::<Vec<_>>(), vec![None, None]);
    use remedy_ui::host::ShareId;
    host.send(HostEvent::Share(ShareId(2), ShareOutcome::Shared));
    host.send(HostEvent::Share(ShareId(1), ShareOutcome::Cancelled));
    host.send(HostEvent::Share(ShareId(1), ShareOutcome::Shared));
    host.run_frames(&mut runner, 1);
    assert_eq!(
        *outcomes.borrow(),
        vec![ShareOutcome::Shared, ShareOutcome::Cancelled],
        "each outcome reaches its own share, once"
    );
}

#[test]
fn suspended_windows_draw_again_after_resume() {
    let mut runner = red_block_app();
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    host.send(HostEvent::Suspended);
    host.run_frames(&mut runner, 1);
    let before = host.window(id).map_or(0, |w| w.presents());
    host.run_frames(&mut runner, 2);
    assert_eq!(host.window(id).map_or(0, |w| w.presents()), before, "nothing presents while suspended");
    host.send(HostEvent::Resumed);
    host.run_frames(&mut runner, 2);
    assert!(host.window(id).map_or(0, |w| w.presents()) > before, "the window draws again");
    assert_eq!(px(&host, id, 2, 2), [255, 0, 0, 255]);
}

#[test]
fn windows_fill_the_screen_and_say_so() {
    let handle: Rc<std::cell::RefCell<Option<remedy_ui::Window>>> = Rc::default();
    let h = handle.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let h = h.clone();
        app.open(WindowSpec::new("Player"), move |_, window| {
            window.set_fullscreen(true);
            *h.borrow_mut() = Some(window.clone());
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    let window = handle.borrow().clone().unwrap_or_else(|| unreachable!("the window"));
    assert!(host.window(id).is_some_and(|w| w.is_fullscreen()));
    assert!(window.is_fullscreen());
    window.set_fullscreen(false);
    host.run_frames(&mut runner, 1);
    assert!(host.window(id).is_some_and(|w| !w.is_fullscreen()));
    assert!(!window.is_fullscreen());
}

#[test]
fn the_primary_selection_is_set_and_pasted_through_the_host() {
    let got: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let g = got.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let g = g.clone();
        app.open(WindowSpec::new("Primary"), move |ui, _window| {
            let g = g.clone();
            let el = ui.el("canvas").width(100.0).height(40.0).on_text_input(
                remedy_ui_core::Rect::default,
                move |input| {
                    if let remedy_ui::view::TextInput::Paste(t) = input {
                        g.borrow_mut().push(t.clone());
                    }
                },
            );
            ui.root().child(el.clone());
            el.focus();
            ui.set_primary_selection("selected here");
            ui.request_paste_primary(Some("fallback".into()));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert_eq!(host.primary_selection(), Some("selected here"));
    assert_eq!(*got.borrow(), ["selected here"], "the middle button's paste reads the primary selection");
}

#[cfg(feature = "customize")]
#[test]
fn customizations_live_in_the_apps_data_folder() {
    use remedy_ui::customize::{Token, TokenValue};
    let dir = std::env::temp_dir().join(format!("remedy-ui-app-customize-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    /// The data folder the app reported, and whether a file picker was wired.
    type Seen = Option<(Option<std::path::PathBuf>, bool)>;
    let seen: Rc<std::cell::RefCell<Seen>> = Rc::default();
    let s2 = seen.clone();
    let mut runner = Runner::new(config().data_dir(&dir), move |app: &App| {
        let Some(c) = app.customizations() else { return };
        *s2.borrow_mut() = Some((app.data_dir(), c.has_file_picker()));
        c.enable_theme(remedy_ui::style::Theme::dark_forest());
        app.open(WindowSpec::new("Customized").size(200.0, 100.0), move |ui, _| {
            c.attach(ui);
            let _ = c.set_token(ui, &Token::stroke(), TokenValue::Px(2.0));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let got = seen.borrow().clone();
    assert_eq!(
        got,
        Some((Some(dir.clone()), true)),
        "the configured folder, and the platform's file dialogs"
    );
    let theme = std::fs::read_to_string(dir.join("customizations").join("theme.json")).unwrap_or_default();
    assert!(theme.contains("\"stroke\""), "saved in the app's data folder: {theme}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persisted_signals_in_a_window_come_back_after_a_restart() {
    let dir = std::env::temp_dir().join(format!("remedy-ui-app-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let seen: Rc<std::cell::RefCell<Vec<f32>>> = Rc::default();
    let start = |seen: Rc<std::cell::RefCell<Vec<f32>>>, change: bool| {
        Runner::new(config().data_dir(&dir), move |app: &App| {
            let app2 = app.clone();
            app.open(WindowSpec::new("Split").size(200.0, 100.0), move |_, _| {
                // The window's content is built with the app's store as context.
                let split = Signal::new(0.5f32).persist("split");
                seen.borrow_mut().push(split.get());
                if change {
                    split.set(0.25);
                    app2.quit();
                }
            });
        })
    };
    let mut runner = start(seen.clone(), true);
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert!(dir.join("state").join("split.json").exists(), "written by the time the app quit");
    let mut again = start(seen.clone(), false);
    let mut host = HeadlessHost::new();
    host.run_frames(&mut again, 2);
    assert_eq!(*seen.borrow(), [0.5, 0.25], "restored in the next run");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_frame_runs_on_the_frame_clock() {
    let times: Rc<std::cell::RefCell<Vec<Duration>>> = Rc::default();
    let t2 = times.clone();
    let mut runner = Runner::new(config(), move |app: &App| {
        let t3 = t2.clone();
        app.open(WindowSpec::new("Clock").size(100.0, 100.0), move |ui, _| {
            let canvas = ui.el("canvas").width(20.0).height(20.0).paint(move |cx| {
                t3.borrow_mut().push(cx.time);
                cx.request_next_frame();
            });
            ui.root().child(canvas);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 4);
    let seen = times.borrow().clone();
    assert!(seen.len() >= 2, "painted every frame it asked for: {seen:?}");
    assert!(seen.windows(2).all(|w| matches!(w, [a, b] if b > a)), "the clock moves on each frame: {seen:?}");
}

#[test]
fn a_zoomed_window_draws_larger_and_maps_clicks_back() {
    let count = Signal::new(0u32);
    let win: Rc<std::cell::RefCell<Option<remedy_ui::Window>>> = Rc::default();
    let w2 = win.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let w = app.open(WindowSpec::new("Zoom").size(200.0, 100.0), move |ui, _| {
            let button = ui
                .el("button")
                .layout(|l| {
                    l.width = Length::Px(40.0);
                    l.height = Length::Px(20.0);
                })
                .on_click(move || count.update(|c| *c += 1));
            ui.root().child(button);
        });
        *w2.borrow_mut() = Some(w);
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    let before = px(&host, id, 60, 30);
    assert_ne!(before, [255, 0, 0, 255], "outside the 40 × 20 button at 1×");
    let Some(w) = win.borrow().clone() else { unreachable!("open") };
    w.set_zoom(2.0);
    assert_eq!(w.zoom(), 2.0);
    host.run_frames(&mut runner, 2);
    assert_eq!(px(&host, id, 60, 30), [255, 0, 0, 255], "the button is 80 × 40 at 2×");
    let ui = w.ui();
    assert_eq!(
        ui.map(|u| u.window_size()),
        Some(Size::new(100.0, 50.0)),
        "content lays out in half the size"
    );
    // A click where the button is drawn reaches it.
    host.send(HostEvent::Pointer(id, press(70.0, 30.0, PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, press(70.0, 30.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert_eq!(count.get(), 1);
}

#[test]
fn a_tray_app_knows_its_icon_is_up_and_can_minimize_on_close_instead() {
    use remedy_ui::host::{TrayIcon, TrayImage};
    let app_seen: Rc<std::cell::RefCell<Option<App>>> = Rc::default();
    let a2 = app_seen.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        *a2.borrow_mut() = Some(app.clone());
        app.open(WindowSpec::new("Tray app").minimize_on_close(), |_, _| {});
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let app = app_seen.borrow().clone().unwrap_or_else(|| unreachable!("the app"));
    assert!(!app.tray_shown(), "no icon asked for");
    app.set_tray(Some(TrayIcon::new("Tray app", TrayImage { size: 2, rgba: vec![255; 16] }, Vec::new())));
    host.run_frames(&mut runner, 1);
    assert!(app.tray_shown(), "the host took the icon");
    app.set_tray(None);
    host.run_frames(&mut runner, 1);
    assert!(!app.tray_shown());
    let id = first(&host);
    host.send(HostEvent::CloseRequested(id));
    host.run_frames(&mut runner, 1);
    assert!(
        host.window(id).is_some_and(|w| w.is_minimized() && w.options().visible),
        "minimized, not hidden or closed"
    );
}

#[test]
fn a_frame_rate_limit_holds_frames_back_until_their_turn() {
    let n = Signal::new(0u32);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.set_frame_rate_limit(Some(10));
        app.open(WindowSpec::new("Ten a second"), move |ui, _| {
            ui.root().child(ui.el("label").text(move || format!("{}", n.get())));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    let before = host.window(id).map_or(0, |w| w.presents());
    // A change every refresh for well under a tenth of a second: one frame at most.
    for _ in 0..4 {
        n.update(|v| *v += 1);
        host.run_frames(&mut runner, 1);
    }
    let soon = host.window(id).map_or(0, |w| w.presents());
    assert!(soon <= before + 1, "{before} -> {soon}");
    // Its turn comes: the held frame is drawn.
    host.run_frames(&mut runner, 10);
    assert!(host.window(id).map_or(0, |w| w.presents()) > soon, "the last change is shown");
}

#[test]
fn an_app_can_reduce_motion_where_the_system_does_not() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.open(WindowSpec::new("Calm"), |_, _| {});
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let reduced = |runner: &Runner| {
        runner.app().windows().first().and_then(|w| w.ui()).is_some_and(|ui| ui.reduced_motion())
    };
    assert!(!reduced(&runner), "the system does not ask for it");
    runner.app().set_reduce_motion(true);
    host.run_frames(&mut runner, 1);
    assert!(reduced(&runner), "the app's setting holds still");
    runner.app().set_reduce_motion(false);
    host.run_frames(&mut runner, 1);
    assert!(!reduced(&runner), "off follows the system again");
}

#[test]
fn a_file_dialog_tells_a_cancel_from_no_dialog() {
    use remedy_ui::host::FileDialog;
    let got: Rc<std::cell::RefCell<Option<bool>>> = Rc::default();
    let g2 = got.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Files"), |_, _| {});
        let g3 = g2.clone();
        app.try_file_dialog(None, FileDialog::default(), move |r| {
            *g3.borrow_mut() = Some(matches!(r, Ok(ref p) if p.is_empty()))
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert_eq!(*got.borrow(), Some(true), "cancelled: Ok with nothing chosen");
}

#[test]
fn hide_on_close_can_change_while_the_window_is_open() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        let w = app.open(WindowSpec::new("Tray app"), |_, _| {});
        w.set_hide_on_close(true);
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let id = first(&host);
    host.send(HostEvent::CloseRequested(id));
    host.run_frames(&mut runner, 1);
    assert!(host.window(id).is_some_and(|w| !w.options().visible), "hidden, not closed");
    if let Some(w) = runner.app().windows().into_iter().next() {
        w.set_hide_on_close(false);
    }
    host.run_frames(&mut runner, 1);
    host.send(HostEvent::CloseRequested(id));
    host.run_frames(&mut runner, 1);
    assert!(host.window(id).is_none(), "closed once it no longer hides");
}

/// The app's own signals outlive the binding that first asked for them: `tray_shown` read
/// first inside a binding (a window's build, an effect) must still be there after that
/// binding runs again, and so must a window opened from inside one.
#[test]
fn app_signals_outlive_the_binding_that_first_asked_for_them() {
    let tick = Signal::new(0u32);
    let seen = Rc::new(Cell::new(0u32));
    let s2 = seen.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.keep_running(true);
        let app2 = app.clone();
        app.open(WindowSpec::new("Tray app"), move |ui, _| {
            let (app3, s3) = (app2.clone(), s2.clone());
            let _ = ui.root().bind(move || {
                tick.get();
                // First asked for here, inside the binding.
                let _ = app3.tray_shown();
                s3.set(s3.get() + 1);
                if s3.get() == 1 {
                    // A window opened from inside the binding keeps its signals too.
                    app3.open(WindowSpec::new("Inner"), |_, _| {});
                }
            });
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    for n in 1..=3 {
        tick.set(n);
        host.run_frames(&mut runner, 1);
    }
    assert!(seen.get() >= 4, "the binding ran again ({} runs)", seen.get());
    assert!(!runner.app().tray_shown());
    for w in runner.app().windows() {
        assert!(!w.is_maximized(), "a window's own signals are still there");
    }
}

/// A window shown again, or one the platform says is back in sight, is presented whole: the
/// platform may not have kept what it showed (X11 forgets an unmapped window's pixels), and a
/// present of only what changed would leave the rest blank.
#[test]
fn a_window_back_in_sight_is_presented_whole() {
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), |app: &App| {
        app.keep_running(true);
        app.open(WindowSpec::new("Tray app").size(320.0, 200.0), |ui, _| {
            let _ = ui.root().child(remedy_ui::widgets::text(ui, "Hello"));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 3);
    let id = host.window_ids().first().copied().unwrap_or_else(|| unreachable!("a window"));
    let whole = |host: &HeadlessHost| {
        let w = host.window(id).unwrap_or_else(|| unreachable!("open"));
        let area: f32 = w.last_damage().iter().map(|r| r.width() * r.height()).sum();
        (area, w.presents())
    };
    let (_, before) = whole(&host);
    // Shown again by the app (from the tray).
    let w = runner.app().windows().into_iter().next().unwrap_or_else(|| unreachable!("open"));
    w.set_visible(false);
    host.run_frames(&mut runner, 1);
    w.set_visible(true);
    host.run_frames(&mut runner, 2);
    let (area, after) = whole(&host);
    assert!(after > before, "it was presented again");
    assert!(area >= 320.0 * 200.0, "all of it ({area} pixels of damage)");
    // Back in sight as the platform says (an X11 expose).
    host.send(HostEvent::Occluded(id, false));
    host.run_frames(&mut runner, 2);
    let (area, again) = whole(&host);
    assert!(again > after, "presented again");
    assert!(area >= 320.0 * 200.0, "all of it ({area} pixels of damage)");
}

#[test]
fn printed_pages_reach_the_print_dialog() {
    let done: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
    let d2 = done.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let window = app.open(WindowSpec::new("Print").size(100.0, 100.0), |_, _| {});
        let d = d2.clone();
        app.print_html(Some(&window), "Soap Recipe", "<html><body>Olive 500 g</body></html>", move |r| {
            d.borrow_mut().push(r.is_ok())
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    let window = host.window_ids().first().copied();
    assert_eq!(host.prints().len(), 1);
    let first = host.prints().first();
    assert_eq!(first.map(|(w, title, _)| (*w, title.as_str())), Some((window, "Soap Recipe")));
    assert!(first.is_some_and(|(_, _, html)| html.contains("Olive 500 g")));
    assert_eq!(*done.borrow(), [true]);
}

#[test]
fn a_finger_swipe_scrolls_and_a_tap_still_clicks() {
    let taps = Signal::new(0u32);
    let list_el = std::rc::Rc::new(std::cell::RefCell::new(None));
    let l2 = list_el.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let l3 = l2.clone();
        app.open(WindowSpec::new("Touch").size(200.0, 200.0), move |ui, _| {
            let mut list = ui.el("scroll").scroll().column().layout(|l| {
                l.grow = 1.0;
                l.min_height = Length::Px(0.0);
            });
            for _ in 0..30 {
                list = list.child(ui.el("button").height(40.0).on_click(move || taps.update(|t| *t += 1)));
            }
            *l3.borrow_mut() = Some(list.clone());
            ui.root().child(list);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    let ui = runner.app().windows().first().and_then(|w| w.ui()).unwrap_or_else(|| unreachable!("a window"));
    let list = list_el.borrow().clone().unwrap_or_else(|| unreachable!("the list"));
    let finger = |y: f32, phase: PointerPhase| {
        press(100.0, y, phase).with_id(1).with_kind(remedy_ui::input::PointerKind::Touch)
    };
    // A swipe up over the buttons scrolls the list and clicks nothing.
    host.send(HostEvent::Pointer(id, finger(150.0, PointerPhase::Down)));
    for y in [140.0, 120.0, 100.0, 80.0, 60.0] {
        host.send(HostEvent::Pointer(id, finger(y, PointerPhase::Move)));
        host.run_frames(&mut runner, 1);
    }
    host.send(HostEvent::Pointer(id, finger(60.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 2);
    assert!(ui.scroll_offset(list.id()).y >= 80.0, "offset {:?}", ui.scroll_offset(list.id()));
    assert_eq!(taps.get(), 0, "a swipe is not a tap");
    // A tap still clicks.
    host.send(HostEvent::Pointer(id, finger(100.0, PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, finger(102.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert_eq!(taps.get(), 1);
    // A long press is a right-click, not a tap.
    let menus = Rc::new(Cell::new(0u32));
    let m2 = menus.clone();
    let _ = list.clone().on_pointer(move |e| {
        if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::SECONDARY) {
            m2.set(m2.get() + 1);
        }
        false
    });
    host.send(HostEvent::Pointer(id, finger(100.0, PointerPhase::Down)));
    host.run_frames(&mut runner, 1);
    assert_eq!(menus.get(), 0, "not yet");
    host.advance(Duration::from_millis(600));
    host.run_frames(&mut runner, 1);
    assert_eq!(menus.get(), 1, "the menu opens while the finger is still down");
    host.send(HostEvent::Pointer(id, finger(100.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert_eq!((menus.get(), taps.get()), (1, 1), "lifting after a long press is not a tap");
}

#[test]
fn a_finger_tap_on_a_field_opens_the_keyboard() {
    let field_el = Rc::new(std::cell::RefCell::new(None));
    let f2 = field_el.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let f3 = f2.clone();
        app.open(WindowSpec::new("Form").size(200.0, 200.0), move |ui, _| {
            let field = ui.text_field("text_input", FieldKind::Single).el().height(40.0);
            *f3.borrow_mut() = Some(field.clone());
            ui.root().child(field);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    let field = field_el.borrow().clone().unwrap_or_else(|| unreachable!("the field"));
    let b = field.bounds().unwrap_or_default();
    let (x, y) = ((b.min.x + b.max.x) * 0.5, (b.min.y + b.max.y) * 0.5);
    assert!(b.width() > 0.0 && b.height() > 0.0, "the field is laid out: {b:?}");
    let finger = |phase| {
        press(x, y, phase).with_id(1).with_kind(remedy_ui::input::PointerKind::Touch)
    };
    host.send(HostEvent::Pointer(id, finger(PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, finger(PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert!(field.is_focused(), "a tap focuses the field");
    assert!(
        host.window(id).is_some_and(|w| w.text_input().is_some()),
        "focus opened a text-input session"
    );
}

#[test]
fn a_rising_keyboard_scrolls_the_focused_field_above_it() {
    let field_el = Rc::new(std::cell::RefCell::new(None));
    let f2 = field_el.clone();
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let f3 = f2.clone();
        app.open(WindowSpec::new("Form").size(200.0, 400.0), move |ui, _| {
            let mut list = ui.el("scroll").scroll().column().layout(|l| {
                l.grow = 1.0;
                l.min_height = Length::Px(0.0);
            });
            for _ in 0..8 {
                list = list.child(ui.el("button").height(40.0));
            }
            let field = ui.text_field("text_input", FieldKind::Single).el().height(30.0);
            *f3.borrow_mut() = Some(field.clone());
            ui.root().child(list.child(field).child(ui.el("button").height(400.0)));
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    let field = field_el.borrow().clone().unwrap_or_else(|| unreachable!("the field"));
    field.focus();
    host.run_frames(&mut runner, 2);
    let before = field.bounds().unwrap_or_default();
    assert!(before.max.y <= 400.0 && before.min.y >= 300.0, "starts low on screen: {before:?}");
    // The keyboard covers the bottom 200 px.
    host.send(HostEvent::SafeArea(
        id,
        remedy_ui::host::SafeArea { top: 0.0, right: 0.0, bottom: 200.0, left: 0.0 },
    ));
    host.run_frames(&mut runner, 3);
    let after = field.bounds().unwrap_or_default();
    assert!(after.max.y <= 200.5, "above the keyboard: {after:?}");
}

#[test]
#[cfg(feature = "desktop")]
fn the_badge_names_the_app_and_links_reach_on_url() {
    use remedy_ui::host::Badge;
    use std::cell::RefCell;
    let urls = Rc::new(RefCell::new(Vec::<String>::new()));
    let launches = Rc::new(RefCell::new(Vec::<Vec<String>>::new()));
    let (u2, l2) = (urls.clone(), launches.clone());
    let mut runner = Runner::with_geometry(
        config().app_id("com.example.hearth"),
        GeometryStore::in_memory(),
        move |app: &App| {
            app.set_badge(Badge::count(3));
            app.on_url(&["hearth"], move |u| u2.borrow_mut().push(u));
            app.on_second_instance(move |a| l2.borrow_mut().push(a));
        },
    );
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 1);
    assert_eq!(host.badge(), Some(("com.example.hearth", Badge::Count(3))));
    runner.app().set_badge(None);
    host.run_frames(&mut runner, 1);
    assert_eq!(host.badge(), None);

    // A link opened while the app runs comes as a later launch's arguments (or, on macOS, the
    // bridge's "get URL" event in the same shape).
    host.send(HostEvent::SecondInstance(vec![
        "--quiet".into(),
        "HEARTH://note/42".into(),
        "https://x".into(),
    ]));
    host.run_frames(&mut runner, 1);
    assert_eq!(*urls.borrow(), vec!["HEARTH://note/42".to_owned()]);
    assert_eq!(launches.borrow().len(), 1, "the launch is still reported whole");
}

#[test]
fn an_element_hides_the_mouse_pointer_while_it_is_over_it() {
    let hide = Signal::new(true);
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        app.open(WindowSpec::new("Movie").size(200.0, 100.0), move |ui, _| {
            let picture = ui.el("box").layout(|l| {
                l.width = Length::Px(100.0);
                l.height = Length::Px(100.0);
            });
            let p = picture.clone();
            let picture = picture.bind(move || {
                let _ = p.clone().cursor_hidden(hide.get());
            });
            ui.root().row().child(picture);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    let hidden = |host: &HeadlessHost| host.window(id).is_some_and(|w| w.is_cursor_hidden());
    assert!(!hidden(&host), "not until the pointer is over it");
    host.send(HostEvent::Pointer(id, press(50.0, 50.0, PointerPhase::Move)));
    host.run_frames(&mut runner, 1);
    assert!(hidden(&host), "over it");
    host.send(HostEvent::Pointer(id, press(150.0, 50.0, PointerPhase::Move)));
    host.run_frames(&mut runner, 1);
    assert!(!hidden(&host), "beside it");
    host.send(HostEvent::Pointer(id, press(50.0, 50.0, PointerPhase::Move)));
    host.run_frames(&mut runner, 1);
    assert!(hidden(&host));
    // The element shows it again with no pointer movement (a timer, a signal).
    hide.set(false);
    host.run_frames(&mut runner, 1);
    assert!(!hidden(&host), "shown again where it rests");
}

#[test]
fn keys_and_clicks_sent_to_a_live_window_reach_it_as_the_platforms_do() {
    let count = Signal::new(0u32);
    let typed = Rc::new(std::cell::RefCell::new(String::new()));
    let win = Rc::new(std::cell::RefCell::new(None));
    let (t2, w2) = (typed.clone(), win.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let w = app.open(WindowSpec::new("Harness").size(200.0, 100.0), move |ui, _| {
            let button = ui
                .el("button")
                .layout(|l| {
                    l.width = Length::Px(60.0);
                    l.height = Length::Px(30.0);
                })
                .on_click(move || count.update(|c| *c += 1));
            let field = ui.text_field("text_input", FieldKind::Single);
            let f2 = field.clone();
            let field = field.on_change(move || *t2.borrow_mut() = f2.value());
            ui.root().child(button).child(field.el().layout(|l| l.width = Length::Px(150.0)));
            field.el().focus();
        });
        *w2.borrow_mut() = Some(w);
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let w = win.borrow().clone().unwrap_or_else(|| unreachable!("the window"));
    // Keys go where a keyboard's would: the focused field.
    w.press_key(KeyEvent::press(Key::Character("h".into())).with_text(Some("h".into())));
    w.press_key(KeyEvent::press(Key::Character("i".into())).with_text(Some("i".into())));
    host.run_frames(&mut runner, 1);
    assert_eq!(typed.borrow().as_str(), "hi");
    // A click on the button, by its place in the window.
    w.click_at(Point::new(10.0, 10.0));
    host.run_frames(&mut runner, 1);
    assert_eq!(count.get(), 1);
    // The window's picture as a PNG (`snapshot_png` comes with the devtools feature).
    #[cfg(feature = "devtools")]
    snapshot_png_matches_the_window(&mut host, &mut runner, &w);
}

#[cfg(feature = "devtools")]
fn snapshot_png_matches_the_window(host: &mut HeadlessHost, runner: &mut Runner, w: &remedy_ui::Window) {
    let png = Rc::new(std::cell::RefCell::new(None));
    let p2 = png.clone();
    w.snapshot_png(move |bytes| *p2.borrow_mut() = bytes);
    host.run_frames(runner, 1);
    let bytes = png.borrow_mut().take().unwrap_or_else(|| unreachable!("a PNG"));
    let mut reader =
        png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap_or_else(|e| unreachable!("{e}"));
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((info.width, info.height), (200, 100));
    let at = (10 * 200 + 10) * 4;
    assert_eq!(buf.get(at..at + 4), Some(&px(host, first(host), 10, 10)[..]), "the button, opaque");
}
