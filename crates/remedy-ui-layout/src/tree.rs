//! The layout tree: nodes, measured leaves, computing, results and pixel snapping.

use remedy_ui_core::{Point, Rect, Scale, Size};
use taffy::{AvailableSpace, NodeId, TaffyTree};

use crate::style::{Direction, LayoutStyle, to_taffy};

/// A node in a [`LayoutTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutNode(NodeId);

/// The space offered to a node along one axis while measuring.
#[derive(Clone, Copy, Debug, PartialEq)]
// Exhaustive on purpose: space is definite, min-content or max-content, as in CSS sizing.
pub enum Available {
    /// A definite amount, in logical pixels.
    Px(f32),
    /// As little as possible (min-content).
    MinContent,
    /// As much as the content wants (max-content).
    MaxContent,
}

/// What a measured leaf (text, an image) is asked.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MeasureInput {
    /// Width already decided by layout, if any: the content box's, inside the padding and
    /// border.
    pub known_width: Option<f32>,
    /// Height already decided by layout, if any: the content box's, inside the padding and
    /// border.
    pub known_height: Option<f32>,
    /// Width on offer.
    pub available_width: Available,
    /// Height on offer.
    pub available_height: Available,
}

type MeasureFn = Box<dyn FnMut(MeasureInput) -> Size>;

/// A layout operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutError(pub String);

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "layout: {}", self.0)
    }
}

impl std::error::Error for LayoutError {}

fn err(e: taffy::TaffyError) -> LayoutError {
    LayoutError(e.to_string())
}

/// Layout nodes and their results. Results are cached: computing again after a change
/// redoes only what the change affects.
pub struct LayoutTree {
    taffy: TaffyTree<MeasureFn>,
    /// One viewport per computed root: the window's box, like CSS's initial containing block.
    /// An auto-width root fills it the way a page body fills the window.
    viewports: std::collections::HashMap<NodeId, NodeId>,
    /// Each node's RemedyUI style and the direction it was last translated with.
    /// Each node's style, its resolved direction, and its parent's (which its insets follow).
    styles: std::collections::HashMap<NodeId, (LayoutStyle, bool, bool)>,
}

