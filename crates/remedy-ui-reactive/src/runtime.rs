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
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, Location, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use remedy_ui_core::{Arena, Id};

use crate::debug::{Cause, Hook, NodeInfo, NodeKey, NodeKind, Update};
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
    Memo {
        value: Rc<RefCell<Option<AnyBox>>>,
        compute: Compute,
        equal: Equal,
    },
    Effect(Rc<RefCell<Box<dyn FnMut()>>>),
    /// Tracks what a closure read during [`Runtime::track_reaction`]; a change calls
    /// `on_change` (which does not re-run the closure).
    Reaction(Rc<dyn Fn()>),
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
    /// Where context lookups go on from this node, when not its owner: a scope that nothing
    /// owns but that was made inside another reads that one's context.
    context_parent: Option<NodeId>,
    owned: Vec<NodeId>,
    cleanups: Vec<Box<dyn FnOnce()>>,
    /// Values this node provides to what it owns ([`crate::provide_context`]), by type.
    contexts: Vec<(std::any::TypeId, Rc<dyn Any>)>,
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
    /// Debug names, for the nodes that were given one.
    names: RefCell<HashMap<NodeId, Cow<'static, str>>>,
    /// True while a `debug::Tracing` guard lives: causes are recorded.
    tracing: Cell<bool>,
    /// The live tracing guards, with their hooks.
    watchers: RefCell<Vec<(u64, Option<Hook>)>>,
    next_watcher: Cell<u64>,
    /// Why each marked node will re-run (only while tracing).
    causes: RefCell<HashMap<NodeId, Vec<Cause>>>,
    /// The recorded runs in progress, innermost last (only while tracing).
    running_updates: RefCell<Vec<Rc<Update>>>,
    update_seq: Cell<u64>,
    /// Told about effect panics no error sink took ([`crate::add_panic_handler`]).
    panic_handlers: RefCell<Vec<(u64, PanicHandler)>>,
}

/// Told about an effect's panic; true when it took care of it.
pub(crate) type PanicHandler = Rc<dyn Fn(&crate::Caught) -> bool>;

/// The public key of a node.
pub(crate) fn key(id: NodeId) -> NodeKey {
    NodeKey((u64::from(id.generation()) << 32) | u64::from(id.index()))
}

fn from_key(k: NodeKey) -> NodeId {
    let index = u32::try_from(k.0 & 0xFFFF_FFFF).unwrap_or(u32::MAX);
    let generation = u32::try_from(k.0 >> 32).unwrap_or(u32::MAX);
    NodeId::from_parts(index, generation)
}

fn kind_of(k: &Kind) -> NodeKind {
    match k {
        Kind::Signal(_) => NodeKind::Signal,
        Kind::Memo { .. } => NodeKind::Memo,
        Kind::Effect(_) => NodeKind::Effect,
        Kind::Reaction(_) => NodeKind::Reaction,
        Kind::Scope => NodeKind::Scope,
    }
}

