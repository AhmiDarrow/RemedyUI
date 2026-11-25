//! Spell checking for text fields: misspellings underlined as people type, and a menu of
//! corrections.
//!
//! Checking never waits on the UI thread. A change to the text starts a short timer; when it
//! fires, the text is sent to the checker's thread, and the answer comes back through the
//! reactive runtime's waker. Typing again before the answer arrives drops that check. The word
//! being typed is left unmarked until typing pauses.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_core::Point;
use remedy_ui_engine::view::{Decoration, El, FieldKind, TextField, Ui};
use crate::base::{MenuItem, context_menu_at};

use crate::spell::{PendingCheck, SpellChecker, SpellError};

/// The words in the spelling menu. The default is English; a field left with the default
/// shows them in the window's language ([`MenuLabels::localized`]), and labels an app sets
/// are shown as given.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MenuLabels {
    /// The entry that adds the word to the personal dictionary.
    pub add_to_dictionary: String,
    /// The entry that ignores the word until the app ends.
    pub ignore: String,
    /// The entry shown when there are no corrections.
    pub no_suggestions: String,
}

impl Default for MenuLabels {
    fn default() -> Self {
        MenuLabels {
            add_to_dictionary: "Add to dictionary".into(),
            ignore: "Ignore".into(),
            no_suggestions: "No suggestions".into(),
        }
    }
}

impl MenuLabels {
    /// The menu's words in `locale`, from the engine's catalog ([`crate::spell::STRINGS`]).
    pub fn localized(locale: &str) -> Self {
        let t = |id| crate::spell::STRINGS.format_in(locale, id, &[]);
        MenuLabels {
            add_to_dictionary: t("spell-add-to-dictionary"),
            ignore: t("spell-ignore"),
            no_suggestions: t("spell-no-suggestions"),
        }
    }
}

/// How a field is spell checked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpellOptions {
    /// How long after the last change the text is sent to be checked.
    pub debounce: Duration,
    /// How long after the last change the word at the caret is marked too.
    pub settle: Duration,
    /// Check the window's locale as well as the checker's own languages (default true).
    pub follow_locale: bool,
    /// The words in the menu.
    pub labels: MenuLabels,
}

impl SpellOptions {
    /// Sets `debounce`: how long after the last change the text is sent to be checked.
    pub fn debounce(mut self, value: Duration) -> Self {
        self.debounce = value;
        self
    }

    /// Sets `settle`: how long after the last change the word at the caret is marked too.
    pub fn settle(mut self, value: Duration) -> Self {
        self.settle = value;
        self
    }

    /// Sets `follow_locale`: check the window's locale as well as the checker's own languages (default true).
    pub fn follow_locale(mut self, value: bool) -> Self {
        self.follow_locale = value;
        self
    }

    /// Sets `labels`: the words in the menu.
    pub fn labels(mut self, value: MenuLabels) -> Self {
        self.labels = value;
        self
    }
}

impl Default for SpellOptions {
    fn default() -> Self {
        SpellOptions {
            debounce: Duration::from_millis(150),
            settle: Duration::from_millis(900),
            follow_locale: true,
            labels: MenuLabels::default(),
        }
    }
}

