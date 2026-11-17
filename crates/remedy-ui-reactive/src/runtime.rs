//! The reactive graph: nodes, edges, marking and flushing.
//!
//! One runtime per thread. It holds every node (signal, memo, effect, scope) in a
//! generational arena. The algorithm is push-pull and glitch-free:
//!
//! - Setting a signal marks its direct subscribers `Dirty` and everything further downstream
//!   `Check`. Nothing recomputes then.
//! - A memo is brought up to date when it is read: a `Check` memo first updates its sources
//!   in order and recomputes only if one of them really changed; a `Dirty` memo recomputes.
//!   A memo whose new value equals its old one does not mark its subscribers dirty.
//! - Effects reached by marking are queued and run once each when the outermost batch ends,
//!   in the order they were queued.
//!
//! The runtime never holds a borrow of its node arena while app code runs, so app code may
//! read and write signals freely from inside memos and effects.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use remedy_ui_core::{Arena, Id};

use crate::error::ReactiveError;
use crate::waker::PostQueue;

pub(crate) type NodeId = Id<Node>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum State {
    Clean,
    Check,
    Dirty,
}

pub(crate) type AnyBox = Box<dyn Any>;
pub(crate) type Compute = Rc<dyn Fn(Option<&dyn Any>) -> AnyBox>;
pub(crate) type Equal = fn(&dyn Any, &dyn Any) -> bool;

pub(crate) enum Kind {
    Signal(Rc<RefCell<AnyBox>>),
    Memo { value: Rc<RefCell<Option<AnyBox>>>, compute: Compute, equal: Equal },
    Effect(Rc<RefCell<Box<dyn FnMut()>>>),
    Scope,
}

pub(crate) struct Node {
    kind: Kind,
    state: State,
    queued: bool,
    running: bool,
    sources: Vec<NodeId>,
    subscribers: Vec<NodeId>,
    owner: Option<NodeId>,
    owned: Vec<NodeId>,
    cleanups: Vec<Box<dyn FnOnce()>>,
    pub(crate) created_at: &'static Location<'static>,
}

pub(crate) struct Runtime {
    pub(crate) id: u32,
    nodes: RefCell<Arena<Node>>,
    observer: Cell<Option<NodeId>>,
    owner: Cell<Option<NodeId>>,
    batch_depth: Cell<u32>,
    pending: RefCell<VecDeque<NodeId>>,
    flushing: Cell<bool>,
    pub(crate) posts: Arc<PostQueue>,
    effect_runs: Cell<u64>,
    memo_runs: Cell<u64>,
    runaway_effects: Cell<u64>,
}

/// How many times one effect may run in a single flush before it is treated as a runaway
/// (an effect that keeps writing what it reads and never settles). This is a correctness
/// bound against hangs, not a capacity limit: a settling effect needs a handful of runs.
const RUNAWAY_RUNS: u32 = 10_000;

static NEXT_RUNTIME: AtomicU32 = AtomicU32::new(1);

thread_local! {
    static RUNTIME: Runtime = Runtime::new();
}

/// Runs `f` with this thread's runtime.
pub(crate) fn with_rt<R>(f: impl FnOnce(&Runtime) -> R) -> Result<R, ReactiveError> {
    RUNTIME.try_with(f).map_err(|_| ReactiveError::RuntimeGone)
}

/// Restores a `Cell` when dropped, so a panic in app code cannot leave the runtime with the
/// wrong observer, owner or flags.
struct Restore<'a, T: Copy> {
    cell: &'a Cell<T>,
    old: T,
}

impl<T: Copy> Drop for Restore<'_, T> {
    fn drop(&mut self) {
        self.cell.set(self.old);
    }
}

fn swap<T: Copy>(cell: &Cell<T>, new: T) -> Restore<'_, T> {
    let old = cell.replace(new);
    Restore { cell, old }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.posts.close();
    }
}

impl Runtime {
    fn new() -> Self {
        Runtime {
            id: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            nodes: RefCell::new(Arena::new()),
            observer: Cell::new(None),
            owner: Cell::new(None),
            batch_depth: Cell::new(0),
            pending: RefCell::new(VecDeque::new()),
            flushing: Cell::new(false),
            posts: Arc::new(PostQueue::default()),
            effect_runs: Cell::new(0),
            memo_runs: Cell::new(0),
            runaway_effects: Cell::new(0),
        }
    }

