//! The signal graph for devtools: what depends on what, and why each piece of work re-ran.
//!
//! The graph's edges (what each memo, effect and reaction read; who reads each signal and
//! memo) are always there: the runtime needs them. [`graph`] and [`node`] read them out with
//! names ([`Signal::named`](crate::Signal::named)) and creation sites.
//!
//! Why something re-ran is recorded only while a [`Tracing`] guard is alive on the thread
//! ([`trace`], [`trace_with`]). Then each signal write remembers where it happened, marking
//! carries it to every node downstream along the path it took, and each run of a memo, effect
//! or reaction becomes an [`Update`] naming its [`Cause`]s: "the effect re-ran because
//! `message.edited` (set at chat.rs:88) changed, through the memo `visible`". Without a guard
//! the runtime checks one flag per write and per run, and records nothing.
//!
//! ```
//! use remedy_ui_reactive::{Effect, Signal, debug};
//! use std::{cell::RefCell, rc::Rc};
//!
//! let edited = Signal::new(false).named("message.edited");
//! let seen = Rc::new(RefCell::new(Vec::new()));
//! let log = seen.clone();
//! let _tracing = debug::trace_with(move |u| log.borrow_mut().push(u.clone()));
//! let _row = Effect::new(move || {
//!     let _ = edited.get();
//! });
//! edited.set(true);
//! let seen = seen.borrow();
//! assert!(seen[0].causes.is_empty(), "the first run has no cause");
//! assert_eq!(seen[1].causes[0].signal, edited.key());
//! assert_eq!(seen[1].causes[0].signal_name.as_deref(), Some("message.edited"));
//! ```

use std::borrow::Cow;
use std::panic::Location;
use std::rc::Rc;

use crate::runtime::with_rt;

/// A node of the reactive graph (signal, memo, effect, reaction or scope), stable for as long
/// as the node lives and never reused for another while the runtime runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey(pub u64);

impl std::fmt::Display for NodeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A [`Signal`](crate::Signal): state.
    Signal,
    /// A [`Memo`](crate::Memo): a derived value.
    Memo,
    /// An [`Effect`](crate::Effect): work that re-runs (element bindings are effects).
    Effect,
    /// A [`Reaction`](crate::Reaction): work scheduled when what it tracked changes
    /// (element painters are reactions).
    Reaction,
    /// A [`Scope`](crate::Scope): an owner.
    Scope,
}

impl NodeKind {
    /// The kind in lowercase words.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Signal => "signal",
            NodeKind::Memo => "memo",
            NodeKind::Effect => "effect",
            NodeKind::Reaction => "reaction",
            NodeKind::Scope => "scope",
        }
    }
}

/// One node of the graph and its edges, as [`graph`] and [`node`] read them.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    /// The node.
    pub key: NodeKey,
    /// What it is.
    pub kind: NodeKind,
    /// Its debug name, if it was given one.
    pub name: Option<Cow<'static, str>>,
    /// Where it was created.
    pub created_at: &'static Location<'static>,
    /// What it read the last time it ran or tracked (memos, effects, reactions).
    pub sources: Vec<NodeKey>,
    /// What read it (signals and memos): its direct dependents.
    pub subscribers: Vec<NodeKey>,
    /// The scope, memo or effect that owns it.
    pub owner: Option<NodeKey>,
}

impl NodeInfo {
    /// Its name, or its kind and creation site.
    pub fn label(&self) -> String {
        match &self.name {
            Some(n) => n.to_string(),
            None => {
                format!("{} at {}:{}", self.kind.as_str(), self.created_at.file(), self.created_at.line())
            }
        }
    }
}

/// Why a node re-ran: a signal written, and the memos the change passed through to reach it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cause {
    /// The signal that was written.
    pub signal: NodeKey,
    /// Its debug name, if it has one.
    pub signal_name: Option<Cow<'static, str>>,
    /// Where the signal was created.
    pub signal_created_at: Option<&'static Location<'static>>,
    /// Where it was written.
    pub written_at: Option<&'static Location<'static>>,
    /// The memos between the signal and the node, nearest the signal first.
    pub via: Vec<NodeKey>,
}

