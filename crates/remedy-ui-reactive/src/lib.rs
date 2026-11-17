//! Reactive state for RemedyUI.
//!
//! Components run once; state lives in [`Signal`]s; derived values are [`Memo`]s; work that
//! follows state is an [`Effect`]. A change updates exactly what depends on it, and no
//! effect ever sees half-updated state. See `docs/design/02-state-and-elements.md`.
//!
//! ```
//! use remedy_ui_reactive::{Effect, Memo, Signal, batch};
//! use std::{cell::RefCell, rc::Rc};
//!
//! let first = Signal::new("Ada".to_string());
//! let last = Signal::new("Lovelace".to_string());
//! let full = Memo::new(move || format!("{} {}", first.get(), last.get()));
//!
//! let seen = Rc::new(RefCell::new(Vec::new()));
//! let log = seen.clone();
//! Effect::new(move || log.borrow_mut().push(full.get()));
//!
//! batch(|| {
//!     first.set("Grace".into());
//!     last.set("Hopper".into());
//! });
//! // One run per batch: the effect never saw "Grace Lovelace".
//! assert_eq!(*seen.borrow(), ["Ada Lovelace", "Grace Hopper"]);
//! ```
//!
//! Each UI thread has one runtime. Handles are plain ids: they may be captured by closures
//! sent to other threads, but their values are reachable only on the thread that created
//! them. Other threads use a [`Waker`] to post work to the UI thread.

mod error;
mod runtime;
mod signal;
mod store;
mod vec;
mod waker;

pub use error::ReactiveError;
pub use signal::{Effect, Memo, Scope, Signal, Stats, batch, on_cleanup, runtime_id, stats, untrack};
pub use store::{Field, Segment, Store, StoreError};
pub use vec::{OutOfRange, SignalVec, VecDiff, VecReader};
pub use waker::{Closed, PostReport, Waker, has_posted, run_posted, set_wake_handler, waker};
