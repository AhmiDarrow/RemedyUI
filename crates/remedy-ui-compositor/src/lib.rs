//! RemedyUI's compositor.
//!
//! The UI thread records display lists into a [`LayerTree`] and commits it through a
//! [`Mailbox`]. The [`Compositor`] owns what can change without app code: scroll offsets
//! (with fling and overscroll physics, and wheel routing that hands exhausted scrolls
//! outward), and transform and opacity animations. Each frame it plans which tiles to raster
//! (visible first, then a one-tile prefetch ring), the composite of those tiles with clips and
//! opacity groups, and the damage in device pixels, so renderers raster only what changed and
//! bridges present only what changed.
//!
//! **Backdrops.** A layer with a [`Backdrop`] is frosted glass: the composite blurs what the
//! layers before it drew under its box ([`CompositeOp::Backdrop`]), and damage under the glass
//! damages the glass, so a frame drawn from damage alone matches a full one.
//!
//! **Progressive zoom.** When a layer's raster scale changes (a zoom, a move to a screen of
//! another density), its tiles at the old scale stay and are drawn scaled in place of the new
//! ones until those are rastered. New tiles raster visible-first, nearest the middle of the
//! screen first, within a per-frame [`RasterBudget`] measured on the machine; content with
//! nothing to stand in for it is never held back. See `docs/design/01-threads-and-frames.md`
//! and `docs/design/04-rendering.md` (Tiles).

mod ambient;
mod compositor;
mod damage;
mod layer;
mod mailbox;
mod thread;
mod tiles;

pub use ambient::{AmbientLoop, LoopMotion, Wave};
pub use compositor::{CompositeOp, Compositor, FramePlan, TileRequest, logical_size};
pub use damage::Damage;
pub use layer::{Backdrop, Layer, LayerId, LayerTree, ScrollFollow, ScrollSnap, SnapPoint, Sticky};
pub use mailbox::Mailbox;
pub use thread::{CompositorThread, FrameReport, Presenter};
pub use tiles::{DEFAULT_ZOOM_RASTER_TIME, RasterBudget, TileKey, quantize_scale, scale_step, tile_pixels};
