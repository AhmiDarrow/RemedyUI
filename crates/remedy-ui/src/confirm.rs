//! A question for the person ([`App::confirm`]): a small window of the app's own, drawn in
//! its theme, that works with no other window open (a tray app's "Quit?").

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Size;
use remedy_ui_input::Key;
use remedy_ui_layout::{Align, Justify};
use remedy_ui_view::Ui;
use remedy_ui_widgets_core as w;

use crate::{App, Window, WindowSpec};

/// What [`App::confirm`] asks: a title, the text, and the buttons' words.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ConfirmSpec {
    /// The window's title and the dialog's heading ("Quit Portal?").
    pub title: String,
    /// What it says under the heading. A blank line starts a new paragraph.
    pub text: String,
    /// The button that answers yes; `None` says "OK" in the app's language.
    pub ok: Option<String>,
    /// The button that answers no; `None` says "Cancel" in the app's language.
    pub cancel: Option<String>,
    /// Shows the no button (true by default); false makes it a notice with one button,
    /// which answers yes.
    pub show_cancel: bool,
    /// The yes button does something that cannot be taken back (quitting, deleting): it is
    /// drawn as a warning, and the no button has the focus, so Enter answers no, as a
    /// warning message box's default button does.
    pub danger: bool,
    /// The window's width in logical pixels (420 by default); its height fits the text.
    pub width: f32,
    /// The tallest it grows for a long text before the text scrolls (640 by default).
    pub max_height: f32,
}

impl ConfirmSpec {
    /// Asks `title`, explained by `text`, with OK and Cancel.
    pub fn new(title: impl Into<String>, text: impl Into<String>) -> Self {
        ConfirmSpec {
            title: title.into(),
            text: text.into(),
            ok: None,
            cancel: None,
            show_cancel: true,
            danger: false,
            width: 420.0,
            max_height: 640.0,
        }
    }

    /// The yes button's words.
    pub fn ok(mut self, label: impl Into<String>) -> Self {
        self.ok = Some(label.into());
        self
    }

    /// The no button's words.
    pub fn cancel(mut self, label: impl Into<String>) -> Self {
        self.cancel = Some(label.into());
        self
    }

    /// A notice: one button, which answers yes.
    pub fn notice(mut self) -> Self {
        self.show_cancel = false;
        self
    }

    /// Marks the yes button as one that cannot be taken back (see [`ConfirmSpec::danger`]).
    pub fn danger(mut self, on: bool) -> Self {
        self.danger = on;
        self
    }

    /// Sets the window's width in logical pixels.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets how tall the window may grow before the text scrolls.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = height;
        self
    }
}

/// Calls the answer once, whichever way the dialog ends.
type Answer = Rc<RefCell<Option<Box<dyn FnOnce(bool)>>>>;

fn answer(slot: &Answer, yes: bool) {
    let f = slot.borrow_mut().take();
    if let Some(f) = f {
        f(yes);
    }
}

pub(crate) fn open(app: &App, spec: ConfirmSpec, done: Box<dyn FnOnce(bool)>) -> Window {
    let slot: Answer = Rc::new(RefCell::new(Some(done)));
    let mut ws = WindowSpec::new(spec.title.clone()).size(spec.width, 200.0).always_on_top(true).hidden();
    ws.options.resizable = false;
    ws.hidden_until_drawn = true;
    let s2 = slot.clone();
    let win = app.open(ws, move |ui, win| build(ui, win, &spec, &s2));
    // Closed from its title bar (or by the system): no.
    win.on_closed(move || answer(&slot, false));
    win
}

fn build(ui: &Ui, win: &Window, spec: &ConfirmSpec, slot: &Answer) {
    let unit = ui.theme().spacing.unit;
    let pad = unit * 5.0;
    let press = |yes: bool| {
        let (slot, win) = (slot.clone(), win.clone());
        move || {
            answer(&slot, yes);
            win.close();
        }
    };
    let ok_label = spec.ok.clone().unwrap_or_else(|| crate::STRINGS.get(ui, "confirm-ok"));
    let ok =
        w::button(ui, ok_label, press(true)).variant(if spec.danger { "danger_solid" } else { "primary" });
    let cancel = spec.show_cancel.then(|| {
        let label = spec.cancel.clone().unwrap_or_else(|| crate::STRINGS.get(ui, "confirm-cancel"));
        w::button(ui, label, press(false))
    });
    // Each platform's order: the answer that goes ahead last on Windows reads left to right
    // as "OK Cancel"; macOS and the Linux desktops put it on the right.
    let mut buttons = vec![ok.clone()];
    if let Some(c) = &cancel {
        if cfg!(windows) {
            buttons.push(c.clone());
        } else {
            buttons.insert(0, c.clone());
        }
    }
    let actions = buttons
        .into_iter()
        .fold(ui.el("dialog_actions").row().gap(unit * 2.0).justify(Justify::End), |r, b| r.child(b));
    let paragraphs = spec
        .text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .fold(ui.el("dialog_content").column().gap(unit * 2.0), |c, p| c.child(w::text(ui, p.to_owned())));
    let body = ui
        .el("dialog_content")
        .column()
        .gap(unit * 3.0)
        .semantics(Semantics::new(Role::Dialog).label(spec.title.clone()).description(spec.text.clone()))
        .child(w::heading(ui, 3, spec.title.clone()))
        .child(paragraphs)
        .child(actions);
    let (s2, w2) = (slot.clone(), win.clone());
    let root = ui
        .root()
        .column()
        .align(Align::Stretch)
        .padding(pad)
        .scroll()
        // Tab and Shift+Tab stay among the buttons; Escape answers no, as Cancel does.
        .focus_scope()
        .on_key(move |e| {
            if e.pressed && e.key == Key::Escape {
                answer(&s2, false);
                w2.close();
                return true;
            }
            false
        })
        .child(body.clone());
    let _ = root;
    // The text sets the height: lay it out once at the window's width before it is shown.
    let _ = ui.frame(Size::new(spec.width, spec.max_height.max(1.0)), 1.0);
    let height = body.bounds().map_or(200.0, |b| b.height() + pad * 2.0);
    win.set_size(spec.width, height.clamp(1.0, spec.max_height.max(1.0)).ceil());
    win.center();
    // Brought to the front and focused even from a tray app with no window, as a message box
    // that sets itself in front is.
    win.focus();
    let first = if spec.danger { cancel.unwrap_or(ok) } else { ok };
    first.focus_visibly();
}

