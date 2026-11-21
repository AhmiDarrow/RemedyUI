//! Recursion over the element tree that cannot be a loop runs on a stack that grows: a tree
//! of any depth is walked on any thread.

/// Stack that must be left before going a level deeper, in bytes: more than one level of a
/// walk and the work at its deepest point (a leaf's drawing) use, unoptimized.
const RED_ZONE: usize = 256 << 10;

/// Stack added when what is left falls below [`RED_ZONE`], in bytes: enough for a few
/// hundred more levels before the next is needed.
const SEGMENT: usize = 8 << 20;

/// Runs `f`, one level deeper in a recursive walk, here when this stack has room and on a
/// further stack when not.
#[inline]
pub(crate) fn deeper<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, SEGMENT, f)
}