    // ---------- creating and disposing ----------

    pub(crate) fn create(&self, kind: Kind, created_at: &'static Location<'static>) -> NodeId {
        let owner = self.owner.get();
        let state = match kind {
            Kind::Signal(_) | Kind::Scope => State::Clean,
            Kind::Memo { .. } | Kind::Effect(_) => State::Dirty,
        };
        let node = Node {
            kind,
            state,
            queued: false,
            running: false,
            sources: Vec::new(),
            subscribers: Vec::new(),
            owner,
            owned: Vec::new(),
            cleanups: Vec::new(),
            created_at,
        };
        let mut nodes = self.nodes.borrow_mut();
        let id = match nodes.insert(node) {
            Ok(id) => id,
            // Four billion live nodes: memory is long gone before this. Treat it like an
            // allocation failure, which aborts in Rust too.
            Err(_) => std::process::abort(),
        };
        if let Some(owner) = owner
            && let Some(o) = nodes.get_mut(owner)
        {
            o.owned.push(id);
        }
        id
    }

    pub(crate) fn exists(&self, id: NodeId) -> bool {
        self.nodes.borrow().contains(id)
    }

    pub(crate) fn created_at(&self, id: NodeId) -> Option<&'static Location<'static>> {
        self.nodes.borrow().get(id).map(|n| n.created_at)
    }

    /// Disposes `id`, everything it owns, and runs their clean-ups (children first, then each
    /// node's own clean-ups in reverse order of registration).
    pub(crate) fn dispose(&self, id: NodeId) {
        let parent = self.nodes.borrow().get(id).and_then(|n| n.owner);
        if let Some(parent) = parent
            && let Some(p) = self.nodes.borrow_mut().get_mut(parent)
        {
            p.owned.retain(|&c| c != id);
        }
        self.dispose_tree(id);
    }

    fn dispose_tree(&self, id: NodeId) {
        let owned = match self.nodes.borrow_mut().get_mut(id) {
            Some(n) => std::mem::take(&mut n.owned),
            None => return,
        };
        for child in owned.into_iter().rev() {
            self.dispose_tree(child);
        }
        self.run_cleanups(id);
        let removed = self.nodes.borrow_mut().remove(id);
        if let Some(node) = removed {
            let mut nodes = self.nodes.borrow_mut();
            for src in node.sources {
                if let Some(s) = nodes.get_mut(src) {
                    s.subscribers.retain(|&x| x != id);
                }
            }
            for sub in node.subscribers {
                if let Some(s) = nodes.get_mut(sub) {
                    s.sources.retain(|&x| x != id);
                }
            }
            drop(nodes);
            self.pending.borrow_mut().retain(|&x| x != id);
            // Dropping the node's value or closure runs app `Drop` code; no borrow is held.
            drop(node.kind);
        }
    }

    fn run_cleanups(&self, id: NodeId) {
        let cleanups = match self.nodes.borrow_mut().get_mut(id) {
            Some(n) => std::mem::take(&mut n.cleanups),
            None => return,
        };
        for f in cleanups.into_iter().rev() {
            f();
        }
    }

    /// Disposes what `id` owns and runs its clean-ups, keeping `id` itself. Used before a
    /// memo or effect runs again.
    fn reset_owned(&self, id: NodeId) {
        let owned = match self.nodes.borrow_mut().get_mut(id) {
            Some(n) => std::mem::take(&mut n.owned),
            None => return,
        };
        for child in owned.into_iter().rev() {
            self.dispose_tree(child);
        }
        self.run_cleanups(id);
    }

    pub(crate) fn on_cleanup(&self, f: Box<dyn FnOnce()>) -> bool {
        let Some(owner) = self.owner.get() else { return false };
        match self.nodes.borrow_mut().get_mut(owner) {
            Some(n) => {
                n.cleanups.push(f);
                true
            }
            None => false,
        }
    }

    /// Runs `f` with `owner` as the owner of anything it creates, and no observer.
    pub(crate) fn with_owner<R>(&self, owner: Option<NodeId>, f: impl FnOnce() -> R) -> R {
        let _o = swap(&self.owner, owner);
        let _w = swap(&self.observer, None);
        f()
    }

    // ---------- reading ----------

