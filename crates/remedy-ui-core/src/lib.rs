//! Base types shared by every RemedyUI crate.
//!
//! This crate is layer L0 of the engine (see `docs/design/00-principles-and-modularity.md`):
//! it has no dependencies and holds only plain data and small algorithms.
//!
//! - [`geometry`]: points, vectors, sizes, rectangles, insets and corner radii in logical pixels.
//! - [`units`]: the scale from logical to device pixels, and snapping edges to the device grid.
//! - [`color`]: colors in sRGB, with conversion to linear light and mixing in Oklab.
//! - [`arena`]: generational ids and an arena that grows with use and never hands out a stale id.

pub mod arena;
pub mod color;
pub mod geometry;
pub mod units;

pub use arena::{Arena, Id};
pub use color::Color;
pub use geometry::{CornerRadii, Insets, Point, Rect, Size, Vec2};
pub use units::Scale;
