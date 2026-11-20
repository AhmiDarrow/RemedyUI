//! Finding text on screen: a bar with a field, "3 of 12", previous, next and close. It
//! searches every text inside a scope element (a document, a chat, a page), marks every match
//! in the theme's mark color, selects the current one and scrolls it into view. Matching
//! ignores case in every script. Enter goes to the next match, Shift+Enter to the previous
//! one, F3 and Shift+F3 do the same anywhere in the bar, and Escape closes it.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_a11y::{Live, Role, Semantics};
use remedy_ui_input::Key;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

/// A match: the text it is in and its bytes there.
type Hit = (El, Range<usize>);

/// A search over the text inside one element. Cheap to clone.
#[derive(Clone)]
pub struct FindState {
    scope: El,
    /// What is searched for.
    pub query: Signal<String>,
    /// How many matches there are.
    pub count: Signal<usize>,
    /// Which match is current, counting from 0 (meaningful while `count` is above 0).
    pub current: Signal<usize>,
    hits: Rc<RefCell<Vec<Hit>>>,
    marked: Rc<RefCell<Vec<El>>>,
}

impl std::fmt::Debug for FindState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FindState({:?}, {} matches)",
            self.query.get_untracked().unwrap_or_default(),
            self.hits.borrow().len()
        )
    }
}

/// Where `needle` occurs in `hay`, ignoring case, as byte ranges of `hay` that do not overlap.
pub(crate) fn matches(hay: &str, needle: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // Each folded character with the bytes of the original character it came from.
    let folded: Vec<(char, usize, usize)> = hay
        .char_indices()
        .flat_map(|(i, c)| {
            let end = i + c.len_utf8();
            c.to_lowercase().map(move |l| (l, i, end))
        })
        .collect();
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(window) = folded.get(k..k + needle.len()) {
        let (Some(first), Some(last)) = (window.first(), window.last()) else { break };
        if window.iter().zip(&needle).all(|((c, ..), n)| c == n) {
            let (start, end) = (first.1, last.2);
            // Folding can turn one character into several; a match covers whole characters
            // of the original and never overlaps the one before.
            if out.last().is_none_or(|r: &Range<usize>| r.end <= start) {
                out.push(start..end);
            }
            k += needle.len();
        } else {
            k += 1;
        }
    }
    out
}

impl FindState {
    /// A search over the text inside `scope`.
    pub fn new(scope: &El) -> Self {
        FindState {
            scope: scope.clone(),
            query: Signal::new(String::new()),
            count: Signal::new(0),
            current: Signal::new(0),
            hits: Rc::default(),
            marked: Rc::default(),
        }
    }

    /// The texts inside the scope that show, in reading order.
    fn texts(&self) -> Vec<(El, String)> {
        let mut out = Vec::new();
        let mut stack = vec![self.scope.clone()];
        while let Some(e) = stack.pop() {
            if e.info().is_none_or(|i| i.hidden) {
                continue;
            }
            if let Some(t) = e.text_content() {
                out.push((e.clone(), t));
            }
            let mut kids = e.child_elements();
            kids.reverse();
            stack.extend(kids);
        }
        out
    }

    /// Searches for `query`, marking every match and showing the first.
    pub fn search(&self, query: &str) {
        self.query.set(query.to_owned());
        self.refresh();
    }

    /// Searches again for the same text (the content changed), keeping the current match
    /// where it can.
    pub fn refresh(&self) {
        let query = self.query.get_untracked().unwrap_or_default();
        self.unmark();
        let mut hits = Vec::new();
        let mut marked = Vec::new();
        for (el, text) in self.texts() {
            let found = matches(&text, &query);
            if found.is_empty() {
                continue;
            }
            el.mark_text(found.clone());
            marked.push(el.clone());
            hits.extend(found.into_iter().map(|r| (el.clone(), r)));
        }
        let n = hits.len();
        *self.hits.borrow_mut() = hits;
        *self.marked.borrow_mut() = marked;
        self.count.set(n);
        let at = self.current.get_untracked().unwrap_or(0).min(n.saturating_sub(1));
        self.show(at);
    }

