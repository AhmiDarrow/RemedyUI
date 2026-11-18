//! The frame pipeline: style, layout, bounds, paint chunks, layers and semantics, each
//! redone only for what changed.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_compositor::{Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Insets, Point, Rect, Size, Vec2};
use remedy_ui_paint::{Affine, Brush, DisplayList, GlyphRun, Item};
use remedy_ui_style::{States, StyleKey};
use remedy_ui_text::{Antialias, Hinting, Paragraph};
use remedy_ui_tree::{Dirty, ElementId};

use crate::element::{ElData, Inner, PaintCx, Ui, root_key};

/// What one frame produced.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// The layers to commit; `None` when nothing visible changed.
    pub layers: Option<LayerTree>,
    /// Scroll containers by layer, for routing scroll offsets back with
    /// [`Ui::set_scroll_offset`].
    pub scrollers: Vec<(LayerId, ElementId)>,
}

#[derive(Default)]
pub(crate) struct FrameState {
    lists: HashMap<LayerId, (Arc<DisplayList>, u64)>,
    pub(crate) bounds_stale: bool,
    scrollers: Vec<(LayerId, ElementId)>,
}

/// The layer id of a scroll container's viewport and of its content.
pub(crate) fn scroll_ids(id: ElementId) -> (LayerId, LayerId) {
    let k = root_key(id);
    (LayerId(k.wrapping_mul(4).wrapping_add(1)), LayerId(k.wrapping_mul(4).wrapping_add(2)))
}

/// The layer ids of an isolated element: its clip, and its content inside.
pub(crate) fn isolate_ids(id: ElementId) -> (LayerId, LayerId) {
    let k = root_key(id);
    let clip = LayerId(k.wrapping_mul(4).wrapping_add(3));
    (clip, LayerId(clip.0 | (1 << 62)))
}

/// A paint segment after a child layer: keeps paint order around scroll containers.
fn segment_id(owner: LayerId, n: u64) -> LayerId {
    LayerId((owner.0.wrapping_mul(1_000_003) ^ n.wrapping_mul(0x9E37_79B9)) | (1 << 63))
}

pub(crate) fn text_style_for(d: &ElData, theme: &remedy_ui_style::Theme) -> remedy_ui_text::TextStyle {
    remedy_ui_text::TextStyle {
        families: if d.resolved.mono { theme.mono_families.clone() } else { theme.font_families.clone() },
        size: d.resolved.text_size,
        weight: d.resolved.weight,
        color: d.resolved.foreground,
        ..remedy_ui_text::TextStyle::default()
    }
}

fn record(
    d: &ElData,
    rtl: bool,
    size: Size,
    theme: &remedy_ui_style::Theme,
    engine: &mut remedy_ui_text::TextEngine,
    focused: bool,
    scale: f32,
) -> DisplayList {
    let mut list = DisplayList::new();
    let s = &d.resolved;
    let rect = Rect::from_origin_size(Point::ZERO, size);
    let radii = CornerRadii::all(s.radius);
    if s.background.a > 0.0 {
        list.fill_rounded(rect, radii, Brush::Solid(s.background));
    }
    if s.border_width > 0.0 && s.border.a > 0.0 {
        list.border(rect, radii, s.border_width, s.border);
    }
    let bw = s.border_width;
    let content = rect.deflate(Insets {
        top: s.padding.top + bw,
        end: s.padding.end + bw,
        bottom: s.padding.bottom + bw,
        start: s.padding.start + bw,
    });
    if let Some(slot) = &d.text {
        let mut slot = slot.borrow_mut();
        slot.rtl = rtl;
        let p = slot.paragraph(engine, Some(content.width().max(0.0)));
        let x = content.min.x + p.x_offset(content.width());
        // Marked and selected text show their highlights under the glyphs.
        let tokens = theme.derive();
        for r in &d.marks {
            for rect in p.selection_rects(r.clone()) {
                list.fill_rect(
                    rect.translate(remedy_ui_core::Vec2::new(x, content.min.y)),
                    Brush::Solid(tokens.mark),
                );
            }
        }
        if let Some(r) = d.sel.clone() {
            for rect in p.selection_rects(r) {
                list.fill_rect(
                    rect.translate(remedy_ui_core::Vec2::new(x, content.min.y)),
                    Brush::Solid(tokens.selected),
                );
            }
        }
        glyphs(&mut list, &p, Point::new(x, content.min.y), s.foreground);
    }
    if let Some(field) = &d.field {
        let mut f = field.borrow_mut();
        if f.rtl != rtl {
            f.rtl = rtl;
            f.invalidate();
        }
        drop(f);
        field.borrow_mut().paint(&mut list, content, s, theme, engine, focused);
    }
    if let Some(painter) = &d.painter {
        let mut cx = PaintCx { list: &mut list, size, style: s, theme, states: d.states, scale };
        let tracked = d.paint_watch.as_ref().is_some_and(|w| w.track(|| painter(&mut cx)).is_ok());
        if !tracked {
            painter(&mut cx);
        }
    }
    list
}

