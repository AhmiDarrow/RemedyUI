//! The signal graph of an element (its bindings, what they read, and what else depends on
//! that) and the "why did this update" log: each element marked for work in a window, with
//! the signal writes that made its binding re-run.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use remedy_ui_core::reactive::debug::{self, NodeInfo, NodeKey, Tracing};
use remedy_ui_engine::view::{DirtyRecord, ElementId, Ui};

/// Something downstream of a signal, and the element it belongs to when it is a binding or
/// painter of the inspected window.
#[derive(Clone, Debug, PartialEq)]
pub struct Dependent {
    /// The node.
    pub node: NodeInfo,
    /// The element whose binding or painter it is.
    pub element: Option<ElementId>,
    /// That element, described ([`crate::describe`]).
    pub element_label: Option<String>,
}

/// A signal or memo a binding read, and everything downstream of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    /// The signal or memo.
    pub node: NodeInfo,
    /// What depends on it, directly or through memos, nearest first.
    pub dependents: Vec<Dependent>,
}

/// One binding or painter of an element, with what it read the last time it ran.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The effect (binding) or reaction (painter).
    pub node: NodeInfo,
    /// What it read.
    pub reads: Vec<Read>,
}

fn dependent(ui: &Ui, key: NodeKey) -> Option<Dependent> {
    let node = debug::node(key)?;
    let element = ui.element_of_binding(key);
    let element_label = element.and_then(|e| ui.element(e).info()).map(|i| crate::describe(&i));
    Some(Dependent { node, element, element_label })
}

/// The bindings and painter of `element`, what each read, and what depends on those reads
/// (in this window or anywhere on its thread).
pub fn element_signals(ui: &Ui, element: ElementId) -> Vec<Binding> {
    let Some(info) = ui.element(element).info() else { return Vec::new() };
    info.binding_keys
        .iter()
        .filter_map(|k| debug::node(*k))
        .map(|node| {
            let reads = node
                .sources
                .iter()
                .filter_map(|s| debug::node(*s))
                .map(|src| {
                    let dependents =
                        debug::dependents(src.key).into_iter().filter_map(|d| dependent(ui, d)).collect();
                    Read { node: src, dependents }
                })
                .collect();
            Binding { node, reads }
        })
        .collect()
}

/// The widget names of what devtools draw into an inspected window (the inspector's outline,
/// the performance overlay): their own updates are not the app's and stay out of the log.
pub const DEVTOOLS_WIDGETS: [&str; 2] = ["inspector_highlight", "perf_overlay"];

/// True when `id` is, or is inside, something devtools drew into the window.
fn is_devtools(ui: &Ui, id: ElementId) -> bool {
    let mut cur = Some(ui.element(id));
    while let Some(e) = cur {
        if e.info().is_some_and(|i| DEVTOOLS_WIDGETS.contains(&i.widget)) {
            return true;
        }
        cur = e.parent();
    }
    false
}

/// One element marked for work, with why.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct UpdateEntry {
    /// The mark as the view reported it.
    pub record: DirtyRecord,
    /// The element, described when it was marked ([`crate::describe`]).
    pub element_label: String,
}

impl UpdateEntry {
    /// "row “42” repainted because its text binding re-ran: `message.edited` changed (set at
    /// chat.rs:88)".
    pub fn sentence(&self) -> String {
        format!("{} {} because {}", self.element_label, self.record.verb(), self.record.because())
    }
}

/// The "why did this update" log of one window: while it lives, reactive tracing is on for
/// the window's thread and every element marked for work in the window is recorded, newest
/// last, keeping as many entries as asked for ([`UpdateLog::set_keep`]). Dropping the last
/// handle stops recording.
#[derive(Clone)]
pub struct UpdateLog(Rc<LogInner>);

struct LogInner {
    target: Ui,
    entries: RefCell<VecDeque<UpdateEntry>>,
    keep: Cell<usize>,
    listener: RefCell<Option<Rc<dyn Fn()>>>,
    observer: Cell<u64>,
    _tracing: Tracing,
}

impl Drop for LogInner {
    fn drop(&mut self) {
        self.target.remove_dirty_observer(self.observer.get());
    }
}

impl std::fmt::Debug for UpdateLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UpdateLog({} entries)", self.0.entries.borrow().len())
    }
}

impl UpdateLog {
    /// Starts recording why elements of `target` update, keeping the newest `keep` entries.
    pub fn attach(target: &Ui, keep: usize) -> UpdateLog {
        let inner = Rc::new(LogInner {
            target: target.clone(),
            entries: RefCell::new(VecDeque::new()),
            keep: Cell::new(keep.max(1)),
            listener: RefCell::new(None),
            observer: Cell::new(0),
            _tracing: debug::trace(),
        });
        let weak: Weak<LogInner> = Rc::downgrade(&inner);
        let id = target.add_dirty_observer(Rc::new(move |r: &DirtyRecord| {
            if let Some(inner) = weak.upgrade() {
                UpdateLog(inner).push(r);
            }
        }));
        inner.observer.set(id);
        UpdateLog(inner)
    }

    fn push(&self, r: &DirtyRecord) {
        if is_devtools(&self.0.target, r.element) {
            return;
        }
        let label = self
            .0
            .target
            .element(r.element)
            .info()
            .map_or_else(|| format!("element {:?}", r.element), |i| crate::describe(&i));
        {
            let mut e = self.0.entries.borrow_mut();
            e.push_back(UpdateEntry { record: r.clone(), element_label: label });
            while e.len() > self.0.keep.get() {
                e.pop_front();
            }
        }
        let listener = self.0.listener.borrow().clone();
        if let Some(f) = listener {
            f();
        }
    }

    /// The entries, oldest first.
    pub fn entries(&self) -> Vec<UpdateEntry> {
        self.0.entries.borrow().iter().cloned().collect()
    }

    /// The entries about `element`, oldest first.
    pub fn entries_for(&self, element: ElementId) -> Vec<UpdateEntry> {
        self.0.entries.borrow().iter().filter(|e| e.record.element == element).cloned().collect()
    }

    /// How many entries it keeps.
    pub fn keep(&self) -> usize {
        self.0.keep.get()
    }

    /// Keeps the newest `keep` entries from now on.
    pub fn set_keep(&self, keep: usize) {
        self.0.keep.set(keep.max(1));
        let mut e = self.0.entries.borrow_mut();
        while e.len() > self.0.keep.get() {
            e.pop_front();
        }
    }

    /// Forgets every entry.
    pub fn clear(&self) {
        self.0.entries.borrow_mut().clear();
    }

    /// Calls `f` after each new entry (inside the mark that made it: `f` should only
    /// schedule work).
    pub fn on_entry(&self, f: impl Fn() + 'static) {
        *self.0.listener.borrow_mut() = Some(Rc::new(f));
    }

    /// The window it records.
    pub fn target(&self) -> &Ui {
        &self.0.target
    }
}
