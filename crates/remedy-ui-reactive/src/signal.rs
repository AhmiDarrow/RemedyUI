//! Signals, memos, effects and scopes: the public handles over the runtime.

use std::any::Any;
use std::borrow::Cow;
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;

use crate::debug::NodeKey;
use crate::error::ReactiveError;
use crate::runtime::{Kind, NodeId, Runtime, key, with_rt};

fn on_rt<R>(rt_id: u32, f: impl FnOnce(&Runtime) -> Result<R, ReactiveError>) -> Result<R, ReactiveError> {
    with_rt(|rt| if rt.id == rt_id { f(rt) } else { Err(ReactiveError::WrongThread) })?
}

fn current_rt_id() -> Result<u32, ReactiveError> {
    with_rt(|rt| rt.id)
}

fn creation_site(rt_id: u32, id: NodeId) -> Option<&'static Location<'static>> {
    on_rt(rt_id, |rt| Ok(rt.created_at(id))).ok().flatten()
}

fn name(rt_id: u32, id: NodeId, name: Cow<'static, str>) {
    let _ = on_rt(rt_id, |rt| Ok(rt.name_node(id, name)));
}

#[track_caller]
fn create(kind: Kind) -> (u32, NodeId) {
    let at = Location::caller();
    match with_rt(|rt| (rt.id, rt.create(kind, at))) {
        Ok(v) => v,
        Err(e) => e.raise(None),
    }
}

/// A piece of state. Reading it inside a memo or effect subscribes that memo or effect;
/// writing it updates exactly what read it.
///
/// A `Signal` is a `Copy` handle. It can be captured by closures sent to other threads (for
/// example through a [`crate::Waker`]), but its value can only be reached on the UI thread
/// that created it.
pub struct Signal<T> {
    rt: u32,
    id: NodeId,
    _t: PhantomData<fn() -> T>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Signal<T> {}

impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.rt == other.rt && self.id == other.id
    }
}
impl<T> Eq for Signal<T> {}

impl<T> std::fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signal({}:{:?})", self.rt, self.id)
    }
}

impl<T: 'static> Signal<T> {
    /// A new signal holding `value`, owned by the current scope.
    #[track_caller]
    pub fn new(value: T) -> Self {
        let cell = Rc::new(std::cell::RefCell::new(Box::new(value) as Box<dyn Any>));
        let (rt, id) = create(Kind::Signal(cell));
        Signal { rt, id, _t: PhantomData }
    }

    fn access<R>(&self, track: bool, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        let cell = on_rt(self.rt, |rt| rt.signal_cell(self.id, track))?;
        let guard = cell.try_borrow().map_err(|_| ReactiveError::Borrowed)?;
        let value = guard.downcast_ref::<T>().ok_or(ReactiveError::WrongKind)?;
        Ok(f(value))
    }

    /// Runs `f` with a reference to the value, subscribing the current observer.
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        self.access(true, f)
    }

    /// Like [`Signal::try_with`]. Panics if the signal was disposed or is used off its thread;
    /// that is a bug, and the nearest error boundary contains it.
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self.access(true, f) {
            Ok(r) => r,
            Err(e) => e.raise(creation_site(self.rt, self.id)),
        }
    }

    /// Runs `f` with the value without subscribing anything.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        self.access(false, f)
    }

    /// A clone of the value, subscribing the current observer.
    pub fn try_get(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.access(true, T::clone)
    }

    /// A clone of the value. Panics like [`Signal::with`].
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// A clone of the value without subscribing anything.
    pub fn get_untracked(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.access(false, T::clone)
    }

    /// The signal's key in the reactive graph ([`crate::debug`]).
    pub fn key(&self) -> NodeKey {
        key(self.id)
    }

    /// Gives the signal a debug name: devtools say "`message.edited` changed" instead of
    /// where it was created.
    pub fn named(self, n: impl Into<Cow<'static, str>>) -> Self {
        name(self.rt, self.id, n.into());
        self
    }

    /// Changes the value with `f` and notifies when `f` returns true. Effects run when the
    /// outermost batch ends.
    #[track_caller]
    pub fn try_update_if(&self, f: impl FnOnce(&mut T) -> bool) -> Result<(), ReactiveError> {
        let at = Location::caller();
        on_rt(self.rt, |rt| {
            let cell = rt.signal_cell(self.id, false)?;
            let changed = {
                let mut guard = cell.try_borrow_mut().map_err(|_| ReactiveError::Borrowed)?;
                let value = guard.downcast_mut::<T>().ok_or(ReactiveError::WrongKind)?;
                f(value)
            };
            if changed {
                rt.notify(self.id, Some(at));
            }
            Ok(())
        })
    }

    /// Changes the value with `f` and notifies.
    #[track_caller]
    pub fn try_update(&self, f: impl FnOnce(&mut T)) -> Result<(), ReactiveError> {
        self.try_update_if(|v| {
            f(v);
            true
        })
    }

    /// Changes the value with `f` and notifies. Panics like [`Signal::with`].
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        if let Err(e) = self.try_update(f) {
            e.raise(creation_site(self.rt, self.id));
        }
    }

    /// Replaces the value and notifies.
    #[track_caller]
    pub fn try_set(&self, value: T) -> Result<(), ReactiveError> {
        self.try_update(|v| *v = value)
    }

    /// Replaces the value and notifies. Panics like [`Signal::with`].
    #[track_caller]
    pub fn set(&self, value: T) {
        self.update(|v| *v = value);
    }

    /// Replaces the value and notifies only if it is different.
    #[track_caller]
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        let r = self.try_update_if(|v| {
            if *v == value {
                false
            } else {
                *v = value;
                true
            }
        });
        if let Err(e) = r {
            e.raise(creation_site(self.rt, self.id));
        }
    }

    /// Takes the value, leaving the default in its place, and notifies.
    #[track_caller]
    pub fn take(&self) -> T
    where
        T: Default,
    {
        let mut out = None;
        self.update(|v| out = Some(std::mem::take(v)));
        out.unwrap_or_default()
    }

    /// True while the signal exists.
    pub fn is_alive(&self) -> bool {
        on_rt(self.rt, |rt| Ok(rt.exists(self.id))).unwrap_or(false)
    }

    /// Disposes the signal now, instead of with its owner.
    pub fn dispose(self) {
        let _ = on_rt(self.rt, |rt| {
            rt.dispose(self.id);
            Ok(())
        });
    }
}