#[cfg(all(test, feature = "headless"))]
mod tests {
    use std::cell::Cell;

    use remedy_ui_a11y::Role;
    use remedy_ui_input::{Key, KeyEvent, Modifiers};

    use super::*;
    use crate::AppConfig;
    use crate::testing::TestApp;

    fn asking(spec: ConfirmSpec) -> (TestApp, Rc<Cell<Option<bool>>>) {
        let got: Rc<Cell<Option<bool>>> = Rc::default();
        let g2 = got.clone();
        let app = TestApp::new(AppConfig::new("Tray"), move |app: &App| {
            // A tray app: no window of its own.
            app.keep_running(true);
            app.confirm(spec, move |yes| g2.set(Some(yes)));
        });
        (app, got)
    }

    fn quit() -> ConfirmSpec {
        ConfirmSpec::new(
            "Quit Portal?",
            "This stops the Portal server.\n\nOpen Portal again from the Start menu.",
        )
        .ok("Yes")
        .cancel("No")
        .danger(true)
    }

    #[test]
    fn it_opens_in_front_sized_to_its_text_and_read_as_a_dialog() {
        let (mut app, got) = asking(quit());
        app.frames(2);
        let ids = app.host().window_ids();
        assert_eq!(ids.len(), 1, "the dialog is the only window");
        let Some(id) = ids.first().copied() else { unreachable!("one window") };
        let host = app.host();
        let Some(win) = host.window(id) else { unreachable!("open") };
        assert!(win.options().visible, "shown");
        assert!(win.options().always_on_top, "above other windows");
        assert!(!win.options().resizable);
        assert!(win.is_centered(), "in the middle of the display");
        assert_eq!(win.options().title, "Quit Portal?");
        let (w, h, scale) = win.size();
        assert_eq!(w as f32 / scale, 420.0);
        assert!((80.0..400.0).contains(&(h as f32 / scale)), "fits its text: {h}");
        assert_eq!(host.focused_window(), Some(id), "focused");
        let Some(dialog) = app.node(Role::Dialog, "Quit Portal?") else { unreachable!("a dialog") };
        assert!(dialog.description.as_deref().is_some_and(|d| d.contains("stops the Portal server")));
        assert!(app.node(Role::Heading(3), "Quit Portal?").is_some());
        assert!(app.node(Role::Button, "No").is_some_and(|n| n.focused), "danger: No has the focus");
        assert_eq!(got.get(), None, "nothing answered yet");
    }

    #[test]
    fn enter_answers_the_focused_button_and_closes() {
        let (mut app, got) = asking(quit());
        app.frames(1);
        app.key(KeyEvent::press(Key::Enter));
        app.frames(2);
        assert_eq!(got.get(), Some(false), "Enter on No");
        assert!(app.host().window_ids().is_empty(), "closed");

        let (mut app, got) = asking(ConfirmSpec::new("Restart the server?", "Anything playing will stop."));
        app.frames(1);
        assert!(app.node(Role::Button, "OK").is_some_and(|n| n.focused), "not danger: OK has the focus");
        app.key(KeyEvent::press(Key::Enter));
        app.frames(1);
        assert_eq!(got.get(), Some(true));
    }

    #[test]
    fn escape_and_closing_answer_no_and_tab_stays_inside() {
        let (mut app, got) = asking(quit());
        app.frames(1);
        app.key(KeyEvent::press(Key::Tab));
        assert!(app.node(Role::Button, "Yes").is_some_and(|n| n.focused), "Tab moves to Yes");
        app.key(KeyEvent::press(Key::Tab));
        assert!(app.node(Role::Button, "No").is_some_and(|n| n.focused), "and wraps back");
        app.key(KeyEvent::press(Key::Tab).with_modifiers(Modifiers::SHIFT));
        assert!(app.node(Role::Button, "Yes").is_some_and(|n| n.focused));
        app.key(KeyEvent::press(Key::Escape));
        app.frames(1);
        assert_eq!(got.get(), Some(false));

        let (mut app, got) = asking(quit());
        app.frames(1);
        let Some(id) = app.host().window_ids().first().copied() else { unreachable!("open") };
        app.host().send(remedy_ui_host::HostEvent::CloseRequested(id));
        app.frames(2);
        assert_eq!(got.get(), Some(false), "closed from the title bar");
    }

    #[test]
    fn a_press_answers_once_and_a_notice_has_one_button() {
        let (mut app, got) = asking(quit());
        assert!(app.press(Role::Button, "Yes"));
        app.frames(2);
        assert_eq!(got.get(), Some(true));
        assert!(app.host().window_ids().is_empty(), "closed, and the close did not answer again");

        let (mut app, got) = asking(ConfirmSpec::new("Portal", "Portal is not running.").notice());
        app.frames(1);
        assert_eq!(app.find_all(Role::Button).len(), 1);
        assert!(app.press(Role::Button, "OK"));
        assert_eq!(got.get(), Some(true));
    }
}
