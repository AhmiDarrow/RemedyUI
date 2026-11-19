//! The recipes in `examples/`, run with [`TestApp`] on the in-memory host and checked through
//! the accessibility tree, as a screen reader or an agent would meet them.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::a11y::{Action, ReaderNode, Role};
use remedy_ui::core::Point;
use remedy_ui::host::{HostEvent, NotificationResponse, TrayEvent, TrayItem, WindowId};
use remedy_ui::input::{Buttons, Key, KeyEvent, PointerEvent, PointerPhase};
use remedy_ui::testing::TestApp;
use remedy_ui::{App, AppConfig, Ui, WindowSpec};

#[path = "../examples/audio_meter.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod audio_meter;
#[path = "../examples/chat_feed.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod chat_feed;
#[path = "../examples/document_viewer.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod document_viewer;
#[path = "../examples/file_explorer.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod file_explorer;
#[path = "../examples/settings_page.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod settings_page;
#[path = "../examples/sticky_notes.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod sticky_notes;
#[path = "../examples/tray_app.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod tray_app;
#[path = "../examples/unlock_screen.rs"]
#[allow(dead_code, reason = "the example's `main` opens a real window; the test uses its `app`")]
mod unlock_screen;

/// Unwraps what a test needs, failing it with `why`.
trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

/// Starts a recipe on the in-memory host, settled for a few frames. Its data (persisted
/// signals) goes to the test app's own temporary folder, removed with it.
fn start(name: &str, app: impl FnOnce(&App) + 'static) -> TestApp {
    let mut t = TestApp::new(AppConfig::new(name), app);
    t.frames(2);
    t
}

/// Starts a recipe with its data folder at `data` (to start it again on the same data).
fn start_in(name: &str, data: &std::path::Path, app: impl FnOnce(&App) + 'static) -> TestApp {
    let mut t = TestApp::new(AppConfig::new(name).data_dir(data), app);
    t.frames(2);
    t
}

/// The first open window's view.
fn ui(t: &TestApp) -> Ui {
    t.ui().must("a window is open")
}

/// The first window's host id.
fn window(t: &mut TestApp) -> WindowId {
    t.host().window_ids().first().copied().must("a window is open")
}

/// The accessible names in the window, in reading order.
fn names(ui: &Ui) -> Vec<String> {
    ui.reader_view().into_iter().filter_map(|n| n.name).collect()
}

/// Whether some node's name contains `text`.
fn has(ui: &Ui, text: &str) -> bool {
    names(ui).iter().any(|n| n.contains(text))
}

/// The node with `role` and `name`, as a screen reader meets it.
fn node(ui: &Ui, role: Role, name: &str) -> Option<ReaderNode> {
    ui.reader_view().into_iter().find(|n| n.role == role && n.name.as_deref() == Some(name))
}

/// The value of the node with `role` and `name`.
fn value(ui: &Ui, role: Role, name: &str) -> Option<String> {
    node(ui, role, name).and_then(|n| n.value)
}

/// The states of the node with `role` and `name`, in words.
fn states(ui: &Ui, role: Role, name: &str) -> Vec<&'static str> {
    node(ui, role, name).map(|n| n.states).unwrap_or_default()
}

/// Clicks the element with `role` and `name` as a screen reader would, and lets it settle.
fn click(t: &mut TestApp, role: Role, name: &str) {
    assert!(t.press(role, name), "{role:?} {name:?} takes a click: {:?}", names(&ui(t)));
    t.frames(1);
}

/// Presses and releases the primary button at `p` in window `id`.
fn press_at(t: &mut TestApp, id: WindowId, p: Point) {
    let down = PointerEvent::new(PointerPhase::Down, p).with_buttons(Buttons::PRIMARY);
    t.host().send(HostEvent::Pointer(id, down));
    t.host().send(HostEvent::Pointer(id, PointerEvent::new(PointerPhase::Up, p)));
    t.frames(1);
}