/// Adds a paragraph's glyph runs at `origin`. Run colors come from the paragraph's spans;
/// `default` is the element's text color.
/// What an element tells assistive technology: its own semantics, or what its text or field
/// implies, plus disabled, click and focus from its state.
pub(crate) fn composed_semantics(d: &ElData, is_root: bool, focusable: bool) -> Semantics {
    let mut s = match &d.semantics {
        Some(s) => s.clone(),
        None if is_root => Semantics::new(Role::Window),
        None => match (&d.text, &d.field) {
            (Some(t), _) => Semantics::new(Role::Label).label(t.borrow().text.as_str()),
            (None, Some(f)) => f.borrow().semantics(),
            _ => Semantics::new(Role::Group),
        },
    };
    if d.states.contains(States::DISABLED) {
        s = s.disabled(true);
    }
    if d.click.is_some() && d.semantics.is_none() {
        s = s.action(remedy_ui_a11y::Action::Click);
    }
    if focusable {
        s = s.action(remedy_ui_a11y::Action::Focus);
    }
    s
}

pub(crate) fn glyphs(list: &mut DisplayList, p: &Paragraph, origin: Point, default: Color) {
    for run in p.glyph_runs() {
        let color = if run.color == Color::BLACK { default } else { run.color };
        for bar in [run.underline, run.strikethrough].into_iter().flatten() {
            list.fill_rect(bar.translate(origin.to_vec2()), Brush::Solid(color));
        }
        list.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(origin.x + g.x, origin.y + g.y))).collect(),
            hinting: Hinting::Full,
            antialias: Antialias::Grayscale,
        }));
    }
}

struct Builder {
    id: LayerId,
    bounds: Rect,
    scroll: Option<Size>,
    follow: Option<remedy_ui_compositor::ScrollFollow>,
    list: DisplayList,
    /// Push items open in `list`, re-opened when a segment starts.
    open: Vec<Item>,
    children: Vec<Layer>,
    segments: u64,
}

impl Builder {
    fn new(id: LayerId, bounds: Rect) -> Self {
        Builder {
            id,
            bounds,
            scroll: None,
            follow: None,
            list: DisplayList::new(),
            open: Vec::new(),
            children: Vec::new(),
            segments: 0,
        }
    }

    fn push(&mut self, item: Item) {
        self.open.push(item.clone());
        self.list.push(item);
    }

    fn pop(&mut self, item: Item) {
        self.open.pop();
        self.list.push(item);
    }

    /// Closes the current segment so a child layer can go above it, and opens the next.
    fn split(&mut self, state: &mut FrameState) {
        for item in self.open.iter().rev() {
            self.list.push(match item {
                Item::PushClip { .. } => Item::PopClip,
                Item::PushLayer { .. } => Item::PopLayer,
                _ => Item::PopTransform,
            });
        }
        let list = std::mem::take(&mut self.list);
        let seg = segment_id(self.id, self.segments);
        self.segments += 1;
        let local = Rect::from_origin_size(Point::ZERO, self.bounds.size());
        if !list.is_empty() {
            self.children.push(finish(state, seg, local, list));
        }
        for item in self.open.clone() {
            self.list.push(item);
        }
    }