/// One pending mark: the node, the state to raise it to, and (while tracing) why. A
/// `causes_only` mark carries a cause to nodes already marked, without touching state.
struct Mark {
    id: NodeId,
    state: State,
    cause: Option<Cause>,
    causes_only: bool,
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
            names: RefCell::new(HashMap::new()),
            tracing: Cell::new(false),
            watchers: RefCell::new(Vec::new()),
            next_watcher: Cell::new(1),
            causes: RefCell::new(HashMap::new()),
            running_updates: RefCell::new(Vec::new()),
            update_seq: Cell::new(0),
            panic_handlers: RefCell::new(Vec::new()),
        }
    }

    // ---------- creating and disposing ----------

    pub(crate) fn create(&self, kind: Kind, created_at: &'static Location<'static>) -> NodeId {
        self.create_under(kind, created_at, self.owner.get(), None)
    }

    /// Creates a node owned by `owner` (disposed with it), reading context through
    /// `context_parent` when given (instead of through its owner).
    pub(crate) fn create_under(
        &self,
        kind: Kind,
        created_at: &'static Location<'static>,
        owner: Option<NodeId>,
        context_parent: Option<NodeId>,
    ) -> NodeId {
        let state = match kind {
            Kind::Signal(_) | Kind::Scope | Kind::Reaction(_) => State::Clean,
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
            context_parent,
            owned: Vec::new(),
            cleanups: Vec::new(),
            contexts: Vec::new(),
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

    /// What `id` read last time it ran (or tracked), with where each was created.
    pub(crate) fn sources(&self, id: NodeId) -> Vec<crate::Source> {
        let nodes = self.nodes.borrow();
        let Some(n) = nodes.get(id) else { return Vec::new() };
        n.sources
            .iter()
            .filter_map(|s| {
                let src = nodes.get(*s)?;
                let kind = match src.kind {
                    Kind::Signal(_) => crate::SourceKind::Signal,
                    Kind::Memo { .. } => crate::SourceKind::Memo,
                    _ => return None,
                };
                Some(crate::Source { kind, created_at: src.created_at, key: key(*s) })
            })
            .collect()
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

    /// Disposes `id` and all it owns, owned nodes first (the last-made first), each after
    /// all it owns. An explicit stack, not recursion: ownership any number of levels deep is
    /// disposed on any thread's stack.
    fn dispose_tree(&self, id: NodeId) {
        // Each node, and whether what it owns is already on the stack above it.
        let mut stack = vec![(id, false)];
        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                self.dispose_one(id);
                continue;
            }
            let owned = match self.nodes.borrow_mut().get_mut(id) {
                Some(n) => std::mem::take(&mut n.owned),
                None => continue,
            };
            stack.push((id, true));
            stack.extend(owned.into_iter().map(|child| (child, false)));
        }
    }

    /// Runs `id`'s clean-ups and removes it, unlinking it from what it read and what read it.
    fn dispose_one(&self, id: NodeId) {
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
            {
                let mut names = self.names.borrow_mut();
                if !names.is_empty() {
                    names.remove(&id);
                }
            }
            if self.tracing.get() {
                self.causes.borrow_mut().remove(&id);
            }
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
            Some(n) => {
                // What it provided is provided again by the run that follows.
                n.contexts.clear();
                std::mem::take(&mut n.owned)
            }
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

    /// The current owner, if any.
    pub(crate) fn owner(&self) -> Option<NodeId> {
        self.owner.get()
    }

    /// The owner of `id`.
    pub(crate) fn owner_of(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.borrow().get(id).and_then(|n| n.owner)
    }

    /// Makes `id` provide `value` (replacing what it provided of the same type).
    pub(crate) fn provide(&self, id: NodeId, ty: std::any::TypeId, value: Rc<dyn Any>) -> bool {
        match self.nodes.borrow_mut().get_mut(id) {
            Some(n) => {
                match n.contexts.iter_mut().find(|(t, _)| *t == ty) {
                    Some(slot) => slot.1 = value,
                    None => n.contexts.push((ty, value)),
                }
                true
            }
            None => false,
        }
    }

    /// The nearest value of type `ty` provided by `start` or its owners (only `start` itself
    /// when `own` is set).
    pub(crate) fn context(
        &self,
        start: Option<NodeId>,
        ty: std::any::TypeId,
        own: bool,
    ) -> Option<Rc<dyn Any>> {
        let nodes = self.nodes.borrow();
        let mut cur = start;
        while let Some(id) = cur {
            let n = nodes.get(id)?;
            if let Some((_, v)) = n.contexts.iter().find(|(t, _)| *t == ty) {
                return Some(v.clone());
            }
            if own {
                return None;
            }
            cur = n.context_parent.or(n.owner);
        }
        None
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
    /// `at` is where the write happened, recorded while tracing.
    pub(crate) fn notify(&self, id: NodeId, at: Option<&'static Location<'static>>) {
        let subs = match self.nodes.borrow().get(id) {
            Some(n) => n.subscribers.clone(),
            None => return,
        };
        let cause = self.tracing.get().then(|| Cause {
            signal: key(id),
            signal_name: self.names.borrow().get(&id).cloned(),
            signal_created_at: self.created_at(id),
            written_at: at,
            via: Vec::new(),
        });
        for sub in subs {
            self.mark(sub, State::Dirty, cause.clone());
        }
        self.flush_if_idle();
    }

    /// Records `cause` for `id`; true when it is new (no cause from the same signal yet).
    fn add_cause(&self, id: NodeId, cause: &Cause) -> bool {
        let mut causes = self.causes.borrow_mut();
        let list = causes.entry(id).or_default();
        if list.iter().any(|c| c.signal == cause.signal) {
            return false;
        }
        list.push(cause.clone());
        true
    }

    fn mark(&self, start: NodeId, state: State, cause: Option<Cause>) {
        let mut stack = vec![Mark { id: start, state, cause, causes_only: false }];
        while let Some(Mark { id, state, cause, causes_only }) = stack.pop() {
            // Only while tracing: the cause reaches this node, and goes on downstream when it
            // is new here, even where the state is already raised.
            let fresh = cause.as_ref().is_some_and(|c| self.add_cause(id, c));
            let mut nodes = self.nodes.borrow_mut();
            let Some(node) = nodes.get_mut(id) else { continue };
            let raise = !causes_only && node.state < state;
            if !raise && !fresh {
                continue;
            }
            let was_clean = node.state == State::Clean;
            if raise {
                node.state = state;
                if matches!(node.kind, Kind::Effect(_) | Kind::Reaction(_)) && !node.queued {
                    node.queued = true;
                    self.pending.borrow_mut().push_back(id);
                }
            }
            let propagate = raise && was_clean;
            if propagate || fresh {
                let next = cause.map(|mut c| {
                    c.via.push(key(id));
                    c
                });
                stack.extend(node.subscribers.iter().map(|&s| Mark {
                    id: s,
                    state: State::Check,
                    cause: next.clone(),
                    causes_only: !propagate,
                }));
            }
        }
    }

    // ---------- updating ----------

    fn update_if_necessary(&self, id: NodeId) -> Result<(), ReactiveError> {
        // A memo's sources are brought up to date first, and running it reads more: a chain
        // of memos recurses once per link.
        crate::deep::deeper(|| self.update_here(id))
    }

    /// [`Runtime::update_if_necessary`], on this stack.
    fn update_here(&self, id: NodeId) -> Result<(), ReactiveError> {
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
        } else if self.tracing.get() {
            // Checked and found unchanged: what marked it did not make it run.
            self.causes.borrow_mut().remove(&id);
        }
        Ok(())
    }

    /// Runs `f` with reaction `id` as the observer, replacing what it tracked before.
    pub(crate) fn track_reaction<R>(&self, id: NodeId, f: impl FnOnce() -> R) -> R {
        // The borrow ends before `f` can run: a match would hold it through the `None` arm,
        // where `f` (a paint closure, say) reads signals.
        let taken = self.nodes.borrow_mut().get_mut(id).map(|n| {
            n.state = State::Clean;
            std::mem::take(&mut n.sources)
        });
        let Some(old) = taken else { return untracked(self, f) };
        if self.tracing.get() {
            self.causes.borrow_mut().remove(&id);
        }
        {
            let mut nodes = self.nodes.borrow_mut();
            for src in old {
                if let Some(s) = nodes.get_mut(src) {
                    s.subscribers.retain(|&x| x != id);
                }
            }
        }
        let _w = swap(&self.observer, Some(id));
        f()
    }

    /// Runs a memo's computation or an effect's closure with fresh dependencies, or tells a
    /// reaction that what it tracked changed.
    fn run(&self, id: NodeId) -> Result<(), ReactiveError> {
        let reaction = match self.nodes.borrow().get(id).map(|n| &n.kind) {
            Some(Kind::Reaction(f)) => Some(f.clone()),
            _ => None,
        };
        if let Some(on_change) = reaction {
            // Keeps its sources: it is told again on the next change until it re-tracks.
            let _w = swap(&self.observer, None);
            let _u = self.begin_update(id);
            on_change();
            return Ok(());
        }
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
        // Recorded (and hooks told) before the run's own tracking starts.
        let _u = self.begin_update(id);
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
                        // While tracing, the causes reached them already, when the write
                        // marked them for checking.
                        self.mark(sub, State::Dirty, None);
                    }
                }
            }
            Some(Run::Effect(f)) => {
                self.effect_runs.set(self.effect_runs.get() + 1);
                // An effect that sets a signal it reads marks itself again; the flush loop
                // runs it once more after this run, never recursively.
                let caught = match f.try_borrow_mut() {
                    Ok(mut f) => catch_unwind(AssertUnwindSafe(&mut *f)).err(),
                    Err(_) => return Err(ReactiveError::Cycle),
                };
                if let Some(payload) = caught {
                    self.contain(id, payload);
                }
            }
            None => {}
        }
        Ok(())
    }

    /// Hands an effect's panic to the nearest [`crate::ErrorSink`] its owners provide, then
    /// to the thread's panic handlers; with neither, the panic goes on unwinding.
    fn contain(&self, id: NodeId, payload: Box<dyn Any + Send>) {
        let caught = crate::Caught {
            message: crate::panic_message(payload.as_ref()),
            node: key(id),
            created_at: self.created_at(id),
        };
        let sink = self
            .context(Some(id), std::any::TypeId::of::<crate::ErrorSink>(), false)
            .and_then(|v| v.downcast_ref::<crate::ErrorSink>().cloned());
        if let Some(sink) = sink {
            sink.report(caught);
            return;
        }
        let handlers: Vec<PanicHandler> =
            self.panic_handlers.borrow().iter().map(|(_, h)| h.clone()).collect();
        if handlers.iter().any(|h| h(&caught)) {
            return;
        }
        std::panic::resume_unwind(payload);
    }

    pub(crate) fn add_panic_handler(&self, h: PanicHandler) -> u64 {
        let id = self.next_watcher.get();
        self.next_watcher.set(id + 1);
        self.panic_handlers.borrow_mut().push((id, h));
        id
    }

    pub(crate) fn remove_panic_handler(&self, id: u64) {
        self.panic_handlers.borrow_mut().retain(|(i, _)| *i != id);
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
        if self.pending.borrow().is_empty() {
            return;
        }
        let _f = swap(&self.flushing, true);
        let _span = remedy_ui_trace::phase!("binds").entered();
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

    // ---------- the signal graph, for devtools ----------

    pub(crate) fn tracing(&self) -> bool {
        self.tracing.get()
    }

    pub(crate) fn watch(&self, hook: Option<Hook>) -> u64 {
        let id = self.next_watcher.get();
        self.next_watcher.set(id + 1);
        self.watchers.borrow_mut().push((id, hook));
        self.tracing.set(true);
        id
    }

    pub(crate) fn unwatch(&self, id: u64) {
        let mut w = self.watchers.borrow_mut();
        w.retain(|(i, _)| *i != id);
        if w.is_empty() {
            self.tracing.set(false);
            self.causes.borrow_mut().clear();
        }
    }

    /// While tracing: records that `id` starts a run, with its causes, and tells the hooks.
    /// The record stays current until the returned guard drops.
    fn begin_update(&self, id: NodeId) -> Option<UpdateGuard<'_>> {
        if !self.tracing.get() {
            return None;
        }
        let causes = self.causes.borrow_mut().remove(&id).unwrap_or_default();
        let (kind, created_at) = {
            let nodes = self.nodes.borrow();
            let n = nodes.get(id)?;
            (kind_of(&n.kind), n.created_at)
        };
        let seq = self.update_seq.get() + 1;
        self.update_seq.set(seq);
        let update = Rc::new(Update {
            seq,
            node: key(id),
            kind,
            name: self.names.borrow().get(&id).cloned(),
            created_at,
            causes,
        });
        self.running_updates.borrow_mut().push(update.clone());
        let hooks: Vec<Hook> = self.watchers.borrow().iter().filter_map(|(_, h)| h.clone()).collect();
        if !hooks.is_empty() {
            // Hooks read what they like without subscribing the run to it.
            let _w = swap(&self.observer, None);
            for h in hooks {
                h(&update);
            }
        }
        Some(UpdateGuard(self))
    }

    pub(crate) fn current_update(&self) -> Option<Update> {
        self.running_updates.borrow().last().map(|u| (**u).clone())
    }

    fn info(&self, id: NodeId, n: &Node) -> NodeInfo {
        NodeInfo {
            key: key(id),
            kind: kind_of(&n.kind),
            name: self.names.borrow().get(&id).cloned(),
            created_at: n.created_at,
            sources: n.sources.iter().map(|s| key(*s)).collect(),
            subscribers: n.subscribers.iter().map(|s| key(*s)).collect(),
            owner: n.owner.map(key),
        }
    }

    pub(crate) fn graph(&self) -> Vec<NodeInfo> {
        let nodes = self.nodes.borrow();
        nodes.iter().map(|(id, n)| self.info(id, n)).collect()
    }

    pub(crate) fn node_info(&self, k: NodeKey) -> Option<NodeInfo> {
        let id = from_key(k);
        let nodes = self.nodes.borrow();
        nodes.get(id).map(|n| self.info(id, n))
    }

    pub(crate) fn dependents(&self, k: NodeKey) -> Vec<NodeKey> {
        let nodes = self.nodes.borrow();
        let start = from_key(k);
        let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut queue: VecDeque<NodeId> =
            nodes.get(start).map(|n| n.subscribers.clone()).unwrap_or_default().into();
        while let Some(id) = queue.pop_front() {
            if id == start || !seen.insert(id) {
                continue;
            }
            out.push(key(id));
            if let Some(n) = nodes.get(id) {
                queue.extend(n.subscribers.iter().copied());
            }
        }
        out
    }

    pub(crate) fn set_name(&self, k: NodeKey, name: Cow<'static, str>) -> bool {
        self.name_node(from_key(k), name)
    }

    pub(crate) fn name_node(&self, id: NodeId, name: Cow<'static, str>) -> bool {
        if !self.exists(id) {
            return false;
        }
        self.names.borrow_mut().insert(id, name);
        true
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

/// Ends a recorded run: the one outside it becomes current again.
struct UpdateGuard<'a>(&'a Runtime);

impl Drop for UpdateGuard<'_> {
    fn drop(&mut self) {
        self.0.running_updates.borrow_mut().pop();
    }
}

fn untracked<R>(rt: &Runtime, f: impl FnOnce() -> R) -> R {
    let _w = swap(&rt.observer, None);
    f()
}

enum Run {
    Memo(Rc<RefCell<Option<AnyBox>>>, Compute, Equal),
    Effect(Rc<RefCell<Box<dyn FnMut()>>>),
}