/// A folder of its own under the system's temporary folder, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("remedy-ui-recipe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).must("a temporary folder");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn chat_feed_shows_the_latest_and_sends_from_the_composer() {
    let mut t = start("Chat feed", chat_feed::app);
    let ui = ui(&t);
    assert!(node(&ui, Role::List, "Messages").is_some(), "{:?}", names(&ui));
    assert!(has(&ui, "I'll write them after lunch."), "the newest message shows: {:?}", names(&ui));
    // Three messages leave the view at the top, so the list asked for older history.
    assert!(has(&ui, "Earlier note 40"), "older history loaded above: {:?}", names(&ui));
    let field = node(&ui, Role::MultilineTextInput, "Message").must("the composer").id;
    assert!(ui.accessibility_action(field, Action::Focus));
    assert!(ui.accessibility_set_value(field, "Shipping it now"));
    // Enter sends, though the composer takes several lines.
    t.key(KeyEvent::press(Key::Enter));
    t.frames(2);
    assert!(has(&ui, "Shipping it now"), "sent and shown: {:?}", names(&ui));
    assert_eq!(
        value(&ui, Role::MultilineTextInput, "Message").unwrap_or_default(),
        "",
        "the composer is empty again"
    );
    // An empty message is not sent.
    let before = names(&ui).len();
    click(&mut t, Role::Button, "Send");
    assert_eq!(names(&ui).len(), before);
}

#[test]
fn file_explorer_lists_a_folder_and_opens_another_from_the_tree() {
    let tmp = TempDir::new("explorer");
    std::fs::create_dir_all(tmp.0.join("docs")).must("docs");
    std::fs::write(tmp.0.join("docs").join("notes.md"), "# Notes").must("notes.md");
    std::fs::write(tmp.0.join("file2.txt"), "hello").must("file2.txt");
    std::fs::write(tmp.0.join("file10.txt"), vec![b'x'; 2048]).must("file10.txt");
    let root = tmp.0.clone();
    let mut t = start("File explorer", move |app: &App| {
        app.open(WindowSpec::new("Files").size(820.0, 520.0), move |ui, _| {
            ui.root().child(file_explorer::explorer(ui, &root));
        });
    });
    let ui = ui(&t);
    assert!(node(&ui, Role::Tree, "Folders").is_some(), "{:?}", names(&ui));
    assert!(node(&ui, Role::Table, "Contents").is_some(), "{:?}", names(&ui));
    let all = names(&ui);
    let pos = |s: &str| all.iter().position(|n| n.contains(s)).must(&format!("{s} in {all:?}"));
    assert!(
        pos("docs") < pos("file2.txt") && pos("file2.txt") < pos("file10.txt"),
        "folders first, natural order"
    );
    assert!(has(&ui, "2.0 KB"), "sizes for people: {all:?}");
    assert!(!has(&ui, "notes.md"));
    // Choosing the folder in the tree, as a screen reader would.
    click(&mut t, Role::TreeItem, "docs");
    assert!(has(&ui, "notes.md"), "the chosen folder's files: {:?}", names(&ui));
    assert!(!has(&ui, "file2.txt"));
}

#[test]
fn document_viewer_shows_pages_and_moves_between_them() {
    let mut t = start("Document viewer", document_viewer::app);
    t.frames(4);
    let ui = ui(&t);
    assert!(node(&ui, Role::Document, "Quarterly report").is_some(), "{:?}", names(&ui));
    assert!(has(&ui, "Page 1 of 12"), "{:?}", names(&ui));
    let text = value(&ui, Role::Page, "Page 1").must("the current page's text for screen readers");
    assert!(text.contains("Chapter 1 line 1"), "{text}");
    click(&mut t, Role::Button, "Next page");
    t.frames(4);
    assert!(has(&ui, "Page 2 of 12"), "{:?}", names(&ui));
    assert!(value(&ui, Role::Page, "Page 2").is_some_and(|s| s.contains("Chapter 2 line 1")));
    let zoom = |ui: &Ui| names(ui).into_iter().find(|n| n.ends_with('%')).must("the zoom");
    let before = zoom(&ui);
    click(&mut t, Role::Button, "Zoom in");
    t.frames(2);
    assert_ne!(zoom(&ui), before, "zoomed in");
}

#[test]
fn settings_page_switches_sections_and_disables_what_does_not_apply() {
    let mut t = start("Settings page", settings_page::app);
    let ui = ui(&t);
    assert!(states(&ui, Role::Tab, "Appearance").contains(&"selected"), "{:?}", names(&ui));
    assert!(has(&ui, "Theme") && has(&ui, "High contrast"), "{:?}", names(&ui));
    click(&mut t, Role::Tab, "Notifications");
    assert!(states(&ui, Role::Tab, "Notifications").contains(&"selected"));
    assert!(!has(&ui, "High contrast"), "only the chosen section shows");
    assert!(states(&ui, Role::Switch, "Show notifications").contains(&"checked"));
    assert!(!states(&ui, Role::Switch, "Play a sound").contains(&"disabled"));
    click(&mut t, Role::Switch, "Show notifications");
    assert!(states(&ui, Role::Switch, "Show notifications").contains(&"not checked"));
    assert!(states(&ui, Role::Switch, "Play a sound").contains(&"disabled"), "{:?}", ui.reader_view());
    assert!(states(&ui, Role::Slider, "Volume").contains(&"disabled"));
}