struct State {
    id: u64,
    ui: Ui,
    field: TextField,
    checker: SpellChecker,
    options: SpellOptions,
    attached: bool,
    pending: Option<(PendingCheck, String)>,
    debounce_timer: Option<u64>,
    settle_timer: Option<u64>,
    /// The text last checked and what was found in it.
    result: Option<(String, Vec<Decoration>)>,
    error: Option<SpellError>,
    last_edit: Instant,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    /// Hooks waiting for answers, by id: the waker's closures run on this thread and find
    /// them here, since the closures themselves must be `Send`.
    static HOOKS: RefCell<HashMap<u64, Weak<RefCell<State>>>> = RefCell::new(HashMap::new());
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// The words `dictionary` accepts changed: every field on this thread checked with it checks
/// again (soon, not inside the caller: a menu or settings page may be mid-update).
pub(crate) fn dictionary_changed(dictionary: &crate::spell::PersonalDictionary) {
    let hooks: Vec<Shared> = HOOKS.with(|h| h.borrow().values().filter_map(Weak::upgrade).collect());
    for s in hooks {
        let Ok(state) = s.try_borrow() else { continue };
        if !state.attached || !state.checker.personal().same(dictionary) {
            continue;
        }
        let ui = state.ui.clone();
        drop(state);
        let w = Rc::downgrade(&s);
        let _ = ui.after(std::time::Duration::ZERO, move || {
            if let Some(s) = w.upgrade() {
                start(&s);
            }
        });
    }
}

fn deliver(id: u64) {
    let hook = HOOKS.with(|h| h.borrow().get(&id).and_then(Weak::upgrade));
    match hook {
        Some(s) => finish(&s),
        None => HOOKS.with(|h| {
            h.borrow_mut().remove(&id);
        }),
    }
}

fn cancel_timers(s: &mut State) {
    for t in [s.debounce_timer.take(), s.settle_timer.take()].into_iter().flatten() {
        s.ui.cancel_timer(t);
    }
}

/// Something changed: drop the check in flight and start the timers again.
fn schedule(shared: &Shared) {
    let (ui, debounce, settle) = {
        let mut s = shared.borrow_mut();
        if !s.attached {
            return;
        }
        s.pending = None;
        cancel_timers(&mut s);
        s.last_edit = Instant::now();
        (s.ui.clone(), s.options.debounce, s.options.settle)
    };
    let (w1, w2) = (Rc::downgrade(shared), Rc::downgrade(shared));
    let d = ui.after(debounce, move || {
        if let Some(s) = w1.upgrade() {
            s.borrow_mut().debounce_timer = None;
            start(&s);
        }
    });
    let t = ui.after(settle, move || {
        if let Some(s) = w2.upgrade() {
            s.borrow_mut().settle_timer = None;
            settled(&s);
        }
    });
    let mut s = shared.borrow_mut();
    s.debounce_timer = Some(d);
    s.settle_timer = Some(t);
}

/// The languages a field is checked in now.
fn languages(s: &State) -> Vec<String> {
    if s.options.follow_locale { s.checker.languages_for(&s.ui.locale()) } else { s.checker.languages() }
}

/// Sends the field's text to be checked.
fn start(shared: &Shared) {
    let mut s = shared.borrow_mut();
    if !s.attached {
        return;
    }
    let text = s.field.value();
    s.pending = None;
    if text.is_empty() {
        s.result = Some((text, Vec::new()));
        s.error = None;
        s.field.set_decorations(Vec::new());
        return;
    }
    let langs = languages(&s);
    let id = s.id;
    let waker = remedy_ui_core::reactive::waker();
    let notify: Box<dyn FnOnce() + Send> = Box::new(move || {
        if let Ok(w) = waker {
            let _ = w.post(move || deliver(id));
        }
    });
    match s.checker.check_async_notify(&text, &langs, Some(notify)) {
        Ok(p) => s.pending = Some((p, text)),
        Err(e) => {
            s.error = Some(e);
            s.result = None;
            s.field.set_decorations(Vec::new());
        }
    }
}

/// An answer may have arrived.
fn finish(shared: &Shared) {
    {
        let mut s = shared.borrow_mut();
        let Some((mut p, text)) = s.pending.take() else { return };
        match p.poll() {
            None => {
                s.pending = Some((p, text));
                return;
            }
            Some(Ok(ranges)) => {
                let marks = ranges.into_iter().map(Decoration::squiggle).collect();
                s.error = None;
                s.result = Some((text, marks));
            }
            Some(Err(e)) => {
                s.error = Some(e);
                s.result = None;
                s.field.set_decorations(Vec::new());
                return;
            }
        }
    }
    show(shared, false);
}

/// Typing has paused: the word at the caret is marked too.
fn settled(shared: &Shared) {
    show(shared, true);
}

/// Puts the last answer on the field. Until typing pauses the word at the caret is left out,
/// since it is probably not finished.
fn show(shared: &Shared, everything: bool) {
    let s = shared.borrow();
    let Some((text, marks)) = s.result.clone() else { return };
    s.field.set_decorations_for(&text, marks);
    let typing = !everything && s.last_edit.elapsed() < s.options.settle && s.field.el().is_focused();
    if typing {
        let caret = s.field.caret();
        let all = s.field.decorations();
        let rest: Vec<Decoration> =
            all.iter().filter(|d| !(d.range.start <= caret && caret <= d.range.end)).cloned().collect();
        if rest.len() != all.len() {
            s.field.set_decorations(rest);
        }
    }
}

/// A field's spell checking. Dropping it leaves the checking running; [`SpellHook::detach`]
/// stops it.
#[derive(Clone)]
pub struct SpellHook {
    shared: Shared,
}

impl std::fmt::Debug for SpellHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpellHook({})", self.shared.borrow().id)
    }
}

impl SpellHook {
    /// Checks the field again now (after the personal dictionary or the languages changed).
    pub fn recheck(&self) {
        start(&self.shared);
    }

