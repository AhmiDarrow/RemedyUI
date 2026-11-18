//! Reactive vectors that report changes as operations.
//!
//! Diffing a 100,000-item list on every insert is how frameworks get slow. A [`SignalVec`]
//! records each change as a [`VecDiff`] instead, and each [`VecReader`] receives exactly the
//! operations it has not seen. A keyed list view applies them directly: a new chat message
//! is one `Push`, so one row mounts and nothing is compared.

use std::fmt;

use crate::error::ReactiveError;
use crate::signal::Signal;

/// One change to a [`SignalVec`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum VecDiff<T> {
    /// The whole contents, for a reader's first read or after `replace`.
    Replace(Vec<T>),
    /// `value` was inserted at `index`.
    Insert {
        /// Position of the new item.
        index: usize,
        /// The new item.
        value: T,
    },
    /// The item at `index` was removed.
    Remove {
        /// Position of the removed item.
        index: usize,
    },
    /// The item at `index` was replaced by `value`.
    Set {
        /// Position of the changed item.
        index: usize,
        /// The new item.
        value: T,
    },
    /// The item at `from` moved to `to` (indices as in `Vec::remove` then `Vec::insert`).
    Move {
        /// Old position.
        from: usize,
        /// New position.
        to: usize,
    },
    /// `value` was added at the end.
    Push(T),
    /// The last item was removed.
    Pop,
    /// Every item was removed.
    Clear,
}

/// An index was past the end of the vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfRange {
    /// The index asked for.
    pub index: usize,
    /// The vector's length at the time.
    pub len: usize,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "index {} is out of range for a list of {}", self.index, self.len)
    }
}

impl std::error::Error for OutOfRange {}

struct Reader {
    id: u64,
    /// The last version this reader has seen; `None` until its first read.
    seen: Option<u64>,
}

struct State<T> {
    items: Vec<T>,
    /// Operations with versions `log_base + 1 ..= version`, kept only while a reader has not
    /// seen them.
    log: std::collections::VecDeque<VecDiff<T>>,
    log_base: u64,
    version: u64,
    readers: Vec<Reader>,
    next_reader: u64,
}

impl<T: Clone> State<T> {
    fn record(&mut self, diff: VecDiff<T>) {
        self.version += 1;
        if self.readers.iter().any(|r| r.seen.is_some()) {
            self.log.push_back(diff);
        } else {
            // Nobody has read yet: their first read is a full `Replace`, so no log is needed.
            self.log.clear();
            self.log_base = self.version;
        }
    }

    fn trim(&mut self) {
        let oldest = self.readers.iter().filter_map(|r| r.seen).min();
        let keep_from = oldest.unwrap_or(self.version);
        while self.log_base < keep_from && self.log.pop_front().is_some() {
            self.log_base += 1;
        }
        if self.log.is_empty() {
            self.log_base = self.version;
        }
    }
}

/// A reactive vector. Reading it through a [`VecReader`] inside an effect subscribes the
/// effect; each change wakes the effect, which receives only the operations it has not seen.
pub struct SignalVec<T> {
    state: Signal<State<T>>,
}

impl<T> Clone for SignalVec<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for SignalVec<T> {}

impl<T> fmt::Debug for SignalVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SignalVec({:?})", self.state)
    }
}

