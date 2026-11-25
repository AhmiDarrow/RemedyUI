//! Forms: fields validated as they change, dirty tracking against the last saved values,
//! and a submit state that slow saves can report back to.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::a11y::{Live, Role, Semantics};
use remedy_ui_core::reactive::{Memo, Signal, untrack};
use remedy_ui_core::style::States;
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_widgets_core::{self as w, Rejection};

/// Where a form's submission stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubmitState {
    /// Nothing submitted since the last change.
    #[default]
    Idle,
    /// Submit was pressed with fields in error.
    Invalid,
    /// The save is running.
    Submitting,
    /// Saved; the form is clean.
    Saved,
    /// The save failed with this message.
    Failed(String),
}

struct Entry {
    label: String,
    error: Memo<Option<String>>,
    dirty: Memo<bool>,
    shown: Signal<bool>,
    reset: Box<dyn Fn()>,
    commit: Box<dyn Fn()>,
}

/// A set of fields submitted together.
#[derive(Clone)]
pub struct Form(Rc<Inner>);

struct Inner {
    entries: RefCell<Vec<Rc<Entry>>>,
    state: Signal<SubmitState>,
    count: Signal<usize>,
}

/// One field of a [`Form`].
#[derive(Clone)]
pub struct FormField(Rc<Entry>);

impl FormField {
    /// The field's error (tracked), whether or not it is shown yet.
    pub fn error(&self) -> Option<String> {
        self.0.error.get()
    }

    /// True once the error should be shown: after the first edit, or after submit.
    pub fn error_shown(&self) -> bool {
        self.0.shown.get()
    }

    /// True when the value differs from the last saved one (tracked).
    pub fn is_dirty(&self) -> bool {
        self.0.dirty.get()
    }

    /// The field's label.
    pub fn label(&self) -> &str {
        &self.0.label
    }
}

/// Lets a save report how it went (from a background task's completion, say).
#[derive(Clone)]
pub struct Submission(Form);

impl Submission {
    /// The save worked: the current values become the saved ones.
    pub fn succeeded(&self) {
        for e in self.0.0.entries.borrow().iter() {
            (e.commit)();
        }
        self.0.0.state.set(SubmitState::Saved);
    }

    /// The save failed; `message` is shown to the user.
    pub fn failed(&self, message: impl Into<String>) {
        self.0.0.state.set(SubmitState::Failed(message.into()));
    }
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    /// An empty form.
    pub fn new() -> Form {
        Form(Rc::new(Inner {
            entries: RefCell::new(Vec::new()),
            state: Signal::new(SubmitState::Idle),
            count: Signal::new(0),
        }))
    }

    /// Adds a field: `value` is checked by `validate` whenever it changes.
    pub fn field<T: Clone + PartialEq + 'static>(
        &self,
        label: impl Into<String>,
        value: Signal<T>,
        validate: impl Fn(&T) -> Result<(), Rejection> + 'static,
    ) -> FormField {
        let saved = Rc::new(RefCell::new(value.get_untracked().ok()));
        let s2 = saved.clone();
        let error = Memo::new(move || value.with(|v| validate(v).err().map(|r| r.to_string())));
        let dirty = Memo::new(move || value.with(|v| s2.borrow().as_ref() != Some(v)));
        let shown = Signal::new(false);
        let state = self.0.state;
        // Any edit reveals the field's error and clears a finished submission (the first run
        // only subscribes).
        let first = std::cell::Cell::new(true);
        let _ = remedy_ui_core::reactive::Effect::new(move || {
            value.with(|_| ());
            if first.replace(false) {
                return;
            }
            untrack(|| {
                if dirty.get() {
                    shown.set_if_changed(true);
                }
                if matches!(
                    state.get_untracked(),
                    Ok(SubmitState::Saved | SubmitState::Failed(_) | SubmitState::Invalid)
                ) {
                    state.set(SubmitState::Idle);
                }
            });
        });
        let (s3, s4) = (saved.clone(), saved);
        let entry = Rc::new(Entry {
            label: label.into(),
            error,
            dirty,
            shown,
            reset: Box::new(move || {
                if let Some(v) = s3.borrow().clone() {
                    value.set(v);
                }
                shown.set(false);
            }),
            commit: Box::new(move || {
                *s4.borrow_mut() = value.get_untracked().ok();
                // The memo compares against `saved`: re-run it.
                value.update(|_| {});
            }),
        });
        self.0.entries.borrow_mut().push(entry.clone());
        self.0.count.update(|n| *n += 1);
        FormField(entry)
    }

    /// True when every field is valid (tracked).
    pub fn is_valid(&self) -> bool {
        self.0.count.get();
        self.0.entries.borrow().iter().all(|e| e.error.get().is_none())
    }

    /// True when any field differs from its saved value (tracked).
    pub fn is_dirty(&self) -> bool {
        self.0.count.get();
        self.0.entries.borrow().iter().any(|e| e.dirty.get())
    }

    /// Labels and errors of the fields in error (tracked).
    pub fn errors(&self) -> Vec<(String, String)> {
        self.0.count.get();
        self.0.entries.borrow().iter().filter_map(|e| e.error.get().map(|m| (e.label.clone(), m))).collect()
    }

    /// The submission state (tracked).
    pub fn state(&self) -> SubmitState {
        self.0.state.get()
    }

    /// Submits: with errors, shows them all and stays put (returns false); otherwise moves to
    /// `Submitting` and calls `save`, which reports back through the [`Submission`] (now
    /// or later).
    pub fn submit(&self, save: impl FnOnce(Submission)) -> bool {
        if self.0.state.get_untracked() == Ok(SubmitState::Submitting) {
            return false;
        }
        let entries = self.0.entries.borrow().clone();
        for e in &entries {
            e.shown.set(true);
        }
        if entries.iter().any(|e| e.error.get_untracked().ok().flatten().is_some()) {
            self.0.state.set(SubmitState::Invalid);
            return false;
        }
        self.0.state.set(SubmitState::Submitting);
        save(Submission(self.clone()));
        true
    }

    /// Puts every field back to its saved value and hides errors.
    pub fn reset(&self) {
        let entries = self.0.entries.borrow().clone();
        for e in entries {
            (e.reset)();
        }
        self.0.state.set(SubmitState::Idle);
    }
}

