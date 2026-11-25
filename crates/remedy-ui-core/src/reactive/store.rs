//! Stores: reactivity inside nested data.
//!
//! One signal for a whole chat room would wake everything on any change; one signal per field
//! is too much code. A [`Store`] holds a whole value and tracks reads and writes per *path*:
//! writing `room.messages().at(3).edited()` wakes only what read that field, what read
//! something that contains it (the message, the list, the room), or something inside it.
//! Siblings are untouched.
//!
//! Field accessors come from `#[derive(Store)]` (in `remedy-ui-macros`), or are written by
//! hand with [`Field::project`].

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::reactive::error::ReactiveError;
use crate::reactive::signal::{Scope, Signal, batch};

/// One step of a path: a struct field, or a position in a vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Segment {
    /// A struct field, by its declaration index.
    Field(u32),
    /// A vector element, by position.
    Index(usize),
}

type Path = Vec<Segment>;

/// Why a store access failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// The store's signal could not be reached.
    Reactive(ReactiveError),
    /// The path does not lead to a value right now (a vector index past the end).
    Missing,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Reactive(e) => write!(f, "{e}"),
            StoreError::Missing => {
                f.write_str("the store path does not lead to a value (index past the end)")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<ReactiveError> for StoreError {
    fn from(e: ReactiveError) -> Self {
        StoreError::Reactive(e)
    }
}

struct Inner<T> {
    value: T,
    triggers: HashMap<Path, Signal<u64>>,
}

/// Reactive nested data with field-level tracking. A `Copy` handle.
pub struct Store<T> {
    inner: Signal<Inner<T>>,
    scope: Scope,
}

impl<T> Clone for Store<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Store<T> {}

impl<T> fmt::Debug for Store<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Store({:?})", self.inner)
    }
}

type Getter<R, V> = Rc<dyn Fn(&R) -> Option<&V>>;
type GetterMut<R, V> = Rc<dyn Fn(&mut R) -> Option<&mut V>>;

/// Pins a closure to the higher-ranked shape a getter needs (the borrow it returns lives as
/// long as the value it was given), which closure inference does not produce by itself.
fn getter<R, V, F: for<'a> Fn(&'a R) -> Option<&'a V>>(f: F) -> F {
    f
}

fn getter_mut<R, V, F: for<'a> Fn(&'a mut R) -> Option<&'a mut V>>(f: F) -> F {
    f
}

/// A path into a [`Store`] leading to a value of type `V`.
pub struct Field<R, V> {
    store: Store<R>,
    path: Path,
    get: Getter<R, V>,
    get_mut: GetterMut<R, V>,
}

impl<R, V> Clone for Field<R, V> {
    fn clone(&self) -> Self {
        Field {
            store: self.store,
            path: self.path.clone(),
            get: self.get.clone(),
            get_mut: self.get_mut.clone(),
        }
    }
}

impl<R, V> fmt::Debug for Field<R, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Field({:?})", self.path)
    }
}

impl<T: 'static> Store<T> {
    /// A store holding `value`, owned by the current scope.
    #[track_caller]
    pub fn new(value: T) -> Self {
        let ((), scope) = Scope::new(|| {});
        let inner = match scope.run(|| Signal::new(Inner { value, triggers: HashMap::new() })) {
            Ok(s) => s,
            Err(e) => e.raise(None),
        };
        Store { inner, scope }
    }

    /// The whole value as a field, the start of every path.
    pub fn root(&self) -> Field<T, T> {
        Field {
            store: *self,
            path: Vec::new(),
            get: Rc::new(getter(|r: &T| Some(r))),
            get_mut: Rc::new(getter_mut(|r: &mut T| Some(r))),
        }
    }

    /// Disposes the store and everything tracking it.
    pub fn dispose(self) {
        self.scope.dispose();
    }

    /// Subscribes the current observer to `path`, making its trigger on first use.
    fn track(&self, path: &[Segment]) -> Result<(), StoreError> {
        let existing = self.inner.with_untracked(|i| i.triggers.get(path).copied())?;
        let trigger = match existing {
            Some(t) => t,
            None => {
                // Triggers belong to the store, not to whoever happened to read first.
                let t = self.scope.run(|| Signal::new(0u64))?;
                self.inner.try_update_if(|i| {
                    i.triggers.insert(path.to_vec(), t);
                    false
                })?;
                t
            }
        };
        trigger.try_with(|_| ())?;
        Ok(())
    }

    /// Wakes readers of `path`, of anything containing it, and of anything inside it.
    fn changed(&self, path: &[Segment]) -> Result<(), StoreError> {
        let hit: Vec<Signal<u64>> = self.inner.with_untracked(|i| {
            i.triggers
                .iter()
                .filter(|(p, _)| p.starts_with(path) || path.starts_with(p))
                .map(|(_, t)| *t)
                .collect()
        })?;
        batch(|| {
            for t in hit {
                let _ = t.try_update(|v| *v = v.wrapping_add(1));
            }
        });
        Ok(())
    }
}