impl<T: Clone + 'static> SignalVec<T> {
    /// A new reactive vector holding `items`.
    #[track_caller]
    pub fn new(items: Vec<T>) -> Self {
        SignalVec {
            state: Signal::new(State {
                items,
                log: std::collections::VecDeque::new(),
                log_base: 0,
                version: 0,
                readers: Vec::new(),
                next_reader: 0,
            }),
        }
    }

    fn change<R>(&self, f: impl FnOnce(&mut State<T>) -> R) -> Result<R, ReactiveError> {
        let mut out = None;
        self.state.try_update(|s| out = Some(f(s)))?;
        out.ok_or(ReactiveError::WrongKind)
    }

    /// Adds `value` at the end.
    pub fn push(&self, value: T) -> Result<(), ReactiveError> {
        self.change(|s| {
            s.items.push(value.clone());
            s.record(VecDiff::Push(value));
        })
    }

    /// Removes and returns the last item.
    pub fn pop(&self) -> Result<Option<T>, ReactiveError> {
        let mut out = None;
        self.state.try_update_if(|s| {
            out = s.items.pop();
            if out.is_some() {
                s.record(VecDiff::Pop);
            }
            out.is_some()
        })?;
        Ok(out)
    }

    /// Inserts `value` at `index` (which may equal the length).
    pub fn insert(&self, index: usize, value: T) -> Result<Result<(), OutOfRange>, ReactiveError> {
        let mut out = Ok(());
        self.state.try_update_if(|s| {
            if index > s.items.len() {
                out = Err(OutOfRange { index, len: s.items.len() });
                return false;
            }
            s.items.insert(index, value.clone());
            s.record(VecDiff::Insert { index, value });
            true
        })?;
        Ok(out)
    }

    /// Removes and returns the item at `index`.
    pub fn remove(&self, index: usize) -> Result<Result<T, OutOfRange>, ReactiveError> {
        let mut out = Err(OutOfRange { index, len: 0 });
        self.state.try_update_if(|s| {
            if index >= s.items.len() {
                out = Err(OutOfRange { index, len: s.items.len() });
                return false;
            }
            out = Ok(s.items.remove(index));
            s.record(VecDiff::Remove { index });
            true
        })?;
        Ok(out)
    }

    /// Replaces the item at `index`.
    pub fn set(&self, index: usize, value: T) -> Result<Result<(), OutOfRange>, ReactiveError> {
        let mut out = Ok(());
        self.state.try_update_if(|s| match s.items.get_mut(index) {
            Some(slot) => {
                *slot = value.clone();
                s.record(VecDiff::Set { index, value });
                true
            }
            None => {
                out = Err(OutOfRange { index, len: s.items.len() });
                false
            }
        })?;
        Ok(out)
    }

    /// Moves the item at `from` to `to`.
    pub fn move_item(&self, from: usize, to: usize) -> Result<Result<(), OutOfRange>, ReactiveError> {
        let mut out = Ok(());
        self.state.try_update_if(|s| {
            let len = s.items.len();
            if from >= len || to >= len {
                out = Err(OutOfRange { index: from.max(to), len });
                return false;
            }
            if from == to {
                return false;
            }
            let item = s.items.remove(from);
            s.items.insert(to, item);
            s.record(VecDiff::Move { from, to });
            true
        })?;
        Ok(out)
    }

    /// Removes every item.
    pub fn clear(&self) -> Result<(), ReactiveError> {
        self.state.try_update_if(|s| {
            if s.items.is_empty() {
                return false;
            }
            s.items.clear();
            s.record(VecDiff::Clear);
            true
        })
    }

    /// Replaces the whole contents.
    pub fn replace(&self, items: Vec<T>) -> Result<(), ReactiveError> {
        self.change(|s| {
            s.items = items.clone();
            s.record(VecDiff::Replace(items));
        })
    }

    /// The number of items, subscribing the current observer.
    pub fn len(&self) -> Result<usize, ReactiveError> {
        self.state.try_with(|s| s.items.len())
    }

    /// True when there are no items, subscribing the current observer.
    pub fn is_empty(&self) -> Result<bool, ReactiveError> {
        self.state.try_with(|s| s.items.is_empty())
    }

    /// Runs `f` with the items, subscribing the current observer.
    pub fn with<R>(&self, f: impl FnOnce(&[T]) -> R) -> Result<R, ReactiveError> {
        self.state.try_with(|s| f(&s.items))
    }

    /// A clone of the item at `index`, subscribing the current observer.
    pub fn get(&self, index: usize) -> Result<Option<T>, ReactiveError> {
        self.state.try_with(|s| s.items.get(index).cloned())
    }

    /// A new reader. Its first [`VecReader::take_diffs`] returns the whole contents as one
    /// `Replace`; later calls return only what changed since.
    pub fn reader(&self) -> Result<VecReader<T>, ReactiveError> {
        let mut id = 0;
        self.state.try_update_if(|s| {
            id = s.next_reader;
            s.next_reader += 1;
            s.readers.push(Reader { id, seen: None });
            false
        })?;
        Ok(VecReader { vec: *self, id })
    }
}

/// Receives a [`SignalVec`]'s changes as operations. Drop it when it is no longer read, so
/// the vector stops keeping operations for it.
pub struct VecReader<T: Clone + 'static> {
    vec: SignalVec<T>,
    id: u64,
}

impl<T: Clone + 'static> fmt::Debug for VecReader<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VecReader({})", self.id)
    }
}

impl<T: Clone + 'static> VecReader<T> {
    /// The operations this reader has not seen yet, oldest first, subscribing the current
    /// observer to further changes.
    pub fn take_diffs(&self) -> Result<Vec<VecDiff<T>>, ReactiveError> {
        // Subscribe first, then consume without notifying anyone.
        self.vec.state.try_with(|_| ())?;
        let mut out = Vec::new();
        self.vec.state.try_update_if(|s| {
            let version = s.version;
            let log_base = s.log_base;
            let Some(reader) = s.readers.iter_mut().find(|r| r.id == self.id) else { return false };
            match reader.seen {
                Some(seen) if seen >= log_base => {
                    let skip = usize::try_from(seen - log_base).unwrap_or(usize::MAX);
                    out.extend(s.log.iter().skip(skip).cloned());
                }
                _ => out.push(VecDiff::Replace(s.items.clone())),
            }
            if let Some(reader) = s.readers.iter_mut().find(|r| r.id == self.id) {
                reader.seen = Some(version);
            }
            s.trim();
            false
        })?;
        Ok(out)
    }
}

impl<T: Clone + 'static> Drop for VecReader<T> {
    fn drop(&mut self) {
        let id = self.id;
        let _ = self.vec.state.try_update_if(|s| {
            s.readers.retain(|r| r.id != id);
            s.trim();
            false
        });
    }
}