/// A labeled form field: the label, the control, and the field's error under it once it
/// should be shown. The control is marked invalid while it is.
#[track_caller]
pub fn form_field(ui: &Ui, field: &FormField, control: El) -> El {
    let unit = ui.theme().spacing.unit;
    let f = field.clone();
    let c2 = control.clone();
    let err = w::column(ui);
    let e2 = err.clone();
    let u2 = ui.clone();
    let err = err.bind(move || {
        let msg = f.error().filter(|_| f.error_shown());
        c2.state(States::INVALID, msg.is_some());
        untrack(|| {
            e2.clear();
            if let Some(m) = msg {
                let _ = e2.clone().child(w::text(&u2, m).variant("error"));
            }
        });
    });
    w::column(ui)
        .gap(unit / 2.0)
        .fill_width()
        .child(w::text(ui, field.label().to_owned()))
        .child(control)
        .child(err)
}

/// The form's container: groups its fields for assistive technology.
#[track_caller]
pub fn form_view(ui: &Ui, label: impl Into<String>) -> El {
    let unit = ui.theme().spacing.unit;
    w::column(ui).gap(unit * 3.0).fill_width().semantics(Semantics::new(Role::Form).label(label))
}

/// A submit button: says "Saving…" while the save runs and ignores presses meanwhile.
#[track_caller]
pub fn submit_button(
    ui: &Ui,
    form: &Form,
    label: impl Into<String>,
    save: impl Fn(Submission) + 'static,
) -> El {
    let label = label.into();
    let f = form.clone();
    let save = Rc::new(save);
    let b = w::button(ui, label.clone(), move || {
        let s = save.clone();
        f.submit(move |sub| s(sub));
    })
    .variant("primary");
    let text = b.child_elements().into_iter().next();
    let (f2, b2, u) = (form.clone(), b.clone(), ui.clone());
    b.bind(move || {
        let busy = f2.state() == SubmitState::Submitting;
        let shown = if busy { crate::tr(&u, "form-saving") } else { label.clone() };
        if let Some(t) = &text {
            let _ = t.clone().text(remedy_ui_engine::view::RichText::plain(shown.clone()));
        }
        let _ = b2
            .clone()
            .semantics(Semantics::new(Role::Button).label(shown).action(remedy_ui_core::a11y::Action::Click));
        b2.state(States::DISABLED, busy);
    })
}

/// A line that announces the form's state: what to fix, that it saved, or why it failed.
#[track_caller]
pub fn form_status(ui: &Ui, form: &Form) -> El {
    let f = form.clone();
    let l = w::text(ui, "").variant("muted");
    let (l2, u) = (l.clone(), ui.clone());
    l.bind(move || {
        let (text, variant) = match f.state() {
            SubmitState::Invalid => {
                let n = f.errors().len();
                (crate::STRINGS.format(&u, "form-fix-fields", &[("count", n.into())]), "error")
            }
            SubmitState::Submitting => (crate::tr(&u, "form-saving"), "muted"),
            SubmitState::Saved => (crate::tr(&u, "form-saved"), "muted"),
            SubmitState::Failed(m) => (m, "error"),
            SubmitState::Idle => {
                (if f.is_dirty() { crate::tr(&u, "form-unsaved") } else { String::new() }, "muted")
            }
        };
        let _ = l2.clone().variant(variant).text(remedy_ui_engine::view::RichText::plain(text.clone()));
        let _ = l2.clone().semantics(Semantics::new(Role::Status).label(text).live(Live::Polite));
    })
}
