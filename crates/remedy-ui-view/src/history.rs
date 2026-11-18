//! A window's undo history: the changes the app (or an agent acting in it) made that people
//! can take back, each named ("Undo: turn on Noise suppression, by assistant"). Text fields
//! keep their own history of typing; this one holds everything else. Ctrl+Z (Command+Z)
//! undoes the latest when no field has focus, Ctrl+Y or Ctrl+Shift+Z redoes.

use std::rc::Rc;

use crate::Ui;

type Action = Rc<dyn Fn()>;

/// One change in the history.
#[derive(Clone)]
pub(crate) struct Entry {
    label: String,
    by: Option<String>,
    undo: Action,
    redo: Action,
}

/// What a history entry says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryItem {
    /// What the change was ("Turn on Noise suppression").
    pub label: String,
    /// Who made it, when it was not the person at the keyboard ("assistant").
    pub by: Option<String>,
}

impl HistoryItem {
    /// What an Undo menu item says: "Undo Turn on Noise suppression, by assistant".
    pub fn undo_label(&self) -> String {
        match &self.by {
            Some(by) => format!("Undo {}, by {by}", self.label),
            None => format!("Undo {}", self.label),
        }
    }
}

/// Both stacks of a window's history.
#[derive(Default)]
pub(crate) struct History {
    done: Vec<Entry>,
    undone: Vec<Entry>,
    /// Who is acting now, when it is not the person at the keyboard.
    actor: Option<String>,
}

impl Ui {
    /// Records a change already made: `undo` takes it back and `redo` makes it again. `by`
    /// names who made it when it was not the person at the keyboard (an agent).
    pub fn record_change(
        &self,
        label: impl Into<String>,
        by: Option<String>,
        undo: impl Fn() + 'static,
        redo: impl Fn() + 'static,
    ) {
        let mut i = self.0.borrow_mut();
        // A change made while an agent acts is the agent's, unless said otherwise.
        let by = by.or_else(|| i.history.actor.clone());
        i.history.done.push(Entry { label: label.into(), by, undo: Rc::new(undo), redo: Rc::new(redo) });
        i.history.undone.clear();
    }

    /// Runs `f` as `agent`: changes recorded while it runs are the agent's.
    pub fn acting_as<T>(&self, agent: &str, f: impl FnOnce() -> T) -> T {
        let before = self.0.borrow_mut().history.actor.replace(agent.to_owned());
        let out = f();
        self.0.borrow_mut().history.actor = before;
        out
    }

    /// The changes that can be undone, latest last.
    pub fn history(&self) -> Vec<HistoryItem> {
        self.0
            .borrow()
            .history
            .done
            .iter()
            .map(|e| HistoryItem { label: e.label.clone(), by: e.by.clone() })
            .collect()
    }

    /// The change Undo would take back.
    pub fn next_undo(&self) -> Option<HistoryItem> {
        self.0.borrow().history.done.last().map(|e| HistoryItem { label: e.label.clone(), by: e.by.clone() })
    }

    /// Takes back the latest change. False when there is none.
    pub fn undo_change(&self) -> bool {
        let entry = self.0.borrow_mut().history.done.pop();
        let Some(e) = entry else { return false };
        (e.undo)();
        self.0.borrow_mut().history.undone.push(e);
        true
    }

    /// Makes the latest undone change again. False when there is none.
    pub fn redo_change(&self) -> bool {
        let entry = self.0.borrow_mut().history.undone.pop();
        let Some(e) = entry else { return false };
        (e.redo)();
        self.0.borrow_mut().history.done.push(e);
        true
    }
}