#[test]
fn settings_page_remembers_the_settings_at_the_next_start() {
    let data = TempDir::new("settings-kept");
    let mut t = start_in("Settings page", &data.0, settings_page::app);
    click(&mut t, Role::Tab, "Notifications");
    click(&mut t, Role::Switch, "Show notifications");
    assert!(states(&ui(&t), Role::Switch, "Show notifications").contains(&"not checked"));
    // Saves are written off the UI thread; wait for them as quitting does.
    t.runner().app().persist_store().must("a store in the data folder").flush().must("the settings saved");
    drop(t);

    let mut t = start_in("Settings page", &data.0, settings_page::app);
    click(&mut t, Role::Tab, "Notifications");
    let ui = ui(&t);
    assert!(states(&ui, Role::Switch, "Show notifications").contains(&"not checked"), "restored");
    assert!(states(&ui, Role::Switch, "Play a sound").contains(&"disabled"));
}

#[test]
fn tray_app_lives_in_the_tray_and_comes_back_for_breaks() {
    let mut t = start("Break reminder", tray_app::app);
    let labels = |t: &mut TestApp| -> Vec<String> {
        t.host()
            .tray()
            .must("a tray icon")
            .menu
            .iter()
            .filter_map(|i| match i {
                TrayItem::Item { label, .. } => Some(label.clone()),
                TrayItem::Separator => None,
            })
            .collect()
    };
    assert_eq!(t.host().tray().map(|i| i.tooltip.clone()).as_deref(), Some("Break reminder"));
    assert_eq!(labels(&mut t), ["Show", "Pause reminders", "Quit"]);
    let ui = ui(&t);
    assert!(has(&ui, "A reminder comes every twenty minutes"), "{:?}", names(&ui));
    // Closing hides the window in the tray; the app keeps running.
    let id = window(&mut t);
    t.host().send(HostEvent::CloseRequested(id));
    t.frames(1);
    assert!(t.host().window(id).is_some_and(|w| !w.options().visible), "hidden, not closed");
    assert!(!t.host().has_exited());
    // Twenty minutes later it comes back with a reminder.
    t.host().advance(tray_app::EVERY);
    t.frames(2);
    assert!(t.host().window(id).is_some_and(|w| w.options().visible), "shown for the break");
    assert!(has(&ui, "Time for a break"), "{:?}", names(&ui));
    let notices = t.host().notifications().to_vec();
    assert_eq!(notices.len(), 1, "the reminder also notifies");
    assert_eq!(
        notices.first().map(|n| (n.id, n.title.as_str(), n.app.as_str())),
        Some((tray_app::BREAK_NOTICE, "Time for a break", "Break reminder"))
    );
    // The notification's button takes the break without opening anything.
    t.host()
        .send(HostEvent::Notification(tray_app::BREAK_NOTICE, NotificationResponse::Action("taken".into())));
    t.frames(2);
    assert!(!has(&ui, "Time for a break"));
    // The next one is taken in the window.
    t.host().advance(tray_app::EVERY);
    t.frames(2);
    assert_eq!(t.host().notifications().len(), 2);
    click(&mut t, Role::Button, "Break taken");
    assert!(!has(&ui, "Time for a break"));
    // Pausing from the tray menu: the menu and the window both say so.
    t.host().send(HostEvent::Tray(TrayEvent::Menu(tray_app::PAUSE)));
    t.frames(2);
    assert_eq!(labels(&mut t), ["Show", "Resume reminders", "Quit"]);
    assert!(states(&ui, Role::Switch, "Pause reminders").contains(&"checked"));
    assert!(has(&ui, "Reminders are paused."));
    t.host().send(HostEvent::Tray(TrayEvent::Menu(tray_app::QUIT)));
    t.frames(1);
    assert!(t.host().has_exited(), "Quit quits");
}

