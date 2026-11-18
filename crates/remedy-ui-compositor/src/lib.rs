//! RemedyUI's compositor.
//!
//! The UI thread records display lists into a [`LayerTree`] and commits it through a
//! [`Mailbox`]. The [`Compositor`] owns what can change without app code: scroll offsets
//! (with fling and overscroll physics, and wheel routing that hands exhausted scrolls
//! outward), and transform and opacity animations. Each frame it plans which tiles to raster
//! (visible first, then a one-tile prefetch ring), the composite of those tiles with clips and
//! opacity groups, and the damage in device pixels, so renderers raster only what changed and
//! bridges present only what changed. See `docs/design/01-threads-and-frames.md` and
//! `docs/design/04-rendering.md`.

mod compositor;
mod damage;
mod layer;
mod mailbox;
mod thread;
mod tiles;

pub use compositor::{CompositeOp, Compositor, FramePlan, TileRequest, logical_size};
pub use damage::Damage;
pub use layer::{Layer, LayerId, LayerTree, ScrollFollow};
pub use mailbox::Mailbox;
pub use thread::{CompositorThread, FrameReport, Presenter};
pub use tiles::{TileKey, quantize_scale, tile_pixels};