impl<R: 'static, V: 'static> Field<R, V> {
    /// The path from the store's root.
    pub fn path(&self) -> &[Segment] {
        &self.path
    }

    /// Runs `f` with the value, subscribing the current observer to this path.
    pub fn with<O>(&self, f: impl FnOnce(&V) -> O) -> Result<O, StoreError> {
        self.store.track(&self.path)?;
        self.store.inner.with_untracked(|i| (self.get)(&i.value).map(f))?.ok_or(StoreError::Missing)
    }

    /// A clone of the value, subscribing the current observer to this path.
    pub fn get(&self) -> Result<V, StoreError>
    where
        V: Clone,
    {
        self.with(V::clone)
    }

    /// Changes the value and wakes the readers of this path, its ancestors and its
    /// descendants.
    pub fn update(&self, f: impl FnOnce(&mut V)) -> Result<(), StoreError> {
        let mut found = false;
        self.store.inner.try_update_if(|i| {
            if let Some(v) = (self.get_mut)(&mut i.value) {
                f(v);
                found = true;
            }
            false
        })?;
        if !found {
            return Err(StoreError::Missing);
        }
        self.store.changed(&self.path)
    }

    /// Replaces the value.
    pub fn set(&self, value: V) -> Result<(), StoreError> {
        self.update(|v| *v = value)
    }

    /// Replaces the value only if it differs, waking readers only then.
    pub fn set_if_changed(&self, value: V) -> Result<(), StoreError>
    where
        V: PartialEq,
    {
        let same = self.store.inner.with_untracked(|i| (self.get)(&i.value).map(|v| *v == value))?;
        match same {
            Some(true) => Ok(()),
            Some(false) => self.set(value),
            None => Err(StoreError::Missing),
        }
    }

    /// The path one step further, through a struct field. `#[derive(Store)]` generates calls
    /// to this; by hand: `msg.project(2, |m| &m.edited, |m| &mut m.edited)`.
    pub fn project<W: 'static>(
        &self,
        field: u32,
        get: fn(&V) -> &W,
        get_mut: fn(&mut V) -> &mut W,
    ) -> Field<R, W> {
        let (outer, outer_mut) = (self.get.clone(), self.get_mut.clone());
        let mut path = self.path.clone();
        path.push(Segment::Field(field));
        Field {
            store: self.store,
            path,
            get: Rc::new(getter(move |r: &R| outer(r).map(get))),
            get_mut: Rc::new(getter_mut(move |r: &mut R| outer_mut(r).map(get_mut))),
        }
    }
}

impl<R: 'static, T: 'static> Field<R, Vec<T>> {
    /// The element at `index`. Reading or writing it fails with [`StoreError::Missing`] while
    /// the index is past the end.
    pub fn at(&self, index: usize) -> Field<R, T> {
        let (outer, outer_mut) = (self.get.clone(), self.get_mut.clone());
        let mut path = self.path.clone();
        path.push(Segment::Index(index));
        Field {
            store: self.store,
            path,
            get: Rc::new(getter(move |r: &R| outer(r).and_then(|v| v.get(index)))),
            get_mut: Rc::new(getter_mut(move |r: &mut R| outer_mut(r).and_then(|v| v.get_mut(index)))),
        }
    }

    /// The number of elements, subscribing the current observer to the vector.
    pub fn len(&self) -> Result<usize, StoreError> {
        self.with(Vec::len)
    }

    /// True when the vector is empty, subscribing the current observer.
    pub fn is_empty(&self) -> Result<bool, StoreError> {
        self.with(Vec::is_empty)
    }
}