/// A derived value, computed from signals and other memos when read. It recomputes only when
/// something it read has really changed, and notifies its readers only when its own value
/// changes (by `PartialEq`).
pub struct Memo<T> {
    rt: u32,
    id: NodeId,
    _t: PhantomData<fn() -> T>,
}

impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Memo<T> {}

impl<T> std::fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Memo({}:{:?})", self.rt, self.id)
    }
}

fn equal<T: PartialEq + 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    matches!((a.downcast_ref::<T>(), b.downcast_ref::<T>()), (Some(a), Some(b)) if a == b)
}

impl<T: PartialEq + 'static> Memo<T> {
    /// A memo computing `f`. It runs the first time it is read, not now.
    #[track_caller]
    pub fn new(f: impl Fn() -> T + 'static) -> Self {
        Self::new_with_previous(move |_| f())
    }

    /// A memo whose computation also gets its previous value, for incremental work.
    #[track_caller]
    pub fn new_with_previous(f: impl Fn(Option<&T>) -> T + 'static) -> Self {
        let compute = Rc::new(move |prev: Option<&dyn Any>| {
            Box::new(f(prev.and_then(|p| p.downcast_ref::<T>()))) as Box<dyn Any>
        });
        let kind = Kind::Memo { value: Rc::new(std::cell::RefCell::new(None)), compute, equal: equal::<T> };
        let (rt, id) = create(kind);
        Memo { rt, id, _t: PhantomData }
    }
}

impl<T: 'static> Memo<T> {
    /// The memo's key in the reactive graph ([`crate::debug`]).
    pub fn key(&self) -> NodeKey {
        key(self.id)
    }

    /// Gives the memo a debug name, shown by devtools.
    pub fn named(self, n: impl Into<Cow<'static, str>>) -> Self {
        name(self.rt, self.id, n.into());
        self
    }

    fn access<R>(&self, track: bool, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        let cell = on_rt(self.rt, |rt| rt.memo_cell(self.id, track))?;
        let guard = cell.try_borrow().map_err(|_| ReactiveError::Cycle)?;
        let value = guard.as_ref().and_then(|v| v.downcast_ref::<T>()).ok_or(ReactiveError::WrongKind)?;
        Ok(f(value))
    }

    /// Runs `f` with the up-to-date value, subscribing the current observer.
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, ReactiveError> {
        self.access(true, f)
    }

    /// Like [`Memo::try_with`]; panics on misuse like [`Signal::with`].
    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self.access(true, f) {
            Ok(r) => r,
            Err(e) => e.raise(creation_site(self.rt, self.id)),
        }
    }

    /// A clone of the up-to-date value.
    pub fn try_get(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.access(true, T::clone)
    }

    /// A clone of the up-to-date value; panics on misuse like [`Signal::with`].
    #[track_caller]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// A clone of the up-to-date value without subscribing anything.
    pub fn get_untracked(&self) -> Result<T, ReactiveError>
    where
        T: Clone,
    {
        self.access(false, T::clone)
    }

    /// Disposes the memo now.
    pub fn dispose(self) {
        let _ = on_rt(self.rt, |rt| {
            rt.dispose(self.id);
            Ok(())
        });
    }
}

