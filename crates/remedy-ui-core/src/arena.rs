//! Generational ids and the arena that hands them out.
//!
//! Signals, elements and other engine objects live in arenas and are referred to by [`Id`].
//! An id is an index plus a generation. When a slot is freed its generation goes up, so an old
//! id can never reach the object that reuses the slot: lookups with a stale id return `None`.
//!
//! The arena has no fixed size. It grows as objects are added and reuses freed slots.

use std::fmt;
use std::marker::PhantomData;

/// A generational id for a value of type `T` in an [`Arena<T>`].
///
/// Ids are `Copy`, compare by value and are only meaningful for the arena that made them.
pub struct Id<T> {
    index: u32,
    generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// The slot index. Exposed for packing ids into other structures and for debugging.
    pub fn index(self) -> u32 {
        self.index
    }

    /// The slot generation.
    pub fn generation(self) -> u32 {
        self.generation
    }

    /// Rebuilds an id from its parts, as produced by [`Id::index`] and [`Id::generation`].
    /// A wrong pair is harmless: lookups with it return `None`.
    pub fn from_parts(index: u32, generation: u32) -> Self {
        Id { index, generation, _marker: PhantomData }
    }

    /// The same index and generation as an id for another type. Used by crates that keep
    /// parallel arenas indexed alike.
    pub fn cast<U>(self) -> Id<U> {
        Id::from_parts(self.index, self.generation)
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Id<T> {}

impl<T> std::hash::Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({}v{})", self.index, self.generation)
    }
}

enum Slot<T> {
    Occupied { generation: u32, value: T },
    Free { generation: u32, next_free: Option<u32> },
}

/// A growable store of values addressed by generational [`Id`]s.
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free_head: Option<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena { slots: Vec::new(), free_head: None, len: 0 }
    }
}

impl<T> fmt::Debug for Arena<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Arena").field("len", &self.len).field("slots", &self.slots.len()).finish()
    }
}

/// The arena already holds `u32::MAX` slots and cannot address another. This is the bound of
/// the 32-bit index, not a chosen limit; at one byte per slot it is four gigabytes of slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaFull;

impl fmt::Display for ArenaFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the arena has used every 32-bit index")
    }
}

impl std::error::Error for ArenaFull {}

impl<T> Arena<T> {
    /// An empty arena.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of live values.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no values are live.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Stores `value` and returns its id, reusing a freed slot when there is one.
    pub fn insert(&mut self, value: T) -> Result<Id<T>, ArenaFull> {
        if let Some(index) = self.free_head
            && let Some(slot) = self.slots.get_mut(index as usize)
            && let Slot::Free { generation, next_free } = *slot
        {
            self.free_head = next_free;
            *slot = Slot::Occupied { generation, value };
            self.len += 1;
            return Ok(Id::from_parts(index, generation));
        }
        let index = u32::try_from(self.slots.len()).map_err(|_| ArenaFull)?;
        if index == u32::MAX {
            return Err(ArenaFull);
        }
        self.slots.push(Slot::Occupied { generation: 0, value });
        self.len += 1;
        Ok(Id::from_parts(index, 0))
    }

    /// The value for `id`, or `None` if it was removed (or `id` is from another arena).
    pub fn get(&self, id: Id<T>) -> Option<&T> {
        match self.slots.get(id.index as usize)? {
            Slot::Occupied { generation, value } if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    /// The value for `id`, mutably.
    pub fn get_mut(&mut self, id: Id<T>) -> Option<&mut T> {
        match self.slots.get_mut(id.index as usize)? {
            Slot::Occupied { generation, value } if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    /// True when `id` refers to a live value.
    pub fn contains(&self, id: Id<T>) -> bool {
        self.get(id).is_some()
    }

    /// Removes and returns the value for `id`. The slot's generation goes up, so `id` and any
    /// copy of it stop working.
    pub fn remove(&mut self, id: Id<T>) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        match slot {
            Slot::Occupied { generation, .. } if *generation == id.generation => {
                // A slot whose generation would wrap is retired instead of reused, so an old
                // id can never match again.
                let next_generation = generation.checked_add(1);
                let freed =
                    Slot::Free { generation: next_generation.unwrap_or(u32::MAX), next_free: self.free_head };
                let Slot::Occupied { value, .. } = std::mem::replace(slot, freed) else {
                    return None;
                };
                if next_generation.is_some() {
                    self.free_head = Some(id.index);
                }
                self.len -= 1;
                Some(value)
            }
            _ => None,
        }
    }

    /// Every live id and value, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| match s {
            Slot::Occupied { generation, value } => {
                u32::try_from(i).ok().map(|i| (Id::from_parts(i, *generation), value))
            }
            Slot::Free { .. } => None,
        })
    }

    /// Every live id and value, mutably, in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Id<T>, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| match s {
            Slot::Occupied { generation, value } => {
                u32::try_from(i).ok().map(|i| (Id::from_parts(i, *generation), value))
            }
            Slot::Free { .. } => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_ids_never_reach_a_reused_slot() -> Result<(), ArenaFull> {
        let mut a = Arena::new();
        let x = a.insert("x")?;
        assert_eq!(a.remove(x), Some("x"));
        let y = a.insert("y")?;
        assert_eq!(x.index(), y.index(), "slot is reused");
        assert_ne!(x, y);
        assert_eq!(a.get(x), None);
        assert_eq!(a.get(y), Some(&"y"));
        assert_eq!(a.remove(x), None);
        assert_eq!(a.len(), 1);
        Ok(())
    }

    #[test]
    fn grows_without_a_fixed_size() -> Result<(), ArenaFull> {
        let mut a = Arena::new();
        let ids: Vec<_> = (0..100_000).map(|i| a.insert(i)).collect::<Result<_, _>>()?;
        assert_eq!(a.len(), 100_000);
        for (i, id) in ids.iter().enumerate().step_by(2) {
            assert_eq!(a.remove(*id), Some(i));
        }
        assert_eq!(a.len(), 50_000);
        assert_eq!(a.iter().count(), 50_000);
        Ok(())
    }

    #[test]
    fn iter_mut_edits_in_place() -> Result<(), ArenaFull> {
        let mut a = Arena::new();
        let x = a.insert(1)?;
        for (_, v) in a.iter_mut() {
            *v += 1;
        }
        assert_eq!(a.get(x), Some(&2));
        Ok(())
    }
}
