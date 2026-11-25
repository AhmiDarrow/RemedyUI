//! Headless testing for RemedyUI.
//!
//! A [`Harness`] owns a window's element tree, a fake clock and the frame loop, so tests can
//! drive an app the way a user would and check exactly what each frame had to do. Time only
//! moves when the test moves it, so animations, timers and debounces are deterministic.
//!
//! This is the M1 cut: tree, clock, timers, posted work, scripted events and focus. Layout,
//! rendering, input recognition and screenshots join as their crates land.
//!
//! ```
//! use remedy_ui_core::reactive::{Effect, Signal};
//! use remedy_ui_core::testing::Harness;
//! use remedy_ui_core::tree::Dirty;
//! use std::time::Duration;
//!
//! let h = Harness::new();
//! let label = h.element(h.tree().root(), "label");
//! let text = Signal::new("Saving…".to_string());
//! let tree = h.tree().clone();
//! h.tree().in_scope(label, move || {
//!     Effect::new(move || {
//!         text.with(|_| ());
//!         tree.mark(label, Dirty::PAINT);
//!     });
//! }).ok();
//! h.frame(); // first frame paints everything
//!
//! h.after(Duration::from_millis(400), move || text.set("Saved".into()));
//! assert!(h.frame().is_idle(), "nothing changes until the clock moves");
//! h.advance(Duration::from_millis(400));
//! // The label repaints; the root is on the path to its layer.
//! assert_eq!(h.frame().paint, [h.tree().root(), label]);
//! ```

use std::cell::{Cell, RefCell};
use std::time::Duration;

use crate::reactive::{PostReport, batch, run_posted};
use crate::tree::{Dirty, Dispatch, ElementId, Tree, TreeError};

struct Timer {
    due: Duration,
    seq: u64,
    f: Box<dyn FnOnce()>,
}

/// What one frame step had to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Posts from other threads that ran at the start of the frame.
    pub posts: PostReport,
    /// Elements whose style was re-resolved.
    pub style: Vec<ElementId>,
    /// Elements laid out again (including ones on the path to them).
    pub layout: Vec<ElementId>,
    /// Elements whose paint chunk was re-recorded (including ones on the path to them).
    pub paint: Vec<ElementId>,
    /// Elements whose layers only need compositing.
    pub composite: Vec<ElementId>,
    /// Elements whose accessibility nodes were rebuilt.
    pub semantics: Vec<ElementId>,
}

impl Frame {
    /// True when the frame had no work at all.
    pub fn is_idle(&self) -> bool {
        self.posts.ran == 0
            && self.posts.panicked == 0
            && self.style.is_empty()
            && self.layout.is_empty()
            && self.paint.is_empty()
            && self.composite.is_empty()
            && self.semantics.is_empty()
    }
}

/// A headless window: its tree, a fake clock and the frame loop.
pub struct Harness {
    tree: Tree,
    now: Cell<Duration>,
    timers: RefCell<Vec<Timer>>,
    next_seq: Cell<u64>,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Harness").field("now", &self.now.get()).field("tree", &self.tree).finish()
    }
}

impl Harness {
    /// A harness with an empty tree at time zero.
    #[track_caller]
    pub fn new() -> Self {
        Harness {
            tree: Tree::new(),
            now: Cell::new(Duration::ZERO),
            timers: RefCell::new(Vec::new()),
            next_seq: Cell::new(0),
        }
    }

    /// The window's tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Creates an element named `name` under `parent`. Panics if `parent` does not exist,
    /// which in a test is a mistake in the test.
    #[track_caller]
    pub fn element(&self, parent: ElementId, name: &'static str) -> ElementId {
        let e = self.tree.create(name);
        if let Err(err) = self.tree.append(parent, e) {
            test_failure(&format!("could not add `{name}`: {err}"));
        }
        e
    }

    /// The first element whose path from the root ends with `path`, such as `"dialog/ok"`.
    pub fn find(&self, path: &str) -> Option<ElementId> {
        self.tree.descendants(self.tree.root()).into_iter().find(|e| self.tree.path(*e).ends_with(path))
    }

    // ---------- time ----------

    /// The fake clock's time since the harness started.
    pub fn now(&self) -> Duration {
        self.now.get()
    }

    /// Runs `f` when the clock reaches `delay` from now.
    pub fn after(&self, delay: Duration, f: impl FnOnce() + 'static) {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        self.timers.borrow_mut().push(Timer { due: self.now.get() + delay, seq, f: Box::new(f) });
    }

    /// Moves the clock forward, firing every timer that comes due, in time order (ties in the
    /// order they were set). Each timer runs in its own batch. Returns how many fired.
    pub fn advance(&self, by: Duration) -> usize {
        let end = self.now.get() + by;
        let mut fired = 0;
        loop {
            let next = {
                let mut timers = self.timers.borrow_mut();
                let pos = timers
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.due <= end)
                    .min_by_key(|(_, t)| (t.due, t.seq))
                    .map(|(i, _)| i);
                pos.map(|i| timers.swap_remove(i))
            };
            let Some(timer) = next else { break };
            self.now.set(timer.due);
            batch(timer.f);
            fired += 1;
        }
        self.now.set(end);
        fired
    }

    /// Timers waiting to fire.
    pub fn pending_timers(&self) -> usize {
        self.timers.borrow().len()
    }

    // ---------- frames ----------

    /// Runs one frame: posted work first, then each phase takes exactly its dirty elements.
    /// Until layout and paint land, the phases record what they would redo.
    pub fn frame(&self) -> Frame {
        let posts = run_posted();
        Frame {
            posts,
            style: self.tree.take_dirty(Dirty::STYLE),
            layout: self.tree.take_dirty(Dirty::LAYOUT | Dirty::CHILD_LAYOUT),
            paint: self.tree.take_dirty(Dirty::PAINT | Dirty::CHILD_PAINT),
            composite: self.tree.take_dirty(Dirty::COMPOSITE),
            semantics: self.tree.take_dirty(Dirty::SEMANTICS),
        }
    }

    // ---------- scripted input ----------

    /// Dispatches `event` to `target`, as input would.
    pub fn send<E: 'static>(&self, target: ElementId, event: &E) -> Result<Dispatch, TreeError> {
        self.tree.dispatch(target, event)
    }

    /// Presses Tab: focus moves to the next focusable element.
    pub fn tab(&self) -> Option<ElementId> {
        self.tree.focus_next()
    }

    /// Presses Shift+Tab: focus moves to the previous focusable element.
    pub fn shift_tab(&self) -> Option<ElementId> {
        self.tree.focus_previous()
    }
}

#[track_caller]
#[allow(clippy::panic, reason = "a harness helper misused by a test fails that test")]
fn test_failure(msg: &str) -> ! {
    panic!("{msg}")
}
