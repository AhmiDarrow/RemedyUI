//! Driving an app in tests: run it on the in-memory host, find elements the way a screen
//! reader does (by role and accessible name), use them as a person would, and read back
//! what a screen reader would hear (value and states).
//!
//! A test app keeps its data (persisted signals, window geometry) in a private temporary
//! folder that is removed when the [`TestApp`](crate::testing::TestApp) is dropped, never
//! in the person's real data folder, unless the test names one with
//! [`AppConfig::data_dir`](crate::AppConfig::data_dir).
//!
//! ```
//! use remedy_ui::a11y::Role;
//! use remedy_ui::reactive::Signal;
//! use remedy_ui::testing::TestApp;
//! use remedy_ui::widgets as w;
//!
//! let clicks = Signal::new(0u32);
//! let mut app = TestApp::window(move |ui, _| {
//!     ui.root().child(w::button(ui, "Add", move || clicks.update(|c| *c += 1)));
//! });
//! assert!(app.press(Role::Button, "Add"));
//! assert_eq!(clicks.get_untracked().ok(), Some(1));
//! let add = app.node(Role::Button, "Add");
//! assert!(add.is_some_and(|n| !n.disabled));
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use remedy_ui_a11y::{Action, ReaderNode, Role};
use remedy_ui_bridge_headless::HeadlessHost;
use remedy_ui_host::{HostEvent, WindowId};
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_view::{El, Ui};

use crate::{App, AppConfig, GeometryStore, Runner, Window, WindowSpec};

/// An app running on the in-memory host ([`HeadlessHost`]), on the CPU renderer, with window
/// geometry kept in memory and its data in a private temporary folder (unless the test set
/// [`AppConfig::data_dir`]).
pub struct TestApp {
    runner: Runner,
    host: HeadlessHost,
    // Dropped after the runner, so nothing writes into the folder after it is removed.
    _data: Option<TempData>,
}

impl std::fmt::Debug for TestApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TestApp({} windows)", self.host.window_ids().len())
    }
}

/// A folder of the test's own under the system's temporary folder, removed on drop.
struct TempData(PathBuf);

impl TempData {
    /// A new, unique folder. It is made now when it can be; where it cannot, the app's
    /// data stores fail to open there and keep their data in memory instead, and nothing
    /// reaches the person's real data folder either way.
    fn new() -> TempData {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!("remedy-ui-test-{}-{n}-{nanos:09}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        TempData(dir)
    }
}

impl Drop for TempData {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl TestApp {
    /// Runs the app `setup` builds under `config` (made CPU-only), through its first frame.
    /// Without [`AppConfig::data_dir`], the app's data goes to a private temporary folder
    /// removed when the test app is dropped ([`TestApp::data_dir`]).
    pub fn new(config: AppConfig, setup: impl FnOnce(&App) + 'static) -> TestApp {
        let mut config = config.cpu_only();
        let data = config.data_dir.is_none().then(TempData::new);
        if let Some(t) = &data {
            config.data_dir = Some(t.0.clone());
        }
        let runner = Runner::with_geometry(config, GeometryStore::in_memory(), setup);
        let mut app = TestApp { runner, host: HeadlessHost::new(), _data: data };
        app.frames(1);
        app
    }

    /// An app with one 480 × 360 window that `build` fills, through its first frame.
    pub fn window(build: impl FnOnce(&Ui, &Window) + 'static) -> TestApp {
        TestApp::new(AppConfig::new("Test"), move |app: &App| {
            app.open(WindowSpec::new("Test").size(480.0, 360.0), build);
        })
    }

    /// Where the app keeps its data: the test's own temporary folder, or the one the test
    /// set with [`AppConfig::data_dir`].
    pub fn data_dir(&self) -> Option<PathBuf> {
        self.runner.app().data_dir()
    }

    /// Runs `n` frames: events sent so far are delivered, timers due are run, and every window
    /// draws what changed.
    pub fn frames(&mut self, n: usize) {
        self.host.run_frames(&mut self.runner, n);
    }

    /// The in-memory host: its windows, clipboard, tray and dialogs, and its clock.
    pub fn host(&mut self) -> &mut HeadlessHost {
        &mut self.host
    }

    /// The runner, and through it the app ([`Runner::app`]).
    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    /// The first open window's view.
    pub fn ui(&self) -> Option<Ui> {
        self.runner.app().windows().first().and_then(Window::ui)
    }

    fn first_window(&self) -> Option<WindowId> {
        self.host.window_ids().first().copied()
    }

    /// The first window as a screen reader meets it after the last frame, in reading order:
    /// hidden content left out, what a merging element holds (a list row: avatar, name,
    /// time, text) folded into its name, overlays and the trusted layer included.
    pub fn reader_view(&self) -> Vec<ReaderNode> {
        self.ui().map(|ui| ui.reader_view()).unwrap_or_default()
    }