    /// Records that the current observer read `id`.
    fn track(&self, id: NodeId) {
        let Some(observer) = self.observer.get() else { return };
        if observer == id {
            return;
        }
        let mut nodes = self.nodes.borrow_mut();
        let already = nodes.get(observer).is_some_and(|o| o.sources.contains(&id));
        if already {
            return;
        }
        if let Some(o) = nodes.get_mut(observer) {
            o.sources.push(id);
        }
        if let Some(s) = nodes.get_mut(id) {
            s.subscribers.push(observer);
        }
    }

    pub(crate) fn untracked<R>(&self, f: impl FnOnce() -> R) -> R {
        let _w = swap(&self.observer, None);
        f()
    }

    pub(crate) fn signal_cell(&self, id: NodeId, track: bool) -> Result<Rc<RefCell<AnyBox>>, ReactiveError> {
        let cell = match self.nodes.borrow().get(id).map(|n| &n.kind) {
            Some(Kind::Signal(cell)) => cell.clone(),
            Some(_) => return Err(ReactiveError::WrongKind),
            None => return Err(ReactiveError::Disposed),
        };
        if track {
            self.track(id);
        }
        Ok(cell)
    }

    pub(crate) fn memo_cell(
        &self,
        id: NodeId,
        track: bool,
    ) -> Result<Rc<RefCell<Option<AnyBox>>>, ReactiveError> {
        self.update_if_necessary(id)?;
        let cell = match self.nodes.borrow().get(id).map(|n| &n.kind) {
            Some(Kind::Memo { value, .. }) => value.clone(),
            Some(_) => return Err(ReactiveError::WrongKind),
            None => return Err(ReactiveError::Disposed),
        };
        if track {
            self.track(id);
        }
        Ok(cell)
    }

    // ---------- writing ----------

    /// Marks the subscribers of a changed signal and runs effects unless a batch is open.
    pub(crate) fn notify(&self, id: NodeId) {
        let subs = match self.nodes.borrow().get(id) {
            Some(n) => n.subscribers.clone(),
            None => return,
        };
        for sub in subs {
            self.mark(sub, State::Dirty);
        }
        self.flush_if_idle();
    }

    fn mark(&self, start: NodeId, state: State) {
        let mut stack = vec![(start, state)];
        while let Some((id, state)) = stack.pop() {
            let mut nodes = self.nodes.borrow_mut();
            let Some(node) = nodes.get_mut(id) else { continue };
            if node.state >= state {
                continue;
            }
            let was_clean = node.state == State::Clean;
            node.state = state;
            if matches!(node.kind, Kind::Effect(_)) && !node.queued {
                node.queued = true;
                self.pending.borrow_mut().push_back(id);
            }
            if was_clean {
                stack.extend(node.subscribers.iter().map(|&s| (s, State::Check)));
            }
        }
    }

    // ---------- updating ----------

    fn update_if_necessary(&self, id: NodeId) -> Result<(), ReactiveError> {
        let (state, sources, running) = match self.nodes.borrow().get(id) {
            Some(n) => (n.state, n.sources.clone(), n.running),
            None => return Err(ReactiveError::Disposed),
        };
        if running {
            return Err(ReactiveError::Cycle);
        }
        if state == State::Check {
            for src in sources {
                let is_memo =
                    matches!(self.nodes.borrow().get(src).map(|n| &n.kind), Some(Kind::Memo { .. }));
                if is_memo {
                    // A source that was disposed simply no longer feeds this node.
                    match self.update_if_necessary(src) {
                        Ok(()) | Err(ReactiveError::Disposed) => {}
                        Err(e) => return Err(e),
                    }
                }
                if self.nodes.borrow().get(id).is_some_and(|n| n.state == State::Dirty) {
                    break;
                }
            }
        }
        // The node is marked clean *before* it runs, so a write it makes to something it
        // reads marks it dirty again and queues another run instead of being lost.
        let dirty = match self.nodes.borrow_mut().get_mut(id) {
            Some(n) => std::mem::replace(&mut n.state, State::Clean) == State::Dirty,
            None => return Err(ReactiveError::Disposed),
        };
        if dirty {
            self.run(id)?;
        }
        Ok(())
    }

