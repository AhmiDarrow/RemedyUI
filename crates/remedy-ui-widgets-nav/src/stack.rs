//! A navigation stack: pages pushed on top of each other with a back button, keeping every
//! page below alive (its scroll position and fields) until it is popped.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_a11y::{Live, Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_reactive::Signal;
use remedy_ui_style::StylePatch;
use remedy_ui_view::{Align, El, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

struct Page {
    title: String,
    el: El,
}

struct Inner {
    ui: Ui,
    pages: RefCell<Vec<Page>>,
    depth: Signal<usize>,
    title: Signal<String>,
    body: El,
    root: El,
}

/// A stack of pages. Alt+Left, the mouse's back button and the back arrow pop the top page.
#[derive(Clone)]
pub struct NavStack(Rc<Inner>);

impl std::fmt::Debug for NavStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NavStack(depth {})", self.0.pages.borrow().len())
    }
}

impl NavStack {
    /// A stack starting with `first` titled `title`.
    #[track_caller]
    pub fn new(ui: &Ui, title: impl Into<String>, first: El) -> NavStack {
        let unit = ui.theme().spacing.unit;
        let depth = Signal::new(1usize);
        let title_s = Signal::new(title.into());
        let body = w::column(ui).grow(1.0).fill_width();
        let heading = w::text(ui, "")
            .variant("strong")
            .style(StylePatch::default().text_size(ui.theme().type_scale.title));
        let h2 = heading.clone();
        let heading = heading.bind(move || {
            let t = title_s.get();
            let _ = h2.clone().text(remedy_ui_view::RichText::plain(t.clone()));
            let _ = h2.clone().semantics(Semantics::new(Role::Heading(2)).label(t).live(Live::Polite));
        });
        let header = w::row(ui)
            .align(Align::Center)
            .gap(unit * 2.0)
            .padding(Insets::symmetric(unit * 2.0, unit * 3.0))
            .fill_width();
        let root = w::column(ui).grow(1.0).fill_width().child(header.clone()).child(body.clone());
        let me = NavStack(Rc::new(Inner {
            ui: ui.clone(),
            pages: RefCell::new(Vec::new()),
            depth,
            title: title_s,
            body,
            root: root.clone(),
        }));
        let m1 = me.clone();
        let back = w::icon_button(ui, Icon::ChevronLeft, crate::tr_fn(ui, "stack-back"), move || {
            m1.pop();
        });
        let b2 = back.clone();
        let back = back.bind(move || {
            let _ = b2.clone().hidden(depth.get() < 2);
        });
        let _ = header.child(back).child(heading);
        me.0.pages
            .borrow_mut()
            .push(Page { title: title_s.get_untracked().unwrap_or_default(), el: first.clone() });
        let _ = me.0.body.clone().child(first.grow(1.0));
        let (m2, m3) = (me.clone(), me.clone());
        let _ = root
            .on_key(move |e| {
                if e.pressed && e.key == Key::ArrowLeft && e.modifiers.alt {
                    return m2.pop();
                }
                false
            })
            .on_pointer(move |e| {
                // The mouse's back button (X1).
                if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::BACK) {
                    return m3.pop();
                }
                false
            })
            .keyboard_alternative(crate::tr_fn(ui, "stack-back-keys"));
        me
    }

    /// The stack's element, to place in a window.
    pub fn el(&self) -> El {
        self.0.root.clone()
    }

    /// Shows `page` on top, titled `title`; the page below stays as it was.
    pub fn push(&self, title: impl Into<String>, page: El) {
        let title = title.into();
        if let Some(top) = self.0.pages.borrow().last() {
            let _ = top.el.clone().hidden(true);
        }
        let _ = self.0.body.clone().child(page.clone().grow(1.0));
        self.0.pages.borrow_mut().push(Page { title: title.clone(), el: page.clone() });
        self.0.title.set(title);
        self.0.depth.set(self.0.pages.borrow().len());
        page.focus_first();
    }

    /// Removes the top page and shows the one below. False on the first page.
    pub fn pop(&self) -> bool {
        let top = {
            let mut pages = self.0.pages.borrow_mut();
            if pages.len() < 2 {
                return false;
            }
            pages.pop()
        };
        if let Some(p) = top {
            p.el.remove();
        }
        let below = self.0.pages.borrow().last().map(|p| (p.title.clone(), p.el.clone()));
        if let Some((t, el)) = below {
            let _ = el.clone().hidden(false);
            self.0.title.set(t);
            el.focus_first();
        }
        self.0.depth.set(self.0.pages.borrow().len());
        true
    }

    /// How many pages are stacked (tracked).
    pub fn depth(&self) -> usize {
        self.0.depth.get()
    }

    /// The top page's title (tracked).
    pub fn title(&self) -> String {
        self.0.title.get()
    }

    /// The window this stack lives in.
    pub fn ui(&self) -> &Ui {
        &self.0.ui
    }
}