    /// Every element in the first window with `role`, in reading order, as a screen reader
    /// reaches them (see [`TestApp::reader_view`]).
    pub fn find_all(&self, role: Role) -> Vec<El> {
        let Some(ui) = self.ui() else { return Vec::new() };
        ui.reader_view()
            .into_iter()
            .filter(|n| n.role == role)
            .filter_map(|n| ui.element_by_node_id(n.id))
            .collect()
    }

    /// The first element in the first window with `role` whose accessible name, as screen
    /// readers get it, is `name`. For an element that merges its descendants (a list row),
    /// that is its own label and its descendants' labels and values joined with ", ".
    pub fn find(&self, role: Role, name: &str) -> Option<El> {
        let ui = self.ui()?;
        let node = self.node(role, name)?;
        ui.element_by_node_id(node.id)
    }

    /// What a screen reader hears of the element with `role` and `name`: its value, and its
    /// states (`checked`, `selected`, `expanded`, `disabled`, `focused`). `focused` is the
    /// keyboard focus now; the rest are as of the last frame.
    pub fn node(&self, role: Role, name: &str) -> Option<ReaderNode> {
        let ui = self.ui()?;
        let mut node =
            ui.reader_view().into_iter().find(|n| n.role == role && n.name.as_deref() == Some(name))?;
        node.focused = ui.element_by_node_id(node.id).is_some_and(|el| el.is_focused());
        Some(node)
    }

    /// Clicks the element with `role` and `name` the way assistive technology does, then runs
    /// a frame. False when there is no such element or it cannot be clicked (a disabled
    /// control refuses, as it refuses the pointer).
    pub fn press(&mut self, role: Role, name: &str) -> bool {
        let (Some(el), Some(ui)) = (self.find(role, name), self.ui()) else { return false };
        let done = ui.accessibility_action(el.node_id(), Action::Click);
        self.frames(1);
        done
    }

    /// Presses and releases a key as a keyboard does: `event` (a press, as
    /// [`KeyEvent::press`] makes) is sent to the first window, then its release, then a frame
    /// runs.
    pub fn key(&mut self, event: KeyEvent) {
        let release = released(&event);
        self.send_key(event);
        self.send_key(release);
        self.frames(1);
    }

    /// Sends only the press of `event` (held keys, auto-repeat with `repeat` set), then runs a
    /// frame. [`TestApp::key_release`] lets it go.
    pub fn key_press(&mut self, event: KeyEvent) {
        let mut event = event;
        event.pressed = true;
        self.send_key(event);
        self.frames(1);
    }

    /// Sends only the release of `event`'s key, then runs a frame.
    pub fn key_release(&mut self, event: KeyEvent) {
        self.send_key(released(&event));
        self.frames(1);
    }

    fn send_key(&mut self, event: KeyEvent) {
        if let Some(id) = self.first_window() {
            self.host.send(HostEvent::Key(id, event));
        }
    }

    /// Types `text` into whatever has keyboard focus in the first window, one key press and
    /// release per character, as a keyboard would.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = if c == ' ' { Key::Space } else { Key::Character(c.to_lowercase().collect()) };
            self.key(KeyEvent::press(key).with_text(Some(c.to_string())));
        }
    }
}