    /// Runs a memo's computation or an effect's closure with fresh dependencies.
    fn run(&self, id: NodeId) -> Result<(), ReactiveError> {
        self.reset_owned(id);
        let old_sources = match self.nodes.borrow_mut().get_mut(id) {
            Some(n) => {
                n.running = true;
                std::mem::take(&mut n.sources)
            }
            None => return Err(ReactiveError::Disposed),
        };
        {
            let mut nodes = self.nodes.borrow_mut();
            for src in old_sources {
                if let Some(s) = nodes.get_mut(src) {
                    s.subscribers.retain(|&x| x != id);
                }
            }
        }
        let kind = match self.nodes.borrow().get(id).map(|n| &n.kind) {
            Some(Kind::Memo { value, compute, equal }) => {
                Some(Run::Memo(value.clone(), compute.clone(), *equal))
            }
            Some(Kind::Effect(f)) => Some(Run::Effect(f.clone())),
            _ => None,
        };
        struct Running<'a>(&'a Runtime, NodeId);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                if let Some(n) = self.0.nodes.borrow_mut().get_mut(self.1) {
                    n.running = false;
                }
            }
        }
        let _running = Running(self, id);
        let _o = swap(&self.owner, Some(id));
        let _w = swap(&self.observer, Some(id));
        match kind {
            Some(Run::Memo(value, compute, equal)) => {
                self.memo_runs.set(self.memo_runs.get() + 1);
                let prev = value.borrow_mut().take();
                let next = compute(prev.as_deref());
                let changed = match &prev {
                    Some(p) => !equal(p.as_ref(), next.as_ref()),
                    None => true,
                };
                *value.borrow_mut() = Some(next);
                drop(prev);
                if changed {
                    let subs = self.nodes.borrow().get(id).map(|n| n.subscribers.clone()).unwrap_or_default();
                    for sub in subs {
                        self.mark(sub, State::Dirty);
                    }
                }
            }
            Some(Run::Effect(f)) => {
                self.effect_runs.set(self.effect_runs.get() + 1);
                // An effect that sets a signal it reads marks itself again; the flush loop
                // runs it once more after this run, never recursively.
                match f.try_borrow_mut() {
                    Ok(mut f) => f(),
                    Err(_) => return Err(ReactiveError::Cycle),
                }
            }
            None => {}
        }
        Ok(())
    }

    // ---------- batching and effects ----------

    pub(crate) fn batch<R>(&self, f: impl FnOnce() -> R) -> R {
        struct Depth<'a>(&'a Runtime);
        impl Drop for Depth<'_> {
            fn drop(&mut self) {
                self.0.batch_depth.set(self.0.batch_depth.get().saturating_sub(1));
            }
        }
        self.batch_depth.set(self.batch_depth.get() + 1);
        let r = {
            let _d = Depth(self);
            f()
        };
        self.flush_if_idle();
        r
    }

    pub(crate) fn flush_if_idle(&self) {
        if self.batch_depth.get() == 0 && !self.flushing.get() {
            self.flush();
        }
    }

    fn flush(&self) {
        let _f = swap(&self.flushing, true);
        let mut runs: std::collections::HashMap<NodeId, u32> = std::collections::HashMap::new();
        loop {
            let next = self.pending.borrow_mut().pop_front();
            let Some(id) = next else { break };
            if let Some(n) = self.nodes.borrow_mut().get_mut(id) {
                n.queued = false;
            }
            let count = runs.entry(id).or_insert(0);
            *count += 1;
            if *count > RUNAWAY_RUNS {
                // Leave it dirty; it runs again on the next change, and the count shows up
                // in `Stats` so the inspector can point at it.
                if *count == RUNAWAY_RUNS + 1 {
                    self.runaway_effects.set(self.runaway_effects.get() + 1);
                }
                continue;
            }
            // A disposed or cyclic effect is skipped; the rest of the queue still runs.
            let _ = self.update_if_necessary(id);
        }
    }

    pub(crate) fn schedule_new_effect(&self, id: NodeId) {
        if let Some(n) = self.nodes.borrow_mut().get_mut(id) {
            n.queued = true;
        }
        self.pending.borrow_mut().push_back(id);
        self.flush_if_idle();
    }

    pub(crate) fn stats(&self) -> crate::Stats {
        crate::Stats {
            nodes: self.nodes.borrow().len(),
            effect_runs: self.effect_runs.get(),
            memo_runs: self.memo_runs.get(),
            pending_effects: self.pending.borrow().len(),
            runaway_effects: self.runaway_effects.get(),
        }
    }
}

enum Run {
    Memo(Rc<RefCell<Option<AnyBox>>>, Compute, Equal),
    Effect(Rc<RefCell<Box<dyn FnMut()>>>),
}