    fn finish_into(mut self, state: &mut FrameState) -> Layer {
        for item in self.open.iter().rev() {
            self.list.push(match item {
                Item::PushClip { .. } => Item::PopClip,
                Item::PushLayer { .. } => Item::PopLayer,
                _ => Item::PopTransform,
            });
        }
        self.open.clear();
        // A layer cut around child layers paints its own content below all of them, so what
        // came after the last child goes above it as one more segment, keeping tree order.
        let own = if self.segments > 0 {
            let rest = std::mem::take(&mut self.list);
            if !rest.is_empty() {
                let seg = segment_id(self.id, self.segments);
                let local = Rect::from_origin_size(Point::ZERO, self.bounds.size());
                self.children.push(finish(state, seg, local, rest));
            }
            DisplayList::new()
        } else {
            self.list
        };
        let mut layer = finish(state, self.id, self.bounds, own);
        layer.children = self.children.into_iter().map(Arc::new).collect();
        if let Some(c) = self.scroll {
            layer = layer.scrolling(c);
        }
        layer.scroll_follow = self.follow;
        layer
    }
}

/// A layer whose version moves only when its list changed since the last frame.
fn finish(state: &mut FrameState, id: LayerId, bounds: Rect, list: DisplayList) -> Layer {
    let (content, version) = match state.lists.get(&id) {
        Some((old, v)) if **old == list => (old.clone(), *v),
        Some((_, v)) => (Arc::new(list), v + 1),
        None => (Arc::new(list), 0),
    };
    state.lists.insert(id, (content.clone(), version));
    let mut layer = Layer::new(id, bounds, DisplayList::new());
    layer.content = content;
    layer.content_version = version;
    layer
}

struct Walk<'a> {
    inner: &'a Inner,
    state: &'a mut FrameState,
    rects: &'a HashMap<ElementId, Rect>,
}

