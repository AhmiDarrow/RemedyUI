//! The bounds renderers draw a display list within: every one, with its default, set by the
//! app with [`set_paint_limits`]. Both renderers (CPU and GPU) read the same bounds, so a
//! raised one draws the same on either.

use std::sync::{PoisonError, RwLock};

use crate::{
    MAX_BACKDROP_DEPTH, MAX_BACKDROP_REPLAYS, MAX_FILTER_MATRIX_DEPTH, MAX_FILTER_MATRIX_GROUPS,
    MAX_FILTER_OFFSET, MAX_FILTER_STD_DEV, MAX_GROUP_DEPTH,
};

/// The bounds renderers draw a display list within, for every renderer in the process
/// ([`set_paint_limits`]). The defaults are the `MAX_*` constants of this crate. Each one
/// keeps a list from a stranger (a theme, a script, a document) from costing without bound;
/// an app that draws only its own lists may raise them, and one showing untrusted lists may
/// lower them.
///
/// What raising each risks: `group_depth` is a safety bound on vello's recursion (one level
/// per open group, outside RemedyUI's control), so a much higher one can overflow the
/// drawing thread's stack; give that thread a larger stack before raising it far.
/// `backdrop_depth` doubles the cost of glass for every level; `backdrop_replays`,
/// `filter_matrix_groups` and `filter_matrix_depth` each cost a picture and a pass of their
/// own; `filter_std_dev` and `filter_offset` grow a filter's buffers by about three times the
/// blur and by the offset on every side, which costs memory with the square of the reach.
///
/// ```
/// use remedy_ui_paint::{PaintLimits, set_paint_limits};
/// set_paint_limits(PaintLimits::default().backdrop_depth(4).filter_std_dev(128.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PaintLimits {
    /// The most groups (clips, opacity and blend layers, filters, glass's layers) open at
    /// once; a push past it opens nothing. Default [`MAX_GROUP_DEPTH`] (256).
    pub group_depth: usize,
    /// How deep glass seen through glass is drawn with its frost. Default
    /// [`MAX_BACKDROP_DEPTH`] (2).
    pub backdrop_depth: usize,
    /// The most panes of glass drawn with their frost in one raster (a tile, a frame).
    /// Default [`MAX_BACKDROP_REPLAYS`] (128).
    pub backdrop_replays: usize,
    /// The most colour-matrix groups drawn through their matrix nested in one another.
    /// Default [`MAX_FILTER_MATRIX_DEPTH`] (4).
    pub filter_matrix_depth: usize,
    /// The most colour-matrix groups drawn through their matrix in one raster. Default
    /// [`MAX_FILTER_MATRIX_GROUPS`] (64).
    pub filter_matrix_groups: usize,
    /// The most a chain of filters blurs, as a standard deviation (device pixels at scale 1
    /// or less). Default [`MAX_FILTER_STD_DEV`] (64).
    pub filter_std_dev: f32,
    /// The farthest a chain of drop shadows falls, in device pixels, on each axis. Default
    /// [`MAX_FILTER_OFFSET`] (2048).
    pub filter_offset: f32,
}

impl PaintLimits {
    /// The defaults, as a constant.
    pub const DEFAULT: PaintLimits = PaintLimits {
        group_depth: MAX_GROUP_DEPTH,
        backdrop_depth: MAX_BACKDROP_DEPTH,
        backdrop_replays: MAX_BACKDROP_REPLAYS,
        filter_matrix_depth: MAX_FILTER_MATRIX_DEPTH,
        filter_matrix_groups: MAX_FILTER_MATRIX_GROUPS,
        filter_std_dev: MAX_FILTER_STD_DEV,
        filter_offset: MAX_FILTER_OFFSET,
    };
}

impl Default for PaintLimits {
    fn default() -> Self {
        PaintLimits::DEFAULT
    }
}

macro_rules! setters {
    ($($name:ident: $t:ty),* $(,)?) => {
        impl PaintLimits {
            $(
                #[doc = concat!("Sets [`", stringify!($name), "`](Self::", stringify!($name), ").")]
                pub fn $name(mut self, value: $t) -> Self {
                    self.$name = value;
                    self
                }
            )*
        }
    };
}

setters! {
    group_depth: usize,
    backdrop_depth: usize,
    backdrop_replays: usize,
    filter_matrix_depth: usize,
    filter_matrix_groups: usize,
    filter_std_dev: f32,
    filter_offset: f32,
}

static LIMITS: RwLock<PaintLimits> = RwLock::new(PaintLimits::DEFAULT);

/// Sets the bounds every renderer in the process draws display lists within, from the next
/// frame (a raster under way keeps the bounds it started with). A blur or offset bound that
/// is not finite or is negative counts as 0.
pub fn set_paint_limits(limits: PaintLimits) {
    *LIMITS.write().unwrap_or_else(PoisonError::into_inner) = limits;
}

/// The bounds renderers draw within now ([`set_paint_limits`]).
pub fn paint_limits() -> PaintLimits {
    *LIMITS.read().unwrap_or_else(PoisonError::into_inner)
}