    /// Stops checking and removes the marks.
    pub fn detach(&self) {
        let mut s = self.shared.borrow_mut();
        s.attached = false;
        s.pending = None;
        cancel_timers(&mut s);
        s.result = None;
        s.field.set_decorations(Vec::new());
        let id = s.id;
        drop(s);
        HOOKS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// True until [`SpellHook::detach`].
    pub fn is_attached(&self) -> bool {
        self.shared.borrow().attached
    }

    /// True while a check is waiting for its answer or its timer.
    pub fn is_checking(&self) -> bool {
        let s = self.shared.borrow();
        s.pending.is_some() || s.debounce_timer.is_some()
    }

    /// Why the last check gave nothing, such as a language no engine can check. `None` when
    /// it worked.
    pub fn last_error(&self) -> Option<SpellError> {
        self.shared.borrow().error.clone()
    }

    /// The byte ranges now marked.
    pub fn misspellings(&self) -> Vec<Range<usize>> {
        let f = self.shared.borrow().field.clone();
        f.decorations().into_iter().map(|d| d.range).collect()
    }

    /// The languages the field is checked in now.
    pub fn languages(&self) -> Vec<String> {
        languages(&self.shared.borrow())
    }

    /// The spelling entries of a context menu for the word at `at` (a window point; `None`
    /// uses the caret): corrections that replace the word, then "Add to dictionary" and
    /// "Ignore". Empty when the point is not on a marked word. Corrections are asked for
    /// when this runs, so it waits for the checking thread briefly.
    pub fn menu_items(&self, at: Option<Point>) -> Vec<MenuItem> {
        let (field, checker, labels, langs) = {
            let s = self.shared.borrow();
            // The app's own words when it gave some; the window's language otherwise.
            let labels = if s.options.labels == MenuLabels::default() {
                MenuLabels::localized(&s.ui.locale())
            } else {
                s.options.labels.clone()
            };
            (s.field.clone(), s.checker.clone(), labels, languages(&s))
        };
        let mark = match at {
            Some(p) => field.decoration_at(p),
            None => {
                let caret = field.caret();
                field.decorations().into_iter().find(|d| d.range.start <= caret && caret <= d.range.end)
            }
        };
        let Some(mark) = mark else { return Vec::new() };
        let text = field.value();
        let Some(word) = text.get(mark.range.clone()).map(str::to_owned) else { return Vec::new() };
        let mut items = Vec::new();
        let suggestions = checker.suggest_in(&word, &langs).unwrap_or_default();
        if suggestions.is_empty() {
            items.push(MenuItem::new(labels.no_suggestions.clone(), || {}));
        }
        for s in suggestions {
            let (f, range, old) = (field.clone(), mark.range.clone(), word.clone());
            let label = s.clone();
            items.push(MenuItem::new(label, move || {
                // Only if the word is still where the menu found it.
                if f.value().get(range.clone()) == Some(old.as_str()) {
                    f.replace_range(range.clone(), &s);
                }
            }));
        }
        // The dictionary tells every field using it to check again.
        let (c1, w1) = (checker.clone(), word.clone());
        items.push(MenuItem::new(labels.add_to_dictionary.clone(), move || {
            c1.add_word(&w1);
        }));
        let (c2, w2) = (checker, word);
        items.push(MenuItem::new(labels.ignore.clone(), move || {
            c2.ignore_word(&w2);
        }));
        items
    }

    /// Opens the spelling menu on a right click or the Menu key over the field, followed by
    /// `extra` (the field's own entries, such as Cut, Copy and Paste). Replaces any other
    /// context menu on the field.
    pub fn context_menu(&self, extra: impl Fn() -> Vec<MenuItem> + 'static) -> El {
        let hook = self.clone();
        let el = self.shared.borrow().field.el();
        context_menu_at(&el, move |at| {
            let mut items = hook.menu_items(at);
            items.extend(extra());
            items
        })
    }
}

/// Spell checking for [`TextField`]s.
pub trait SpellCheckField {
    /// Marks misspelled words in the field with the default options. Returns `None` for
    /// secret fields, which are never checked.
    fn spell_check(&self, checker: &SpellChecker) -> Option<SpellHook>;

    /// Like [`SpellCheckField::spell_check`] with options.
    fn spell_check_with(&self, checker: &SpellChecker, options: SpellOptions) -> Option<SpellHook>;
}

impl SpellCheckField for TextField {
    fn spell_check(&self, checker: &SpellChecker) -> Option<SpellHook> {
        self.spell_check_with(checker, SpellOptions::default())
    }

    fn spell_check_with(&self, checker: &SpellChecker, options: SpellOptions) -> Option<SpellHook> {
        if self.kind() == FieldKind::Secret {
            return None;
        }
        let id = NEXT.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        let ui = self.el().ui().clone();
        let follow = options.follow_locale;
        let shared: Shared = Rc::new(RefCell::new(State {
            id,
            ui: ui.clone(),
            field: self.clone(),
            checker: checker.clone(),
            options,
            attached: true,
            pending: None,
            debounce_timer: None,
            settle_timer: None,
            result: None,
            error: None,
            last_edit: Instant::now(),
        }));
        HOOKS.with(|h| h.borrow_mut().insert(id, Rc::downgrade(&shared)));
        let s2 = shared.clone();
        self.observe_text(move || schedule(&s2));
        if follow {
            // A new locale means new languages: check again.
            let (s3, first) = (shared.clone(), std::cell::Cell::new(true));
            let _ = self.el().bind(move || {
                let _ = ui.locale();
                if !first.replace(false) {
                    schedule(&s3);
                }
            });
        }
        schedule(&shared);
        Some(SpellHook { shared })
    }
}