impl Walk<'_> {
    /// Paints `id` (at `rect`, relative to its parent) and its subtree into `b`. `origin` is
    /// where the parent sits in `b`'s layer: a scroll container inside becomes a layer at
    /// its full offset there, while drawing nests transforms relative to each parent.
    fn element(&mut self, b: &mut Builder, id: ElementId, origin: Point) {
        let Some(d) = self.inner.els.get(&id) else { return };
        let Some(rect) = self.rects.get(&id).copied() else { return };
        let at = Point::new(origin.x + rect.min.x, origin.y + rect.min.y);
        let opacity = d.resolved.opacity;
        if opacity <= 0.0 || d.hidden {
            return;
        }
        if let Some(transform) = d.isolate.filter(|_| !d.scroll) {
            // A layer of its own: a clip at the element's box, and inside it the element's
            // paint and children in a layer the compositor transforms. The focus ring goes
            // around the box, outside the clip, in the layer below.
            b.split(self.state);
            let (outer, inner) = isolate_ids(id);
            let size = rect.size();
            let mut ob = Builder::new(outer, Rect::from_origin_size(at, size));
            let mut ib = Builder::new(inner, Rect::from_origin_size(Point::ZERO, size));
            self.body(&mut ib, id, Rect::from_origin_size(Point::ZERO, size), Point::ZERO, false);
            let mut layer = ib.finish_into(self.state);
            layer.transform = transform;
            ob.children.push(layer);
            let radii = CornerRadii::all(d.resolved.radius);
            b.children.push(ob.finish_into(self.state).clipped(radii));
            b.push(Item::PushTransform(Affine::translate(rect.min.to_vec2())));
            self.focus_ring(b, id, Rect::from_origin_size(Point::ZERO, size));
            b.pop(Item::PopTransform);
            return;
        }
        self.body(b, id, rect, at, true);
    }

    /// The focus ring of `id` around `local`, when keyboard focus shows there.
    fn focus_ring(&mut self, b: &mut Builder, id: ElementId, local: Rect) {
        let Some(d) = self.inner.els.get(&id) else { return };
        let s = &d.resolved;
        if d.states.contains(States::FOCUS_VISIBLE) && s.shows_focus_ring() {
            let grow = s.outline_offset + s.outline_width;
            let ring = local.deflate(Insets::all(-grow));
            let radius = if s.radius > 0.0 { (s.radius + grow).max(0.0) } else { 0.0 };
            b.list.border(ring, CornerRadii::all(radius), s.outline_width, s.outline);
        }
    }

    /// Paints `id` at `rect` (in its parent's space; `at` in `b`'s layer) and its subtree
    /// into `b`, with its focus ring when `ring`.
    fn body(&mut self, b: &mut Builder, id: ElementId, rect: Rect, at: Point, ring: bool) {
        let Some(d) = self.inner.els.get(&id) else { return };
        let opacity = d.resolved.opacity;
        b.push(Item::PushTransform(Affine::translate(rect.min.to_vec2())));
        if opacity < 1.0 {
            b.push(Item::PushLayer { opacity, blend: remedy_ui_paint::Blend::Normal });
        }
        if let Some((_, chunk)) = &d.chunk {
            b.list.extend(chunk);
        }
        let kids = self.inner.tree.children(id);
        let local = Rect::from_origin_size(Point::ZERO, rect.size());
        if d.scroll {
            // The viewport becomes a scroll layer; its content scrolls on the compositor.
            b.split(self.state);
            let (viewport, content_id) = scroll_ids(id);
            let content = kids.iter().filter_map(|k| self.rects.get(k)).fold(local, |u, r| u.union(*r));
            let width = if d.scroll_x { content.max.x.max(rect.width()) } else { rect.width() };
            let content_size = Size::new(width, content.max.y.max(rect.height()));
            let mut vb = Builder::new(viewport, Rect::from_origin_size(at, rect.size()));
            vb.scroll = Some(content_size);
            vb.follow = d.scroll_follow.map(|(leader, x, y)| remedy_ui_compositor::ScrollFollow {
                leader: scroll_ids(leader).0,
                x,
                y,
            });
            let mut cb = Builder::new(content_id, Rect::from_origin_size(Point::ZERO, content_size));
            for k in kids {
                self.element(&mut cb, k, Point::ZERO);
            }
            vb.children.push(cb.finish_into(self.state));
            let layer = vb.finish_into(self.state);
            b.children.push(layer);
            self.state.scrollers.push((viewport, id));
        } else {
            if d.clip {
                b.push(Item::PushClip { rect: local, radii: CornerRadii::all(d.resolved.radius) });
            }
            for k in kids {
                self.element(b, k, at);
            }
            if d.clip {
                b.pop(Item::PopClip);
            }
        }
        // Something dragged over it would be taken: the theme's accent outline, inside.
        if d.states.contains(States::DROP_TARGET) {
            let w = 2.0;
            let radius = d.resolved.radius;
            b.list.border(local, CornerRadii::all(radius), w, self.inner.theme.colors.accent);
        }
        // The focus ring goes over the element and its children, so nothing inside hides it.
        if ring {
            self.focus_ring(b, id, local);
        }
        if opacity < 1.0 {
            b.pop(Item::PopLayer);
        }
        b.pop(Item::PopTransform);
    }
}