impl std::fmt::Debug for LayoutTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LayoutTree({} nodes)", self.taffy.total_node_count())
    }
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutTree {
    /// An empty tree. Results are kept in fractional logical pixels; snapping to the device
    /// grid is a separate step ([`LayoutTree::snapped_rect`]) so it can use the window's scale.
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        LayoutTree {
            taffy,
            viewports: std::collections::HashMap::new(),
            styles: std::collections::HashMap::new(),
        }
    }

    /// A container node.
    pub fn node(&mut self, style: &LayoutStyle) -> Result<LayoutNode, LayoutError> {
        let rtl = style.direction == Direction::Rtl;
        let id = self.taffy.new_leaf(to_taffy(style, rtl, rtl)).map_err(err)?;
        self.styles.insert(id, (style.clone(), rtl, rtl));
        Ok(LayoutNode(id))
    }

    /// A leaf whose size comes from `measure` (text, images, custom content).
    pub fn measured(
        &mut self,
        style: &LayoutStyle,
        measure: impl FnMut(MeasureInput) -> Size + 'static,
    ) -> Result<LayoutNode, LayoutError> {
        let rtl = style.direction == Direction::Rtl;
        let id = self.taffy.new_leaf_with_context(to_taffy(style, rtl, rtl), Box::new(measure)).map_err(err)?;
        self.styles.insert(id, (style.clone(), rtl, rtl));
        Ok(LayoutNode(id))
    }

    /// Replaces a node's style.
    pub fn set_style(&mut self, node: LayoutNode, style: &LayoutStyle) -> Result<(), LayoutError> {
        let (rtl, parent) =
            self.styles.get(&node.0).map_or((style.direction == Direction::Rtl, false), |(_, r, p)| (*r, *p));
        // A node given a direction of its own gets it now (and its subtree at the next
        // resolve); one that inherits keeps what it resolved to.
        let rtl = match style.direction {
            Direction::Inherit => rtl,
            Direction::Ltr => false,
            Direction::Rtl => true,
        };
        self.taffy.set_style(node.0, to_taffy(style, rtl, parent)).map_err(err)?;
        self.styles.insert(node.0, (style.clone(), rtl, parent));
        Ok(())
    }

    /// Resolves inherited directions from `node` down, re-translating only nodes whose
    /// direction changed.
    fn resolve_direction(&mut self, node: NodeId, parent_rtl: bool) -> Result<(), LayoutError> {
        let mut stack = vec![(node, parent_rtl)];
        while let Some((id, parent)) = stack.pop() {
            let Some((style, was, was_parent)) = self.styles.get(&id).cloned() else { continue };
            let rtl = match style.direction {
                Direction::Inherit => parent,
                Direction::Ltr => false,
                Direction::Rtl => true,
            };
            if rtl != was || parent != was_parent {
                self.taffy.set_style(id, to_taffy(&style, rtl, parent)).map_err(err)?;
                self.styles.insert(id, (style, rtl, parent));
            }
            for c in self.taffy.children(id).map_err(err)? {
                stack.push((c, rtl));
            }
        }
        Ok(())
    }

    /// Marks a measured leaf's content as changed (new text), so it is measured again.
    pub fn mark_dirty(&mut self, node: LayoutNode) -> Result<(), LayoutError> {
        self.taffy.mark_dirty(node.0).map_err(err)
    }

    /// Appends a child.
    pub fn add_child(&mut self, parent: LayoutNode, child: LayoutNode) -> Result<(), LayoutError> {
        self.taffy.add_child(parent.0, child.0).map_err(err)
    }

    /// Replaces all children.
    pub fn set_children(&mut self, parent: LayoutNode, children: &[LayoutNode]) -> Result<(), LayoutError> {
        let ids: Vec<NodeId> = children.iter().map(|c| c.0).collect();
        self.taffy.set_children(parent.0, &ids).map_err(err)
    }

    /// Removes a node (its children are detached, not removed).
    pub fn remove(&mut self, node: LayoutNode) -> Result<(), LayoutError> {
        if let Some(v) = self.viewports.remove(&node.0) {
            self.taffy.remove(v).map_err(err)?;
        }
        self.styles.remove(&node.0);
        self.taffy.remove(node.0).map(|_| ()).map_err(err)
    }

    /// The node's children.
    pub fn children(&self, node: LayoutNode) -> Vec<LayoutNode> {
        self.taffy.children(node.0).map(|c| c.into_iter().map(LayoutNode).collect()).unwrap_or_default()
    }

    /// Lays out `root` within `available` logical pixels (`None` on an axis means
    /// unconstrained: as large as the content wants).
    pub fn compute(
        &mut self,
        root: LayoutNode,
        width: Option<f32>,
        height: Option<f32>,
    ) -> Result<(), LayoutError> {
        self.resolve_direction(root.0, false)?;
        let size = |v: Option<f32>| v.map_or(taffy::Dimension::auto(), taffy::Dimension::length);
        let viewport_style = taffy::Style {
            display: taffy::Display::Block,
            size: taffy::Size { width: size(width), height: size(height) },
            ..taffy::Style::default()
        };
        let viewport = match self.viewports.get(&root.0).copied() {
            Some(v) => {
                if self.taffy.style(v).map_err(err)? != &viewport_style {
                    self.taffy.set_style(v, viewport_style).map_err(err)?;
                }
                v
            }
            None => {
                let v = self.taffy.new_with_children(viewport_style, &[root.0]).map_err(err)?;
                self.viewports.insert(root.0, v);
                v
            }
        };
        let space = |v: Option<f32>| v.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite);
        let available = taffy::Size { width: space(width), height: space(height) };
        self.taffy
            .compute_layout_with_measure(viewport, available, |input, _node, ctx, style| {
                // Taffy's leaf algorithm applies the style (size, min/max, padding, aspect
                // ratio); the measure callback only supplies the content size.
                let conv = |a: AvailableSpace| match a {
                    AvailableSpace::Definite(v) => Available::Px(v),
                    AvailableSpace::MinContent => Available::MinContent,
                    AvailableSpace::MaxContent => Available::MaxContent,
                };
                let mut measure = ctx;
                taffy::compute_leaf_layout(
                    input,
                    style,
                    |_, _| 0.0,
                    |known: taffy::Size<Option<f32>>, avail: taffy::Size<AvailableSpace>| match measure
                        .as_mut()
                    {
                        Some(m) => {
                            // Taffy hands over a decided size as the border box, and the space
                            // on offer as the content box. The content is measured inside the
                            // padding and border, so a decided size goes over as the content
                            // box too: on the axis where taffy knows the size, the space on
                            // offer is exactly that size less the padding and border.
                            let inner = |k: Option<f32>, a: AvailableSpace| match (k, a) {
                                (Some(_), AvailableSpace::Definite(v)) => Some(v.max(0.0)),
                                (k, _) => k,
                            };
                            let s = m(MeasureInput {
                                known_width: inner(known.width, avail.width),
                                known_height: inner(known.height, avail.height),
                                available_width: conv(avail.width),
                                available_height: conv(avail.height),
                            });
                            taffy::Size { width: s.width, height: s.height }
                        }
                        None => taffy::Size::ZERO,
                    },
                )
            })
            .map_err(err)
    }

    /// True when the node lays out right to left (its own or inherited direction).
    pub fn is_rtl(&self, node: LayoutNode) -> bool {
        self.styles.get(&node.0).is_some_and(|(_, rtl, _)| *rtl)
    }

    /// The node's box relative to its parent's top-left, in logical pixels.
    pub fn rect(&self, node: LayoutNode) -> Option<Rect> {
        let l = self.taffy.layout(node.0).ok()?;
        Some(Rect::from_origin_size(
            Point::new(l.location.x, l.location.y),
            Size::new(l.size.width, l.size.height),
        ))
    }

    /// The node's box relative to `root`, adding up parents' offsets.
    pub fn absolute_rect(&self, root: LayoutNode, node: LayoutNode) -> Option<Rect> {
        let mut path = Vec::new();
        let mut cur = node.0;
        while cur != root.0 {
            path.push(cur);
            cur = self.taffy.parent(cur)?;
        }
        let mut origin = Point::ZERO;
        for id in path.iter().rev() {
            let l = self.taffy.layout(*id).ok()?;
            origin = Point::new(origin.x + l.location.x, origin.y + l.location.y);
        }
        let own = self.taffy.layout(node.0).ok()?;
        Some(Rect::from_origin_size(origin, Size::new(own.size.width, own.size.height)))
    }

    /// `absolute_rect` with each edge snapped to the device-pixel grid, so neighbours share
    /// edges exactly and borders stay crisp at fractional scales.
    pub fn snapped_rect(&self, root: LayoutNode, node: LayoutNode, scale: Scale) -> Option<Rect> {
        self.absolute_rect(root, node).map(|r| scale.snap_rect(r))
    }
}