/// Work that runs when what it reads changes: it runs once when created (at the end of the
/// current batch) and again after each change to anything it read. Anything it creates is
/// owned by it and disposed before it runs again, together with its clean-ups.
#[derive(Clone, Copy, Debug)]
pub struct Effect {
    rt: u32,
    id: NodeId,
}

impl Effect {
    /// A new effect running `f`.
    #[track_caller]
    pub fn new(f: impl FnMut() + 'static) -> Self {
        let kind = Kind::Effect(Rc::new(std::cell::RefCell::new(Box::new(f))));
        let (rt, id) = create(kind);
        let _ = on_rt(rt, |r| {
            r.schedule_new_effect(id);
            Ok(())
        });
        Effect { rt, id }
    }

    /// The signals and memos it read the last time it ran.
    pub fn sources(&self) -> Vec<Source> {
        on_rt(self.rt, |rt| Ok(rt.sources(self.id))).unwrap_or_default()
    }

    /// The effect's key in the reactive graph ([`crate::debug`]).
    pub fn key(&self) -> NodeKey {
        key(self.id)
    }

    /// Gives the effect a debug name, shown by devtools.
    pub fn named(self, n: impl Into<Cow<'static, str>>) -> Self {
        name(self.rt, self.id, n.into());
        self
    }

    /// True while the effect exists.
    pub fn is_alive(&self) -> bool {
        on_rt(self.rt, |rt| Ok(rt.exists(self.id))).unwrap_or(false)
    }

    /// Stops the effect and runs its clean-ups.
    pub fn dispose(self) {
        let _ = on_rt(self.rt, |rt| {
            rt.dispose(self.id);
            Ok(())
        });
    }
}

/// Watches what a closure reads without re-running it: [`Reaction::track`] runs the closure
/// and records its reads, and a later change to any of them calls `on_change` (once per
/// change, until the next `track`). For work that can only run at a certain time, such as
/// painting during a frame: `on_change` schedules it, and the scheduled run tracks again.
#[derive(Clone, Copy, Debug)]
pub struct Reaction {
    rt: u32,
    id: NodeId,
}

impl Reaction {
    /// A reaction calling `on_change` when what it tracked changes.
    #[track_caller]
    pub fn new(on_change: impl Fn() + 'static) -> Self {
        let (rt, id) = create(Kind::Reaction(Rc::new(on_change)));
        Reaction { rt, id }
    }

    /// Runs `f`, tracking what it reads in place of what was tracked before. Fails (without
    /// running `f`) only when the thread's runtime is gone.
    pub fn track<R>(&self, f: impl FnOnce() -> R) -> Result<R, ReactiveError> {
        on_rt(self.rt, |rt| Ok(rt.track_reaction(self.id, f)))
    }

    /// The signals and memos it read the last time it tracked.
    pub fn sources(&self) -> Vec<Source> {
        on_rt(self.rt, |rt| Ok(rt.sources(self.id))).unwrap_or_default()
    }

    /// The reaction's key in the reactive graph ([`crate::debug`]).
    pub fn key(&self) -> NodeKey {
        key(self.id)
    }

    /// Gives the reaction a debug name, shown by devtools.
    pub fn named(self, n: impl Into<Cow<'static, str>>) -> Self {
        name(self.rt, self.id, n.into());
        self
    }

    /// True while the reaction exists.
    pub fn is_alive(&self) -> bool {
        on_rt(self.rt, |rt| Ok(rt.exists(self.id))).unwrap_or(false)
    }

    /// Stops watching.
    pub fn dispose(self) {
        let _ = on_rt(self.rt, |rt| {
            rt.dispose(self.id);
            Ok(())
        });
    }
}