impl Ui {
    /// Runs the pipeline for a window of `size` logical pixels at `scale` device pixels per
    /// logical pixel.
    pub fn frame(&self, size: Size, scale: f32) -> Frame {
        // Before this frame lays out, and outside the borrow below: what follows the window's
        // size (overlays keeping inside it) reacts now and is laid out in this frame.
        let window_size = self.0.borrow().window_size;
        if window_size.get_untracked().is_ok_and(|s| s != size) {
            let _ = window_size.try_set(size);
        }
        let mut guard = self.0.borrow_mut();
        let i = &mut *guard;
        let root = i.tree.root();
        if i.size != size || i.scale != scale {
            if i.scale != scale {
                for d in i.els.values() {
                    if let Some(t) = &d.text {
                        let mut t = t.borrow_mut();
                        t.scale = scale;
                        t.cache = None;
                    }
                    if let Some(f) = &d.field {
                        f.borrow_mut().invalidate();
                    }
                }
                let ids: Vec<ElementId> = i.els.keys().copied().collect();
                for id in ids {
                    i.tree.mark(id, Dirty::LAYOUT | Dirty::PAINT);
                }
            }
            i.size = size;
            i.scale = scale;
            i.a11y.set_scale(scale);
            i.tree.mark(root, Dirty::LAYOUT);
        }

        // Style.
        let theme = i.theme.clone();
        let base = i.base.clone();
        // Parents before children: text properties inherit, so a child resolves against its
        // parent's new style, and a parent whose inherited values changed restyles its
        // children.
        let depth = |tree: &remedy_ui_tree::Tree, id: ElementId| {
            let (mut n, mut cur) = (0usize, tree.parent(id));
            while let Some(p) = cur {
                n += 1;
                cur = tree.parent(p);
            }
            n
        };
        let mut queue: std::collections::BTreeMap<(usize, u64), ElementId> =
            std::collections::BTreeMap::new();
        for id in i.tree.take_dirty(Dirty::STYLE) {
            queue.insert((depth(&i.tree, id), root_key(id)), id);
        }
        while let Some(((level, _), id)) = queue.pop_first() {
            let Some(d) = i.els.get(&id) else { continue };
            let key = StyleKey { widget: d.widget, variant: d.variant, states: d.states };
            let inherited = match i.tree.parent(id).and_then(|p| i.els.get(&p)) {
                Some(parent) => parent.resolved.inherited_by_children(&base),
                None => base.clone(),
            };
            let resolved = i.sheet.resolve(&key, &inherited, &d.local);
            let Some(d) = i.els.get_mut(&id) else { continue };
            let passes_on = resolved.foreground != d.resolved.foreground
                || resolved.text_size != d.resolved.text_size
                || resolved.weight != d.resolved.weight
                || resolved.mono != d.resolved.mono;
            let text_changed = resolved.text_size != d.resolved.text_size
                || resolved.weight != d.resolved.weight
                || resolved.mono != d.resolved.mono
                || resolved.foreground != d.resolved.foreground;
            let layout_changed = resolved.padding != d.resolved.padding
                || resolved.mono != d.resolved.mono
                || resolved.text_size != d.resolved.text_size
                || resolved.weight != d.resolved.weight
                || resolved.border_width != d.resolved.border_width;
            let changed = resolved != d.resolved;
            d.resolved = resolved;
            if text_changed {
                let style = text_style_for(d, &theme);
                if let Some(t) = &d.text {
                    let mut t = t.borrow_mut();
                    t.style = style.clone();
                    t.cache = None;
                }
                if let Some(f) = &d.field {
                    f.borrow_mut().set_style(style, &theme);
                }
            }
            let node = d.node;
            if layout_changed {
                let _ = i.layout.mark_dirty(node);
                i.tree.mark(id, Dirty::LAYOUT);
            }
            if changed {
                i.tree.mark(id, Dirty::PAINT);
            }
            if passes_on {
                for c in i.tree.children(id) {
                    queue.insert((level + 1, root_key(c)), c);
                }
            }
        }

        // Layout.
        let layout_dirty = i.tree.take_dirty(Dirty::LAYOUT | Dirty::CHILD_LAYOUT);
        if !layout_dirty.is_empty() {
            for id in &layout_dirty {
                let Some(d) = i.els.get(id) else { continue };
                let mut style = d.layout.clone();
                // Hidden elements take no space, as the `hidden` attribute does on the web.
                if d.hidden {
                    style.display = remedy_ui_layout::Display::None;
                }
                // A scroll container's automatic minimum size is zero (as in CSS): it scrolls
                // its content instead of growing to fit it.
                if d.scroll {
                    if style.min_height == remedy_ui_layout::Length::Auto {
                        style.min_height = remedy_ui_layout::Length::Px(0.0);
                    }
                    if style.min_width == remedy_ui_layout::Length::Auto {
                        style.min_width = remedy_ui_layout::Length::Px(0.0);
                    }
                }
                // Content in a scroll container keeps its size and scrolls, never shrinks.
                if i.tree.parent(*id).and_then(|p| i.els.get(&p)).is_some_and(|p| p.scroll) {
                    style.shrink = 0.0;
                }
                let mut pad = d.resolved.padding;
                if *id == root {
                    // The window root keeps its content clear of system interface. It lays
                    // out left to right whatever the content's direction: start is left.
                    let s = i.safe_area.get_untracked().unwrap_or_default();
                    pad.top += s.top;
                    pad.end += s.right;
                    pad.bottom += s.bottom;
                    pad.start += s.left;
                }
                let bw = d.resolved.border_width;
                style.padding = Insets {
                    top: pad.top + bw,
                    end: pad.end + bw,
                    bottom: pad.bottom + bw,
                    start: pad.start + bw,
                };
                let node = d.node;
                let _ = i.layout.set_style(node, &style);
                let _ = i.layout.mark_dirty(node);
            }
            if let Some(rn) = i.els.get(&root).map(|d| d.node) {
                let _ = i.layout.compute(rn, Some(size.width), Some(size.height));
            }
            i.frame.bounds_stale = true;
        }

        // Bounds: window coordinates for hit testing, local rects for painting.
        let mut rects: HashMap<ElementId, Rect> = HashMap::new();
        let mut stack = vec![(root, Point::ZERO, Vec2::ZERO)];
        while let Some((id, parent_at, scroll)) = stack.pop() {
            let Some(d) = i.els.get(&id) else { continue };
            let mut r = i.layout.rect(d.node).unwrap_or(Rect::ZERO);
            if id == root {
                r = Rect::from_origin_size(Point::ZERO, size);
            }
            // A link inside text sits over its words, wherever the text put them.
            if d.inline_link.is_some() {
                let parent = i.tree.parent(id);
                let width = parent.and_then(|p| rects.get(&p)).map_or(0.0, |p| p.width());
                r = parent.and_then(|p| crate::links::placed_rect(i, id, p, width)).unwrap_or(Rect::ZERO);
            }
            rects.insert(id, r);
            let abs = Point::new(parent_at.x + r.min.x - scroll.x, parent_at.y + r.min.y - scroll.y);
            if i.frame.bounds_stale {
                let _ = i.tree.set_bounds(id, Rect::from_origin_size(abs, r.size()));
            }
            let child_scroll =
                if d.scroll { i.scroll_offsets.get(&id).copied().unwrap_or(Vec2::ZERO) } else { Vec2::ZERO };
            for k in i.tree.children(id).into_iter().rev() {
                stack.push((k, abs, child_scroll));
            }
        }
        let moved = i.frame.bounds_stale;
        i.frame.bounds_stale = false;
        // Elements watching their size hear about changes once the frame is done.
        let mut resized: Vec<(ResizeHandler, Size)> = Vec::new();
        for (id, d) in i.els.iter_mut() {
            if let (Some(cb), Some(r)) = (&d.on_resize, rects.get(id))
                && d.last_size != Some(r.size())
            {
                d.last_size = Some(r.size());
                resized.push((cb.clone(), r.size()));
            }
        }

        // Elements watching whether they are seen hear about changes the same way.
        let mut seen: Vec<(VisibilityHandler, bool)> = Vec::new();
        let window = Rect::from_origin_size(Point::ZERO, size);
        let watchers: Vec<ElementId> =
            i.els.iter().filter(|(_, d)| d.on_visibility.is_some()).map(|(id, _)| *id).collect();
        for id in watchers {
            let now = is_seen(i, id, window);
            if let Some(d) = i.els.get_mut(&id)
                && d.last_visible != Some(now)
                && let Some(cb) = d.on_visibility.clone()
            {
                d.last_visible = Some(now);
                seen.push((cb, now));
            }
        }

        // Paint chunks: re-record what is dirty or changed size.
        let mut repaint = i.tree.take_dirty(Dirty::PAINT | Dirty::CHILD_PAINT);
        let mut painted = false;
        let focused = i.tree.focused();
        let engine = i.engine.clone();
        let mut engine = engine.borrow_mut();
        repaint.retain(|id| i.tree.dirty(*id).is_empty() || true);
        repaint.extend(
            i.els
                .iter()
                .filter(|(id, d)| {
                    let sz = rects.get(id).map(|r| r.size()).unwrap_or(Size::ZERO);
                    d.chunk.as_ref().is_none_or(|(s, _)| *s != sz)
                })
                .map(|(id, _)| *id),
        );
        repaint.sort_unstable_by_key(|id| root_key(*id));
        repaint.dedup();
        for id in repaint {
            let sz = rects.get(&id).map(|r| r.size()).unwrap_or(Size::ZERO);
            let Some(d) = i.els.get(&id) else { continue };
            let rtl = i.layout.is_rtl(d.node);
            let chunk = record(d, rtl, sz, &theme, &mut engine, focused == Some(id), i.scale);
            if let Some(d) = i.els.get_mut(&id) {
                d.chunk = Some((sz, Rc::new(chunk)));
            }
            painted = true;
        }
        drop(engine);

        // Layers.
        let transformed = std::mem::take(&mut i.layers_changed);
        let layers = (painted || moved || transformed).then(|| {
            let mut state = std::mem::take(&mut i.frame);
            state.scrollers.clear();
            let mut b = Builder::new(LayerId(0), Rect::from_origin_size(Point::ZERO, size));
            let mut walk = Walk { inner: i, state: &mut state, rects: &rects };
            walk.element(&mut b, root, Point::ZERO);
            let layer = b.finish_into(&mut state);
            let live: std::collections::HashSet<LayerId> = collect_ids(&layer);
            state.lists.retain(|k, _| live.contains(k));
            i.frame = state;
            LayerTree { roots: vec![Arc::new(layer)] }
        });

        // Semantics.
        let sem_dirty = !i.tree.take_dirty(Dirty::SEMANTICS).is_empty();
        if sem_dirty || i.structure_changed || moved {
            i.structure_changed = false;
            sync_semantics(i, &rects);
        }
        let scrollers = i.frame.scrollers.clone();
        drop(guard);
        for (cb, size) in resized {
            cb(size);
        }
        for (cb, now) in seen {
            cb(now);
        }
        Frame { layers, scrollers }
    }
}

