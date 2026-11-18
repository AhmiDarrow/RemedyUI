//! Driving an app in tests: run it on the in-memory host, find elements the way a screen
//! reader does (by role and name), and use them as a person would.
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
//! ```

use remedy_ui_a11y::{Action, Role};
use remedy_ui_bridge_headless::HeadlessHost;
use remedy_ui_host::{HostEvent, WindowId};
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_view::{El, Ui};

use crate::{App, AppConfig, GeometryStore, Runner, Window, WindowSpec};

/// An app running on the in-memory host ([`HeadlessHost`]), on the CPU renderer, with window
/// geometry kept in memory.
pub struct TestApp {
    runner: Runner,
    host: HeadlessHost,
}

impl std::fmt::Debug for TestApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TestApp({} windows)", self.host.window_ids().len())
    }
}

impl TestApp {
    /// Runs the app `setup` builds under `config` (made CPU-only), through its first frame.
    pub fn new(config: AppConfig, setup: impl FnOnce(&App) + 'static) -> TestApp {
        let runner = Runner::with_geometry(config.cpu_only(), GeometryStore::in_memory(), setup);
        let mut app = TestApp { runner, host: HeadlessHost::new() };
        app.frames(1);
        app
    }

    /// An app with one 480 × 360 window that `build` fills, through its first frame.
    pub fn window(build: impl FnOnce(&Ui, &Window) + 'static) -> TestApp {
        TestApp::new(AppConfig::new("Test"), move |app: &App| {
            app.open(WindowSpec::new("Test").size(480.0, 360.0), build);
        })
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

    /// Every element in the first window with `role`, in tree order (overlays and the trusted
    /// layer included; hidden ones left out, as a screen reader would).
    pub fn find_all(&self, role: Role) -> Vec<El> {
        let Some(ui) = self.ui() else { return Vec::new() };
        let mut out = Vec::new();
        let mut stack: Vec<El> = ui.layers().into_iter().rev().collect();
        while let Some(el) = stack.pop() {
            let info = el.info();
            let hidden = info.as_ref().and_then(|i| i.semantics.as_ref()).is_some_and(|s| s.is_hidden());
            if hidden {
                continue;
            }
            if info.as_ref().and_then(|i| i.semantics.as_ref()).is_some_and(|s| s.role() == role) {
                out.push(el.clone());
            }
            stack.extend(el.child_elements().into_iter().rev());
        }
        out
    }

    /// The first element in the first window with `role` whose accessible name is `name`.
    pub fn find(&self, role: Role, name: &str) -> Option<El> {
        self.find_all(role)
            .into_iter()
            .find(|el| el.info().and_then(|i| i.semantics).is_some_and(|s| s.name() == Some(name)))
    }

    /// Clicks the element with `role` and `name` the way assistive technology does, then runs
    /// a frame. False when there is no such element or it cannot be clicked.
    pub fn press(&mut self, role: Role, name: &str) -> bool {
        let (Some(el), Some(ui)) = (self.find(role, name), self.ui()) else { return false };
        let done = ui.accessibility_action(el.node_id(), Action::Click);
        self.frames(1);
        done
    }

    /// Sends `event` to the first window, then runs a frame.
    pub fn key(&mut self, event: KeyEvent) {
        if let Some(id) = self.first_window() {
            self.host.send(HostEvent::Key(id, event));
        }
        self.frames(1);
    }

    /// Types `text` into whatever has keyboard focus in the first window, one key per
    /// character, as a keyboard would.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = if c == ' ' { Key::Space } else { Key::Character(c.to_lowercase().collect()) };
            self.key(KeyEvent::press(key).with_text(Some(c.to_string())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_reactive::Signal;

    #[test]
    fn find_type_and_press_like_a_person() {
        let saved = Signal::new(String::new());
        let mut app = TestApp::window(move |ui, _| {
            let field = ui.text_field("text_input", remedy_ui_view::FieldKind::Single).label("Name");
            let f2 = field.clone();
            let save = remedy_ui_widgets_core::button(ui, "Save", move || saved.set(f2.value()));
            ui.root().child(field.el()).child(save);
            field.el().focus();
        });
        assert!(app.find(Role::Button, "Save").is_some());
        assert!(app.find(Role::Button, "Nothing like it").is_none());
        app.type_text("Ada L");
        assert!(app.press(Role::Button, "Save"));
        assert_eq!(saved.get_untracked().ok().as_deref(), Some("Ada L"));
        assert!(!app.press(Role::Button, "Missing"));
    }
}
