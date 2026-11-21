//! Recursion that runs through app code (a memo reading a memo) cannot be a loop: it runs
//! on a stack that grows, so a chain of any length is read on any thread.

/// Stack that must be left before going a level deeper, in bytes: more than a level (the
/// runtime's bookkeeping and a computation between two reads) uses, unoptimized.
const RED_ZONE: usize = 256 << 10;

/// Stack added when what is left falls below [`RED_ZONE`], in bytes: enough for a few
/// hundred more levels before the next is needed.
const SEGMENT: usize = 8 << 20;

/// Runs `f`, one level deeper, here when this stack has room and on a further stack when
/// not.
#[inline]
pub(crate) fn deeper<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, SEGMENT, f)
}
