//! The RemedyUI element tree.
//!
//! Components build elements once; the tree keeps them, tracks which frame phases each
//! element needs ([`Dirty`]), routes events through capture and bubble phases, finds what is
//! under the pointer, keeps keyboard focus, and contains failures in error boundaries. See
//! `docs/design/02-state-and-elements.md`.
//!
//! ```
//! use remedy_ui_tree::{Dirty, Phase, Tree};
//!
//! let tree = Tree::new();
//! let panel = tree.create("panel");
//! let button = tree.create("button");
//! tree.append(tree.root(), panel)?;
//! tree.append(panel, button)?;
//! tree.set_relayout_boundary(panel, true)?;
//!
//! // Layout dirtiness climbs only to the nearest relayout boundary.
//! tree.take_dirty(Dirty::LAYOUT | Dirty::CHILD_LAYOUT);
//! tree.mark(button, Dirty::LAYOUT);
//! assert!(tree.dirty(panel).contains(Dirty::CHILD_LAYOUT));
//! assert!(!tree.dirty(tree.root()).contains(Dirty::CHILD_LAYOUT));
//! # Ok::<(), remedy_ui_tree::TreeError>(())
//! ```

mod dirty;
mod tree;

pub use dirty::Dirty;
pub use tree::{
    Dispatch, ElementId, EventCx, Failure, FailureHandler, FocusIn, FocusOut, HitMode, Node, Phase, Tree,
    TreeError,
};