/// What a source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A [`Signal`].
    Signal,
    /// A [`Memo`].
    Memo,
}

/// A signal or memo something read: for inspectors that show why work re-ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Source {
    /// Signal or memo.
    pub kind: SourceKind,
    /// Where it was created.
    pub created_at: &'static Location<'static>,
    /// Its key in the reactive graph ([`crate::debug`]).
    pub key: NodeKey,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let k = match self.kind {
            SourceKind::Signal => "signal",
            SourceKind::Memo => "memo",
        };
        write!(f, "{k} at {}:{}", self.created_at.file(), self.created_at.line())
    }
}

/// An owner for signals, memos, effects and clean-ups. Disposing a scope disposes all of
/// them. Components, windows and list rows are scopes.
#[derive(Clone, Copy, Debug)]
pub struct Scope {
    rt: u32,
    id: NodeId,
}

impl Scope {
    /// Creates a scope under the current owner and runs `f` inside it.
    #[track_caller]
    pub fn new<R>(f: impl FnOnce() -> R) -> (R, Scope) {
        let (rt, id) = create(Kind::Scope);
        let scope = Scope { rt, id };
        let r = match scope.run(f) {
            Ok(r) => r,
            Err(e) => e.raise(creation_site(rt, id)),
        };
        (r, scope)
    }

    /// Runs `f` with this scope as the owner of anything it creates.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> Result<R, ReactiveError> {
        on_rt(self.rt, |rt| {
            if rt.exists(self.id) {
                Ok(rt.with_owner(Some(self.id), f))
            } else {
                Err(ReactiveError::Disposed)
            }
        })
    }

    /// True while the scope exists.
    pub fn is_alive(&self) -> bool {
        on_rt(self.rt, |rt| Ok(rt.exists(self.id))).unwrap_or(false)
    }

    /// The scope's key in the reactive graph ([`crate::debug`]).
    pub fn key(&self) -> NodeKey {
        key(self.id)
    }

    /// Disposes everything the scope owns, runs clean-ups, and removes the scope.
    pub fn dispose(self) {
        let _ = on_rt(self.rt, |rt| {
            rt.dispose(self.id);
            Ok(())
        });
    }
}

/// Registers `f` to run when the current owner (scope, memo or effect) is disposed or runs
/// again. Returns false when there is no current owner, in which case `f` is dropped unrun.
pub fn on_cleanup(f: impl FnOnce() + 'static) -> bool {
    with_rt(|rt| rt.on_cleanup(Box::new(f))).unwrap_or(false)
}

/// Runs `f` so that its signal writes notify once, when `f` returns. Batches nest; effects
/// run when the outermost one ends.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    // If this thread's runtime is gone, `f` still runs, just without batching.
    let _ = with_rt(|rt| rt.batch(|| out = f.take().map(|f| f())));
    finish(out, f)
}

/// Runs `f` without subscribing the current observer to anything `f` reads.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    let _ = with_rt(|rt| rt.untracked(|| out = f.take().map(|f| f())));
    finish(out, f)
}

fn finish<R>(out: Option<R>, f: Option<impl FnOnce() -> R>) -> R {
    match (out, f) {
        (Some(r), _) => r,
        (None, Some(f)) => f(),
        (None, None) => closure_lost(),
    }
}

/// The closure is taken exactly once and either ran (`out` is set) or did not (`f` is
/// still there), so this cannot be reached.
#[allow(clippy::panic, reason = "unreachable by construction; see the doc comment")]
fn closure_lost() -> ! {
    panic!("reactive closure neither ran nor remained")
}

/// Counts of the runtime's work on this thread, for tests and the performance overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stats {
    /// Live nodes (signals, memos, effects, scopes).
    pub nodes: usize,
    /// Effect runs since the runtime started.
    pub effect_runs: u64,
    /// Memo computations since the runtime started.
    pub memo_runs: u64,
    /// Effects queued and not yet run.
    pub pending_effects: usize,
    /// Effects stopped for a flush because they kept re-triggering themselves without
    /// settling. Non-zero means an app bug worth showing in the inspector.
    pub runaway_effects: u64,
}

/// This thread's runtime counters.
pub fn stats() -> Stats {
    with_rt(Runtime::stats).unwrap_or_default()
}

/// The id of this thread's runtime, unique in the process.
pub fn runtime_id() -> Option<u32> {
    current_rt_id().ok()
}
