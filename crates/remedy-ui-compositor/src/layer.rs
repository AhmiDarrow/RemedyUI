//! The layer tree the UI thread commits.

use std::sync::Arc;

use remedy_ui_core::{CornerRadii, Rect, Size, Vec2};
use remedy_ui_paint::{Affine, DisplayList};

/// Frosted glass under a layer: what the window shows below its box, blurred, before its own
/// content (CSS `backdrop-filter: blur()`). It sees every layer drawn before it, back to the
/// innermost translucent group around it (its backdrop root, as on the web).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    /// The blur's standard deviation, in the layer's local units.
    pub blur: f32,
    /// The glass's corners (the layer's box is the glass).
    pub radii: CornerRadii,
}

/// A layer's identity, stable across commits (usually the element id that owns it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub u64);

/// One layer: a display list in its own coordinate space, with the properties the
/// compositor can change on its own (transform, opacity, scroll offset).
///
/// A scroll container following another's offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollFollow {
    /// The scroll container it follows.
    pub leader: LayerId,
    /// Follow the horizontal offset.
    pub x: bool,
    /// Follow the vertical offset.
    pub y: bool,
}

/// Coordinates: the layer sits at `bounds` in its parent's content space. Its local space
/// has the origin at `bounds.min`. Its content (the display list and the child layers) is in
/// content space, which is local space moved by the scroll offset. A clip applies in local
/// space, so scrolled content slides under it.
#[derive(Clone, Debug)]
pub struct Layer {
    /// Identity across commits.
    pub id: LayerId,
    /// Where the layer sits in its parent's content space; its size is the viewport of a
    /// scrolling layer and the clip box of a clipping one.
    pub bounds: Rect,
    /// Applied in local space before placing at `bounds.min`.
    pub transform: Affine,
    /// Group opacity, 0 to 1.
    pub opacity: f32,
    /// Clips content and children to the local box with these corners.
    pub clip: Option<CornerRadii>,
    /// Makes the layer a scroll container whose content has this size.
    pub scroll: Option<Size>,
    /// The scroll offset at which a scroll container shows the start of its content: zero,
    /// or in a right-to-left container whose content overflows to the left, how far it does.
    /// The container opens there and stays anchored there as its content grows.
    pub scroll_origin: Vec2,
    /// A scroll container that moves with another one on some axes, in the same frame
    /// (frozen table columns follow the body vertically; a header follows it horizontally).
    pub scroll_follow: Option<ScrollFollow>,
    /// What the layer draws, in content space.
    pub content: Arc<DisplayList>,
    /// Bumped by the UI thread whenever `content` changes; tiles re-raster on a new version.
    pub content_version: u64,
    /// Children, drawn after the content, in order.
    pub children: Vec<Arc<Layer>>,
    /// Endless transform and opacity loops the compositor runs on this layer at the window's
    /// ambient rate, with no new commits ([`AmbientLoop`](crate::AmbientLoop)).
    pub ambient: Vec<crate::AmbientLoop>,
    /// Blurs what lies under the layer's box before its content draws.
    pub backdrop: Option<Backdrop>,
    /// Filters on the layer and every layer under it, drawn as one picture (a blur, a drop
    /// shadow of its shape), outermost first: what an element with a filter becomes when
    /// some of what it draws is in layers of its own.
    pub filters: Vec<remedy_ui_paint::Filter>,
}

impl Layer {
    /// A layer at `bounds` drawing `content`, unclipped, opaque and untransformed.
    pub fn new(id: LayerId, bounds: Rect, content: DisplayList) -> Self {
        Layer {
            id,
            bounds,
            transform: Affine::IDENTITY,
            opacity: 1.0,
            clip: None,
            scroll: None,
            scroll_origin: Vec2::ZERO,
            scroll_follow: None,
            content: Arc::new(content),
            content_version: 0,
            children: Vec::new(),
            ambient: Vec::new(),
            backdrop: None,
            filters: Vec::new(),
        }
    }

    /// Draws the layer and its children through `filters`, outermost first.
    pub fn with_filters(mut self, filters: Vec<remedy_ui_paint::Filter>) -> Self {
        self.filters = filters;
        self
    }

    /// Blurs what lies under the layer's box (rounded by `radii`) before its content.
    pub fn with_backdrop(mut self, blur: f32, radii: CornerRadii) -> Self {
        self.backdrop = Some(Backdrop { blur, radii });
        self
    }

    /// Clips to the layer box with `radii`.
    pub fn clipped(mut self, radii: CornerRadii) -> Self {
        self.clip = Some(radii);
        self
    }

    /// Makes the layer a clipped scroll container over content of `size`.
    pub fn scrolling(mut self, content_size: Size) -> Self {
        self.scroll = Some(content_size);
        if self.clip.is_none() {
            self.clip = Some(CornerRadii::ZERO);
        }
        self
    }

    /// Adds a child.
    pub fn with_child(mut self, child: Layer) -> Self {
        self.children.push(Arc::new(child));
        self
    }

    /// The local box: origin at zero, the size of `bounds`.
    pub fn local_box(&self) -> Rect {
        Rect::from_origin_size(remedy_ui_core::Point::ZERO, self.bounds.size())
    }

    /// The largest scroll offset on each axis (zero when not scrolling).
    pub fn max_scroll(&self) -> Vec2 {
        match self.scroll {
            Some(c) => Vec2::new(
                (c.width - self.bounds.width()).max(0.0),
                (c.height - self.bounds.height()).max(0.0),
            ),
            None => Vec2::ZERO,
        }
    }

    /// The area content draws to, in content space: what tiles cover.
    pub fn content_extent(&self) -> Option<Rect> {
        let drawn = self.content.bounds();
        let scrolled = self.scroll.map(|s| Rect::from_origin_size(remedy_ui_core::Point::ZERO, s));
        match (drawn, scrolled) {
            (Some(d), Some(s)) => d.intersect(s.union(self.local_box())),
            (Some(d), None) if self.clip.is_some() => d.intersect(self.local_box()),
            (Some(d), None) => Some(d),
            (None, _) => None,
        }
        .filter(|r| !r.is_empty())
    }
}

/// A committed snapshot: the root layers of one window. Unchanged layers are shared with the
/// previous snapshot through `Arc`, so a commit costs as much as the change.
#[derive(Clone, Debug, Default)]
pub struct LayerTree {
    /// Top-level layers, back to front.
    pub roots: Vec<Arc<Layer>>,
}

impl LayerTree {
    /// A tree of these roots.
    pub fn new(roots: Vec<Layer>) -> Self {
        LayerTree { roots: roots.into_iter().map(Arc::new).collect() }
    }

    /// Visits every layer, parents before children.
    pub fn for_each(&self, mut f: impl FnMut(&Layer)) {
        fn walk(l: &Layer, f: &mut impl FnMut(&Layer)) {
            f(l);
            for c in &l.children {
                walk(c, f);
            }
        }
        for r in &self.roots {
            walk(r, &mut f);
        }
    }

    /// The layer with `id`, if any.
    pub fn find(&self, id: LayerId) -> Option<&Layer> {
        fn walk(l: &Layer, id: LayerId) -> Option<&Layer> {
            if l.id == id {
                return Some(l);
            }
            l.children.iter().find_map(|c| walk(c, id))
        }
        self.roots.iter().find_map(|r| walk(r, id))
    }
}
