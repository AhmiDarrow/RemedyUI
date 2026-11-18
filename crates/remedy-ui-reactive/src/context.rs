//! Context: values a scope provides to everything it owns, and error sinks.
//!
//! A component provides a typed value with [`provide_context`]; anything built inside it (and
//! the effects and memos created there, when they run later) reads the nearest one with
//! [`use_context`]. Lookups walk the owner chain: an effect's owner, that owner's owner, and
//! so on. Nothing is global, so tests and previews supply their own theme, locale or
//! services by providing them. Context values are fixed where they are provided; a value that
//! changes is provided as a [`crate::Signal`] (a `Copy` handle), and readers that read the
//! signal are reactive as usual.
//!
//! ```
//! use remedy_ui_reactive::{Scope, provide_context, use_context};
//!
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! struct Density(f32);
//!
//! let ((), _scope) = Scope::new(|| {
//!     provide_context(Density(1.5));
//!     let ((), _inner) = Scope::new(|| {
//!         assert_eq!(use_context::<Density>(), Some(Density(1.5)));
//!     });
//! });
//! assert_eq!(use_context::<Density>(), None, "outside the scope nothing is provided");
//! ```

use std::any::{Any, TypeId};
use std::panic::Location;
use std::rc::Rc;

use crate::debug::NodeKey;
use crate::error::ReactiveError;
use crate::runtime::{NodeId, with_rt};
use crate::signal::{Scope, on_rt};

/// Who owns what is created now: a scope, or a memo or effect while it runs. A handle to it
/// reads and provides context and runs code under it later, for work that starts in a
/// handler or a callback but belongs to a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    rt: u32,
    id: NodeId,
}

impl Owner {
    /// The current owner, if code is running under one.
    pub fn current() -> Option<Owner> {
        with_rt(|rt| rt.owner().map(|id| Owner { rt: rt.id, id })).ok().flatten()
    }

    /// Runs `f` with this owner as the owner of anything it creates (and no observer).
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> Result<R, ReactiveError> {
        on_rt(self.rt, |rt| {
            if rt.exists(self.id) {
                Ok(rt.with_owner(Some(self.id), f))
            } else {
                Err(ReactiveError::Disposed)
            }
        })
    }

    /// The nearest `T` this owner or its owners provide.
    pub fn use_context<T: Clone + 'static>(&self) -> Option<T> {
        lookup(self.rt, Some(self.id), false)
    }

    /// The `T` this owner itself provides, ignoring its owners.
    pub fn own_context<T: Clone + 'static>(&self) -> Option<T> {
        lookup(self.rt, Some(self.id), true)
    }

    /// Provides `value` to everything this owner owns, replacing a `T` it provided before.
    /// False when the owner is gone.
    pub fn provide_context<T: 'static>(&self, value: T) -> bool {
        on_rt(self.rt, |rt| Ok(rt.provide(self.id, TypeId::of::<T>(), Rc::new(value)))).unwrap_or(false)
    }

    /// The owner of this owner.
    pub fn parent(&self) -> Option<Owner> {
        on_rt(self.rt, |rt| Ok(rt.owner_of(self.id))).ok().flatten().map(|id| Owner { rt: self.rt, id })
    }

    /// True while the owner exists.
    pub fn is_alive(&self) -> bool {
        on_rt(self.rt, |rt| Ok(rt.exists(self.id))).unwrap_or(false)
    }

    /// Its key in the reactive graph ([`crate::debug`]).
    pub fn key(&self) -> NodeKey {
        crate::runtime::key(self.id)
    }

    pub(crate) fn id(&self) -> NodeId {
        self.id
    }
}

impl From<Scope> for Owner {
    fn from(s: Scope) -> Self {
        Owner { rt: s.rt, id: s.id }
    }
}

impl Scope {
    /// Provides `value` to everything this scope owns ([`provide_context`]).
    pub fn provide_context<T: 'static>(&self, value: T) -> bool {
        Owner::from(*self).provide_context(value)
    }

    /// The nearest `T` this scope or its owners provide.
    pub fn use_context<T: Clone + 'static>(&self) -> Option<T> {
        Owner::from(*self).use_context()
    }

    /// The `T` this scope itself provides, ignoring its owners.
    pub fn own_context<T: Clone + 'static>(&self) -> Option<T> {
        Owner::from(*self).own_context()
    }
}

fn lookup<T: Clone + 'static>(rt_id: u32, start: Option<NodeId>, own: bool) -> Option<T> {
    let v = on_rt(rt_id, |rt| Ok(rt.context(start, TypeId::of::<T>(), own))).ok().flatten()?;
    v.downcast_ref::<T>().cloned()
}

/// Provides `value` to everything the current owner owns: the scope being built, or the
/// effect or memo running. Returns false (and provides nothing) outside any owner.
pub fn provide_context<T: 'static>(value: T) -> bool {
    Owner::current().is_some_and(|o| o.provide_context(value))
}

/// The nearest `T` provided by the current owner or its owners.
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    Owner::current()?.use_context()
}

/// Runs `f` with a reference to the nearest `T` provided (for values that are not cheap to
/// clone), or returns `None` when nothing provides one.
pub fn with_context<T: 'static, R>(f: impl FnOnce(&T) -> R) -> Option<R> {
    let o = Owner::current()?;
    let v = on_rt(o.rt, |rt| Ok(rt.context(Some(o.id), TypeId::of::<T>(), false))).ok().flatten()?;
    v.downcast_ref::<T>().map(f)
}

/// A panic caught in an effect: what it said and which effect it was.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Caught {
    /// The panic message.
    pub message: String,
    /// The effect, in the reactive graph ([`crate::debug`]).
    pub node: NodeKey,
    /// Where the effect was created.
    pub created_at: Option<&'static Location<'static>>,
}

/// Where effect panics go. Provided as context ([`provide_context`]) by an error boundary:
/// when an effect owned (directly or further down) by the provider panics, the sink gets it
/// and the flush carries on with the other effects.
#[derive(Clone)]
pub struct ErrorSink(Rc<dyn Fn(Caught)>);

impl ErrorSink {
    /// A sink calling `f` for each caught panic.
    pub fn new(f: impl Fn(Caught) + 'static) -> Self {
        ErrorSink(Rc::new(f))
    }

    /// Hands it a failure.
    pub fn report(&self, c: Caught) {
        (self.0)(c);
    }
}

impl std::fmt::Debug for ErrorSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErrorSink")
    }
}

/// Adds a handler for effect panics that no [`ErrorSink`] took: it returns true when it
/// took care of one (a window that knows the effect's element contains it there). Returns
/// an id for [`remove_panic_handler`]. With no handler taking it, the panic keeps unwinding.
pub fn add_panic_handler(f: impl Fn(&Caught) -> bool + 'static) -> u64 {
    with_rt(|rt| rt.add_panic_handler(Rc::new(f))).unwrap_or(0)
}

/// Removes a panic handler.
pub fn remove_panic_handler(id: u64) {
    let _ = with_rt(|rt| rt.remove_panic_handler(id));
}

/// The text of a panic payload.
pub fn panic_message(p: &(dyn Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic without a message".to_owned()
    }
}