#[test]
fn sticky_notes_open_as_frameless_windows_that_save_as_you_write() {
    let tmp = TempDir::new("notes");
    let dir = tmp.0.clone();
    let mut t = start("Sticky notes", move |app: &App| sticky_notes::open_notes(app, &dir));
    let id = window(&mut t);
    let opts = t.host().window(id).map(|w| w.options().clone()).must("the note's window");
    assert!(!opts.decorated && opts.always_on_top, "frameless, on top");
    assert_eq!(opts.title, "Note 1");
    let ui = ui(&t);
    let body = node(&ui, Role::MultilineTextInput, "Note").must("the note's text").id;
    assert!(ui.accessibility_set_value(body, "Buy oat milk"));
    t.frames(1);
    assert_eq!(std::fs::read_to_string(tmp.0.join("1.txt")).ok().as_deref(), Some("Buy oat milk"));
    // The header moves the window.
    let header = ui
        .reader_view()
        .into_iter()
        .find(|n| n.name.as_deref() == Some("Note 1"))
        .and_then(|n| n.bounds)
        .must("the header's title");
    let _ = t.host().take_interactions();
    press_at(&mut t, id, Point::new(header.min.x + 2.0, header.min.y + 2.0));
    assert_eq!(t.host().take_interactions(), vec![(id, None)], "a press on the header begins a move");
    // A second note gets its own window.
    click(&mut t, Role::Button, "New note");
    let titles: Vec<String> = t.runner().app().windows().iter().map(remedy_ui::Window::title).collect();
    assert_eq!(titles, ["Note 1", "Note 2"]);
    // Deleting a note removes its file and its window.
    click(&mut t, Role::Button, "Delete note");
    assert!(!tmp.0.join("1.txt").exists());
    assert_eq!(t.runner().app().windows().len(), 1);
}

#[test]
fn audio_meter_follows_the_level_and_pauses() {
    let mut t = start("Audio meter", audio_meter::app);
    t.frames(10);
    let ui = ui(&t);
    assert!(node(&ui, Role::Status, "Test signal").is_some(), "the speaking ring: {:?}", names(&ui));
    let level = value(&ui, Role::Progress, "Test signal level").must("the meter's level");
    assert!(!level.starts_with("0 percent"), "the test signal is moving: {level}");
    assert!(has(&ui, "Voice activates above 20%"));
    let threshold = node(&ui, Role::Slider, "Voice activation threshold").must("the threshold").id;
    assert!(ui.accessibility_action(threshold, Action::Increment));
    t.frames(1);
    assert!(has(&ui, "Voice activates above 21%"), "{:?}", names(&ui));
    click(&mut t, Role::Button, "Pause test signal");
    t.frames(6);
    assert!(states(&ui, Role::Button, "Pause test signal").contains(&"checked"));
    let level = value(&ui, Role::Progress, "Test signal level").must("the meter's level");
    assert!(level.starts_with("0 percent"), "paused: {level}");
}

#[test]
fn unlock_screen_keeps_the_vault_hidden_until_the_password_opens_it() {
    let check: unlock_screen::Check = Rc::new(|p: &str| p == "hunter2");
    let mut t = start("Unlock screen", move |app: &App| unlock_screen::app_with(app, check));
    let ui = ui(&t);
    assert!(node(&ui, Role::PasswordInput, "Master password").is_some(), "{:?}", names(&ui));
    assert!(!has(&ui, "3 passwords"), "the vault is out of the tree while locked");
    t.type_text("wrong");
    t.key(KeyEvent::press(Key::Enter));
    assert!(has(&ui, "That password does not open this vault."), "{:?}", names(&ui));
    assert!(
        ui.reader_view().iter().all(|n| {
            !n.name.as_deref().unwrap_or_default().contains("wrong")
                && !n.value.as_deref().unwrap_or_default().contains("wrong")
        }),
        "the password never reaches the tree"
    );
    t.type_text("hunter2");
    click(&mut t, Role::Button, "Unlock");
    assert!(has(&ui, "3 passwords"), "unlocked: {:?}", names(&ui));
    assert!(node(&ui, Role::PasswordInput, "Master password").is_none());
    // Five minutes without input locks it again.
    t.host().advance(unlock_screen::IDLE + Duration::from_secs(1));
    t.frames(2);
    assert!(node(&ui, Role::PasswordInput, "Master password").is_some(), "locked when idle");
    assert!(!has(&ui, "3 passwords"));
}