/// The release of `event`'s key, with the same modifiers and position (a release types
/// nothing).
fn released(event: &KeyEvent) -> KeyEvent {
    KeyEvent::release(event.key.clone()).with_modifiers(event.modifiers).with_physical(event.physical.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_a11y::Semantics;
    use remedy_ui_reactive::Signal;
    use remedy_ui_widgets_core as w;

    #[test]
    fn find_type_and_press_like_a_person() {
        let saved = Signal::new(String::new());
        let mut app = TestApp::window(move |ui, _| {
            let field = ui.text_field("text_input", remedy_ui_view::FieldKind::Single).label("Name");
            let f2 = field.clone();
            let save = w::button(ui, "Save", move || saved.set(f2.value()));
            ui.root().child(field.el()).child(save);
            field.el().focus();
        });
        assert!(app.find(Role::Button, "Save").is_some());
        assert!(app.find(Role::Button, "Nothing like it").is_none());
        app.type_text("Ada L");
        assert!(app.press(Role::Button, "Save"));
        assert_eq!(saved.get_untracked().ok().as_deref(), Some("Ada L"));
        assert!(!app.press(Role::Button, "Missing"));
        let name = app.node(Role::TextInput, "Name");
        assert_eq!(name.as_ref().and_then(|n| n.value.as_deref()), Some("Ada L"));
        assert!(name.is_some_and(|n| n.focused), "the field kept focus");
    }

    #[test]
    fn rows_are_found_by_the_name_screen_readers_get() {
        let opened = Signal::new(0u32);
        let mut app = TestApp::window(move |ui, _| {
            let row = ui
                .el("row")
                .row()
                .semantics(Semantics::new(Role::ListItem).merge_descendants().action(Action::Click))
                .on_click(move || opened.update(|n| *n += 1))
                .child(w::text(ui, "Ada"))
                .child(w::text(ui, "Hello there"));
            ui.root().child(ui.el("list").semantics(Semantics::new(Role::List).label("Chats")).child(row));
        });
        assert!(app.find(Role::Label, "Ada").is_none(), "merged into the row, as readers hear it");
        assert!(app.find(Role::ListItem, "Ada, Hello there").is_some());
        assert!(app.press(Role::ListItem, "Ada, Hello there"));
        assert_eq!(opened.get_untracked().ok(), Some(1));
    }

    #[test]
    fn keys_press_and_release_and_states_read_back() {
        let keys: Signal<Vec<(String, bool)>> = Signal::new(Vec::new());
        let on = Signal::new(false);
        let mut app = TestApp::window(move |ui, _| {
            let watch = ui
                .el("box")
                .focusable(true)
                .on_key(move |e| {
                    keys.update(|k| k.push((format!("{:?}", e.key), e.pressed)));
                    false
                })
                .child(w::check(ui, "Remember me", on));
            ui.root().child(watch.clone()).child(w::button(ui, "Later", || {}).disabled(true));
            watch.focus();
        });
        app.key(KeyEvent::press(Key::Enter));
        let seen = keys.get_untracked().unwrap_or_default();
        assert_eq!(seen.iter().map(|(_, p)| *p).collect::<Vec<_>>(), vec![true, false], "{seen:?}");
        app.key_press(KeyEvent::press(Key::Escape));
        app.key_release(KeyEvent::press(Key::Escape));
        assert_eq!(keys.get_untracked().map(|k| k.len()).ok(), Some(4));

        assert_eq!(app.node(Role::Check, "Remember me").and_then(|n| n.checked), Some(false));
        assert!(app.press(Role::Check, "Remember me"));
        assert_eq!(app.node(Role::Check, "Remember me").and_then(|n| n.checked), Some(true));
        assert!(app.node(Role::Button, "Later").is_some_and(|n| n.disabled));
        assert!(!app.press(Role::Button, "Later"), "a disabled control refuses");
    }

    #[test]
    fn a_prompt_opened_outside_an_event_waits_its_full_delay_once_shown() {
        let mut app = TestApp::window(|ui, _| {
            ui.root().child(w::text(ui, "Below"));
        });
        let Some(ui) = app.ui() else { unreachable!("the window is open") };
        // Built by app code outside any event (a reply arriving, a background job ending).
        let _prompt = w::trusted_prompt(&ui, "Allow?", "An assistant asks.", |_| {});
        // Time passes before the next frame shows it: none of it counts.
        app.host().advance(std::time::Duration::from_secs(2));
        app.frames(1);
        let allow_disabled = |app: &TestApp| app.node(Role::Button, "Allow").map(|n| n.disabled);
        assert_eq!(allow_disabled(&app), Some(true), "shown, and not yet pressable");
        app.host().advance(std::time::Duration::from_millis(500));
        app.frames(1);
        assert_eq!(allow_disabled(&app), Some(true), "still within the delay counted from showing");
        app.host().advance(std::time::Duration::from_millis(300));
        app.frames(2);
        assert_eq!(allow_disabled(&app), Some(false), "pressable after the whole delay in view");
        assert!(app.node(Role::Button, "Deny").is_some_and(|n| n.focused), "focus starts on Deny");
    }

    #[test]
    fn test_apps_keep_their_data_in_a_private_folder() {
        let app = TestApp::window(|ui, _| {
            let count = Signal::new(0u32).persist("count");
            count.set(3);
            ui.root().child(w::text(ui, "x"));
        });
        let Some(dir) = app.data_dir() else { unreachable!("a test app has a data folder") };
        assert!(dir.starts_with(std::env::temp_dir()), "{}", dir.display());
        assert!(app.runner().app().persist_store().is_some());
        assert!(dir.join("state").is_dir(), "persisted signals are saved there");
        drop(app);
        assert!(!dir.exists(), "and it is removed with the test app");

        let own = std::env::temp_dir().join(format!("remedy-ui-own-{}", std::process::id()));
        let app = TestApp::new(AppConfig::new("Own").data_dir(&own), |_| {});
        assert_eq!(app.data_dir(), Some(own.clone()), "a folder the test names is used");
        drop(app);
        let _ = std::fs::remove_dir_all(&own);
    }
}
