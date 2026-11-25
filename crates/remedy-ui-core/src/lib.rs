//! Base types shared by every RemedyUI crate.
//!
//! This crate is layer L0 of the engine (see `docs/design/00-principles-and-modularity.md`):
//! it holds plain data and small algorithms, plus the two platform services everything else
//! builds on, so the engine runs the same natively and in a browser page.
//!
//! - [`geometry`]: points, vectors, sizes, rectangles, insets and corner radii in logical pixels.
//! - [`units`]: the scale from logical to device pixels, and snapping edges to the device grid.
//! - [`scrollbar`]: how scroll containers show their scroll bars.
//! - [`color`]: colors in sRGB, with conversion to linear light and mixing in Oklab.
//! - [`arena`]: generational ids and an arena that grows with use and never hands out a stale id.
//! - [`token`]: unguessable tokens for local connections, compared in constant time.
//! - [`time`]: monotonic and wall clocks that work on every platform, browsers included.
//! - [`exec`]: work away from the UI thread: threads natively, cooperative time slices in a
//!   browser page without threads.
//! - [`trace`]: the phase spans every part of the engine emits (through `tracing`), the clock
//!   trace timestamps share across threads, GPU work slices, and gauges of what the engine holds.
//! - [`edition`]: editions, the switch that changes of default behavior hang off.

pub mod arena;
pub mod color;
pub mod color_matrix;
pub mod edition;
pub mod exec;
pub mod geometry;
pub mod scrollbar;
pub mod text;
pub mod time;
pub mod token;
pub mod trace;
pub mod units;

pub use arena::{Arena, Id};
pub use color::Color;
pub use color_matrix::ColorMatrix;
pub use edition::{Edition, ParseEditionError};
pub use geometry::{CornerRadii, Insets, Point, Rect, Size, Vec2};
pub use scrollbar::{ScrollbarColor, ScrollbarVisibility, ScrollbarWidth};
pub use text::{LineHeight, TextAlign, TextTransform};
pub use units::Scale;

pub mod a11y;
pub mod host;
pub mod input;
pub mod layout;
pub mod motion;
pub mod reactive;
pub mod style;
pub mod testing;
pub mod tree;