/// True when some of `id` shows: it and its ancestors are not hidden or transparent, the window
/// is not covered, and its box overlaps the window and every clipping ancestor.
fn is_seen(i: &Inner, id: ElementId, window: Rect) -> bool {
    if i.occluded {
        return false;
    }
    let Some(mut shown) = i.tree.bounds(id) else { return false };
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(d) = i.els.get(&c) else { return false };
        if d.hidden || d.resolved.opacity <= 0.0 {
            return false;
        }
        if c != id && (d.scroll || d.clip) {
            let Some(b) = i.tree.bounds(c) else { return false };
            let Some(r) = shown.intersect(b) else { return false };
            shown = r;
        }
        cur = i.tree.parent(c);
    }
    shown.intersect(window).is_some()
}

/// An element's resize callback.
type ResizeHandler = Rc<dyn Fn(Size)>;

/// An element's callback for being seen or not.
type VisibilityHandler = Rc<dyn Fn(bool)>;

fn collect_ids(l: &Layer) -> std::collections::HashSet<LayerId> {
    let mut out = std::collections::HashSet::new();
    let mut stack = vec![l];
    while let Some(l) = stack.pop() {
        out.insert(l.id);
        stack.extend(l.children.iter().map(|c| &**c));
    }
    out
}

fn sync_semantics(i: &mut Inner, _rects: &HashMap<ElementId, Rect>) {
    let root = i.tree.root();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let mut kids = i.tree.children(id);
        let Some(d) = i.els.get(&id) else { continue };
        // Hidden content leaves the accessibility tree entirely.
        kids.retain(|k| i.els.get(k).is_none_or(|kd| !kd.hidden));
        let bounds = i.tree.bounds(id).unwrap_or(Rect::ZERO);
        let s = composed_semantics(d, id == root, i.tree.is_focusable(id));
        i.a11y.set(root_key(id), s.bounds(bounds));
        let child_keys: Vec<u64> = kids.iter().map(|k| root_key(*k)).collect();
        i.a11y.set_children(root_key(id), &child_keys);
        stack.extend(kids);
    }
    if let Some(f) = i.tree.focused() {
        i.a11y.set_focus(root_key(f));
    }
}