impl Cause {
    /// The signal's name, or where it was created.
    pub fn signal_label(&self) -> String {
        match (&self.signal_name, self.signal_created_at) {
            (Some(n), _) => n.to_string(),
            (None, Some(at)) => format!("signal at {}:{}", at.file(), at.line()),
            (None, None) => format!("signal {}", self.signal),
        }
    }
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` changed", self.signal_label())?;
        if let Some(at) = self.written_at {
            write!(f, " (set at {}:{})", at.file(), at.line())?;
        }
        if !self.via.is_empty() {
            let via: Vec<String> = self.via.iter().map(ToString::to_string).collect();
            write!(f, " through {}", via.join(" → "))?;
        }
        Ok(())
    }
}

/// One run of a memo, effect or reaction while tracing, with why it ran.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    /// Increases by one with each recorded run on this thread.
    pub seq: u64,
    /// The node that ran.
    pub node: NodeKey,
    /// What it is.
    pub kind: NodeKind,
    /// Its debug name, if it has one.
    pub name: Option<Cow<'static, str>>,
    /// Where it was created.
    pub created_at: &'static Location<'static>,
    /// The writes that made it run, each once. Empty for a first run, or for a run that
    /// started before tracing did.
    pub causes: Vec<Cause>,
}

impl Update {
    /// Why it ran, in a sentence fragment: "`message.edited` changed (set at …)", or "first
    /// run".
    pub fn because(&self) -> String {
        if self.causes.is_empty() {
            return "first run".to_owned();
        }
        self.causes.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
    }
}

pub(crate) type Hook = Rc<dyn Fn(&Update)>;

/// Keeps cause recording on for this thread's runtime while alive ([`trace`],
/// [`trace_with`]). Guards nest: recording stops when the last one is dropped.
#[derive(Debug)]
#[must_use = "recording stops when the guard is dropped"]
pub struct Tracing {
    id: u64,
}

impl Drop for Tracing {
    fn drop(&mut self) {
        let id = self.id;
        let _ = with_rt(|rt| rt.unwatch(id));
    }
}

/// Records why work re-runs for as long as the guard lives (see [`Update`]). [`current`]
/// reads the run in progress.
pub fn trace() -> Tracing {
    Tracing { id: with_rt(|rt| rt.watch(None)).unwrap_or(0) }
}

/// Like [`trace`], and also calls `f` at the start of each recorded run, before the run's
/// own code, outside any tracking (what `f` reads subscribes nothing).
pub fn trace_with(f: impl Fn(&Update) + 'static) -> Tracing {
    Tracing { id: with_rt(|rt| rt.watch(Some(Rc::new(f)))).unwrap_or(0) }
}

/// True while a [`Tracing`] guard is alive on this thread.
pub fn is_tracing() -> bool {
    with_rt(|rt| rt.tracing()).unwrap_or(false)
}

/// The innermost run in progress while tracing: inside a binding, what made it run. Layers
/// above use it to say why they marked work (an element repainting because of a write).
pub fn current() -> Option<Update> {
    with_rt(|rt| rt.current_update()).ok().flatten()
}

/// Every live node of this thread's runtime, in arena order.
pub fn graph() -> Vec<NodeInfo> {
    with_rt(|rt| rt.graph()).unwrap_or_default()
}

/// One node, while it lives.
pub fn node(key: NodeKey) -> Option<NodeInfo> {
    with_rt(|rt| rt.node_info(key)).ok().flatten()
}

/// Everything downstream of `key` (what reads it, what reads those, …), nearest first, each
/// once.
pub fn dependents(key: NodeKey) -> Vec<NodeKey> {
    with_rt(|rt| rt.dependents(key)).unwrap_or_default()
}

/// Gives the node a debug name, shown by devtools and in [`Cause`]s. Returns false when the
/// node is gone.
pub fn set_name(key: NodeKey, name: impl Into<Cow<'static, str>>) -> bool {
    let name = name.into();
    with_rt(|rt| rt.set_name(key, name)).unwrap_or(false)
}
