//! Content that moves between a place in a window and a window of its own (a shared screen
//! popped out, a chat torn off). The content is rebuilt where it goes; its state lives in
//! signals the app keeps outside the elements, so it is the same content in either place.
//! Closing its own window brings it back.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

use crate::{App, Window, WindowSpec};

type Build = Rc<dyn Fn(&Ui) -> El>;

/// Content that can move to a window of its own and back. Cheap to clone.
#[derive(Clone)]
pub struct Movable {
    app: App,
    title: String,
    holder: El,
    build: Build,
    window: Rc<RefCell<Option<Window>>>,
    /// True while the content is in its own window.
    pub out: Signal<bool>,
}

impl std::fmt::Debug for Movable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Movable({})", self.title)
    }
}

/// Content built by `build`, shown in `ui`'s window until popped out to a window titled
/// `title`. Put the returned element where the content belongs.
pub fn movable(
    ui: &Ui,
    app: &App,
    title: impl Into<String>,
    build: impl Fn(&Ui) -> El + 'static,
) -> (El, Movable) {
    let holder = ui.el("movable").column().grow(1.0).fill_width();
    let m = Movable {
        app: app.clone(),
        title: title.into(),
        holder: holder.clone(),
        build: Rc::new(build),
        window: Rc::default(),
        out: Signal::new(false),
    };
    m.show_here();
    (holder, m)
}

impl Movable {
    fn show_here(&self) {
        self.holder.clear();
        let ui = self.holder.ui().clone();
        let content = (self.build)(&ui);
        let _ = self.holder.clone().child(content);
    }

    fn show_placeholder(&self) {
        self.holder.clear();
        let ui = self.holder.ui().clone();
        let unit = ui.theme().spacing.unit;
        let me = self.clone();
        let said =
            remedy_ui_words::fill(&ui.word("{title} is in its own window."), &[("title", &self.title)]);
        let note = remedy_ui_widgets_core::text(&ui, said).variant("muted");
        let back = remedy_ui_widgets_core::button(&ui, ui.word("Bring it back"), move || me.bring_back());
        let _ = self.holder.clone().child(
            remedy_ui_widgets_core::column(&ui)
                .gap(unit * 2.0)
                .align(remedy_ui_view::Align::Center)
                .padding(remedy_ui_core::Insets::all(unit * 4.0))
                .child(note)
                .child(back),
        );
    }

    /// Moves the content to a window of its own, sized like the place it leaves.
    pub fn pop_out(&self) {
        if self.out.get_untracked().unwrap_or(false) {
            if let Some(w) = self.window.borrow().as_ref() {
                w.focus();
            }
            return;
        }
        let size = self.holder.bounds().map(|b| b.size()).filter(|s| s.width > 1.0 && s.height > 1.0);
        let (w, h) = size.map_or((800.0, 600.0), |s| (s.width.max(320.0), s.height.max(240.0)));
        let build = self.build.clone();
        let win = self.app.open(WindowSpec::new(self.title.clone()).size(w, h), move |ui, _| {
            let content = build(ui);
            ui.root().child(content.grow(1.0));
        });
        // Closing that window brings the content back.
        let me = self.clone();
        win.on_closed(move || {
            me.window.borrow_mut().take();
            if me.out.get_untracked().unwrap_or(false) {
                me.out.set(false);
                me.show_here();
            }
        });
        *self.window.borrow_mut() = Some(win);
        self.out.set(true);
        self.show_placeholder();
    }

    /// Brings the content back to its place and closes its window.
    pub fn bring_back(&self) {
        if !self.out.get_untracked().unwrap_or(false) {
            return;
        }
        self.out.set(false);
        self.show_here();
        let w = self.window.borrow_mut().take();
        if let Some(w) = w {
            w.close();
        }
    }

    /// Its own window, while it has one.
    pub fn window(&self) -> Option<Window> {
        self.window.borrow().clone()
    }
}