    fn show(&self, at: usize) {
        self.current.set(at);
        let hit = self.hits.borrow().get(at).cloned();
        let ui = self.scope.ui().clone();
        match hit {
            Some((el, range)) => {
                ui.select_text(&el, range);
                el.scroll_into_view();
            }
            None => ui.clear_text_selection(),
        }
    }

    /// Goes to the next match, round to the first after the last.
    pub fn next(&self) {
        let n = self.hits.borrow().len();
        if n > 0 {
            self.show((self.current.get_untracked().unwrap_or(0) + 1) % n);
        }
    }

    /// Goes to the previous match, round to the last before the first.
    pub fn previous(&self) {
        let n = self.hits.borrow().len();
        if n > 0 {
            self.show((self.current.get_untracked().unwrap_or(0) + n - 1) % n);
        }
    }

    fn unmark(&self) {
        for el in self.marked.borrow_mut().drain(..) {
            el.mark_text(Vec::new());
        }
    }

    /// Ends the search: no marks, nothing selected.
    pub fn clear(&self) {
        self.unmark();
        self.hits.borrow_mut().clear();
        self.count.set(0);
        self.current.set(0);
        self.query.set(String::new());
        self.scope.ui().clear_text_selection();
    }

    /// What the bar says: "3 of 12", "No matches", or nothing before a search.
    pub fn status(&self) -> String {
        self.status_in("en")
    }

    /// [`FindState::status`] in `locale` (BCP 47), with RemedyUI's built-in words; English
    /// where it has none.
    pub fn status_in(&self, locale: &str) -> String {
        let n = self.count.get();
        if self.query.get().is_empty() {
            String::new()
        } else if n == 0 {
            remedy_ui_view::words::word_or(locale, "No matches").to_owned()
        } else {
            remedy_ui_view::words::fill(
                remedy_ui_view::words::word_or(locale, "{number} of {total}"),
                &[("number", &(self.current.get() + 1).to_string()), ("total", &n.to_string())],
            )
        }
    }
}

/// A find bar for `state`. `close` is called when people close it (Escape or its close
/// button); the search is cleared first.
#[track_caller]
pub fn find_bar(ui: &Ui, state: &FindState, close: impl Fn() + 'static) -> El {
    let close: Rc<dyn Fn()> = Rc::new(close);
    let s = state.clone();
    let s2 = state.clone();
    let field = w::text_input(ui, ui.word("Find"))
        .placeholder(ui.word("Find"))
        .on_submit_with(move |m| if m.shift { s.previous() } else { s.next() });
    let f2 = field.clone();
    let field = field.on_change(move || s2.search(&f2.value()));
    let st = state.clone();
    let (u, u2) = (ui.clone(), ui.clone());
    let status = ui
        .el("label")
        .variant("muted")
        .data_text()
        .nowrap()
        .text(move || st.status_in(&u.locale()))
        .semantics({
            let st = state.clone();
            move || Semantics::new(Role::Status).label(st.status_in(&u2.locale())).live(Live::Polite)
        });
    let (p, n) = (state.clone(), state.clone());
    let prev = w::icon_button(ui, Icon::ChevronUp, ui.word("Previous match"), move || p.previous());
    let next = w::icon_button(ui, Icon::ChevronDown, ui.word("Next match"), move || n.next());
    let (c, cl) = (state.clone(), close.clone());
    let shut = w::icon_button(ui, Icon::Close, ui.word("Close find"), move || {
        c.clear();
        cl();
    });
    let unit = ui.theme().spacing.unit;
    let (k, kc) = (state.clone(), close);
    ui.el("find_bar")
        .row()
        .fill_width()
        .gap(unit * 2.0)
        .align(remedy_ui_view::Align::Center)
        .child(field.el().grow(1.0))
        .child(status)
        .child(prev)
        .child(next)
        .child(shut)
        .semantics(Semantics::new(Role::Group).label(ui.word("Find")))
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            match e.key {
                Key::Escape => {
                    k.clear();
                    kc();
                    true
                }
                Key::F(3) if e.modifiers.shift => {
                    k.previous();
                    true
                }
                Key::F(3) => {
                    k.next();
                    true
                }
                _ => false,
            }
        })
}
