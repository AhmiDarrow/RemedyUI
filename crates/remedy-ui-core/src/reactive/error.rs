//! Errors from reactive operations.

use std::fmt;
use std::panic::Location;

/// Why a reactive operation could not be done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReactiveError {
    /// The signal, memo, effect or scope was disposed (its owner was disposed, or
    /// `dispose` was called). Its value no longer exists.
    Disposed,
    /// The handle belongs to another thread's runtime. Values are reachable only on the UI
    /// thread that created them; post a closure with a [`crate::reactive::Waker`] instead.
    WrongThread,
    /// This thread's runtime is gone (the thread is shutting down).
    RuntimeGone,
    /// A memo read itself while computing, or an effect re-entered itself.
    Cycle,
    /// The value is being read (inside `with`) and cannot be written at the same time.
    Borrowed,
    /// The handle does not refer to a node of the expected kind. This is a bug in RemedyUI.
    WrongKind,
}

impl fmt::Display for ReactiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ReactiveError::Disposed => "the signal was disposed; its value no longer exists",
            ReactiveError::WrongThread => {
                "the signal belongs to another thread's runtime; post a closure with a Waker to reach it"
            }
            ReactiveError::RuntimeGone => "this thread's reactive runtime has shut down",
            ReactiveError::Cycle => "a memo read itself while computing, or an effect re-entered itself",
            ReactiveError::Borrowed => "the value is being read inside `with` and cannot be written now",
            ReactiveError::WrongKind => "the handle refers to a different kind of node (a RemedyUI bug)",
        })
    }
}

impl std::error::Error for ReactiveError {}

impl ReactiveError {
    /// Turns the error into a panic that names where the node was created. Used by the
    /// non-`try_` accessors: misusing a handle is a bug, and the engine contains such panics
    /// at the nearest error boundary.
    #[track_caller]
    #[allow(clippy::panic, reason = "the documented failure of the non-try accessors")]
    pub(crate) fn raise(self, created_at: Option<&'static Location<'static>>) -> ! {
        match created_at {
            Some(at) => panic!("{self} (created at {at})"),
            None => panic!("{self}"),
        }
    }
}
