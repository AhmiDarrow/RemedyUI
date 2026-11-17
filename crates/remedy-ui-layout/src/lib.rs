//! RemedyUI layout.
//!
//! - [`LayoutTree`]: flex, grid and block layout (taffy, behind RemedyUI's own
//!   [`LayoutStyle`]), measured leaves for text and images, results relative to the parent
//!   or the root, and edges snapped to the device-pixel grid.
//! - [`VirtualList`]: lists of any length with height caches, estimates, anchoring and
//!   stick-to-end.
//!
//! See `docs/design/03-layout.md`.
//!
//! ```
//! use remedy_ui_layout::*;
//!
//! let mut t = LayoutTree::new();
//! let root = t.node(&LayoutStyle { gap: (8.0, 0.0), ..LayoutStyle::row() })?;
//! let a = t.node(&LayoutStyle::fixed(100.0, 28.0))?;
//! let b = t.node(&LayoutStyle { grow: 1.0, ..LayoutStyle::fixed(0.0, 28.0) })?;
//! t.set_children(root, &[a, b])?;
//! t.compute(root, Some(400.0), Some(28.0))?;
//! assert_eq!(t.rect(b).map(|r| (r.min.x, r.width())), Some((108.0, 292.0)));
//! # Ok::<(), LayoutError>(())
//! ```

mod style;
mod tree;
mod virtual_list;

pub use style::{AUTO, Align, Axis, Direction, Display, Justify, LayoutStyle, Length, Position, Track};
pub use tree::{Available, LayoutError, LayoutNode, LayoutTree, MeasureInput};
pub use virtual_list::{JumpAlign, VirtualList};
