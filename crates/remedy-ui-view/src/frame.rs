//! The frame pipeline: style, layout, bounds, paint chunks, layers and semantics, each
//! redone only for what changed.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_compositor::{Layer, LayerId, LayerTree, ScrollSnap, SnapPoint};
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

/// What one frame did, for the performance overlay and tests ([`Ui::last_frame_stats`]).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FrameStats {
    /// Elements whose style was resolved again.
    pub restyled: usize,
    /// Elements laid out again (the dirty ones; their subtrees follow inside layout).
    pub laid_out: usize,
    /// Elements whose paint chunks were recorded again, in id order.
    pub repainted: Vec<ElementId>,
    /// The layers were assembled again.
    pub layers: bool,
    /// The accessibility tree was brought up to date.
    pub semantics: bool,
}

#[derive(Default)]
pub(crate) struct FrameState {
    lists: HashMap<LayerId, (Arc<DisplayList>, u64)>,
    pub(crate) bounds_stale: bool,
    scrollers: Vec<(LayerId, ElementId)>,
    stickies: HashMap<ElementId, remedy_ui_compositor::Sticky>,
}

/// What a sticky element sticks within, for the bounds walk: its nearest scroll container
/// (the window when none), and its parent's box.
#[derive(Clone, Copy)]
struct StickCx {
    scroller: Option<LayerId>,
    viewport: Size,
    /// Window coordinates to the container's content space.
    to_content: Vec2,
    /// The container's scroll offset.
    offset: Vec2,
    /// The parent's content box, in window coordinates.
    within: Rect,
}

/// The physical sides an element sticks to (start and end following its parent's direction,
/// right to left when `parent_rtl` says so), from its style sheet or its layout; `None` when
/// it is not sticky.
fn sticky_sides(d: &ElData, parent_rtl: impl FnOnce() -> bool) -> Option<remedy_ui_style::StickyInsets> {
    let side = |v: f32| v.is_finite().then_some(v);
    let sheet = d.resolved.sticky.sticks();
    let s = if sheet {
        d.resolved.sticky
    } else if d.layout.position == remedy_ui_layout::Position::Sticky {
        let i = d.layout.inset;
        remedy_ui_style::StickyInsets {
            top: side(i.top),
            end: side(i.end),
            bottom: side(i.bottom),
            start: side(i.start),
        }
    } else {
        return None;
    };
    let physical = d.layout.inset_physical && !sheet;
    Some(if !physical && parent_rtl() {
        remedy_ui_style::StickyInsets { start: s.end, end: s.start, ..s }
    } else {
        s
    })
}

/// The layer that moves a sticky element.
fn sticky_id(id: ElementId) -> LayerId {
    LayerId(root_key(id).wrapping_mul(4).wrapping_add(3) | (1 << 61))
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

/// The brush that draws gradient `g` over a box `size` large at the origin, its theme roles
/// resolved under `theme`: what an element's `background_layers` paint.
///
/// ```
/// use remedy_ui_core::Size;
/// use remedy_ui_paint::Brush;
/// use remedy_ui_style::{Gradient, Theme};
///
/// let g = Gradient::parse("radial-gradient(ellipse at center, #fff, transparent)").unwrap();
/// let b = remedy_ui_view::gradient_brush(&g, Size::new(200.0, 100.0), &Theme::dark_forest());
/// assert!(matches!(b, Brush::Elliptical { .. }));
/// ```
pub fn gradient_brush(g: &remedy_ui_style::Gradient, size: Size, theme: &remedy_ui_style::Theme) -> Brush {
    use remedy_ui_style::GradientGeometry as G;
    let stops: Vec<_> =
        g.stops(theme).into_iter().map(|(offset, color)| remedy_ui_paint::Stop { offset, color }).collect();
    match g.geometry(size) {
        G::Linear { start, end } => Brush::Linear { start, end, stops },
        G::Radial { center, radii } if radii.x == radii.y => Brush::Radial { center, radius: radii.x, stops },
        G::Radial { center, radii } => Brush::Elliptical { center, radii, stops },
        G::Conic { center, start_angle } => Brush::Conic { center, start_angle, stops },
        // GradientGeometry is non_exhaustive; a kind this view does not know draws its first color.
        _ => Brush::Solid(g.stops(theme).first().map_or(Color::TRANSPARENT, |s| s.1)),
    }
}

pub(crate) fn text_style_for(d: &ElData, theme: &remedy_ui_style::Theme) -> remedy_ui_text::TextStyle {
    {
        let mut v_ = remedy_ui_text::TextStyle::default();
        v_.families = theme.families(d.resolved.mono, d.resolved.heading);
        v_.letter_spacing = d.resolved.letter_spacing.px(d.resolved.text_size);
        v_.transform = d.resolved.text_transform;
        v_.size = d.resolved.text_size;
        v_.weight = d.resolved.weight;
        v_.color = d.resolved.foreground;
        v_
    }
}

/// What recording an element's paint gave: the list, whether its painter asked for the next
/// frame, and why its painter failed, if it did.
type Recorded = (DisplayList, bool, Option<String>, crate::ambient::PaintAsk);

#[allow(clippy::too_many_arguments, reason = "the frame's state a chunk is recorded with")]
fn record(
    d: &ElData,
    rtl: bool,
    size: Size,
    shown: Option<std::ops::Range<f32>>,
    theme: &remedy_ui_style::Theme,
    engine: &mut remedy_ui_text::TextEngine,
    focused: bool,
    scale: f32,
    time: std::time::Duration,
    reduced_motion: bool,
    ambient_rate: Option<crate::AmbientRate>,
) -> Recorded {
    let mut ask = crate::ambient::PaintAsk::default();
    let mut failed = None;
    let mut next_frame = false;
    let mut list = DisplayList::new();
    let s = &d.resolved;
    let rect = Rect::from_origin_size(Point::ZERO, size);
    let radii = CornerRadii::all(s.radius);
    if s.background.a > 0.0 {
        list.fill_rounded(rect, radii, Brush::Solid(s.background));
    }
    // The last layer is the lowest, as CSS stacks `background-image`.
    for g in s.background_layers.iter().rev() {
        list.fill_rounded(rect, radii, gradient_brush(g, size, theme));
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
        // A huge paragraph laid out lazily draws the lines that show and a window's height
        // more on each side; scrolling out of them paints again ([`lazy_text_moved`]).
        let band = p.is_lazy().then(|| {
            let shown = shown.unwrap_or(0.0..0.0);
            let (a, b) = (shown.start - content.min.y, shown.end - content.min.y);
            let margin = (b - a).max(600.0);
            (a - margin)..(b + margin)
        });
        slot.painted = band.clone().map(|b| (b, p.height()));
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
        match &band {
            Some(band) => glyphs_in(&mut list, &p, Point::new(x, content.min.y), s.foreground, band.clone()),
            None => glyphs(&mut list, &p, Point::new(x, content.min.y), s.foreground),
        }
        // Pictures in the text, in their objects' boxes.
        if !d.inline_images.is_empty() {
            let objects = match &band {
                Some(band) => p.inline_objects_in(band.clone()),
                None => p.inline_objects(),
            };
            for (id, r) in objects {
                if let Some(img) = d.inline_images.iter().find(|i| i.id == id) {
                    list.push(remedy_ui_paint::Item::Image {
                        image: img.image.clone(),
                        rect: r.translate(remedy_ui_core::Vec2::new(x, content.min.y)),
                        radii: CornerRadii::ZERO,
                        sampling: remedy_ui_paint::Sampling::Linear,
                    });
                }
            }
        }
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
        let mut cx = PaintCx {
            list: &mut list,
            size,
            style: s,
            theme,
            states: d.states,
            scale,
            next_frame: false,
            rtl,
            time,
            reduced_motion,
            engine,
            ambient_rate,
            last_tick: d.ambient_last,
            ask: crate::ambient::PaintAsk::default(),
        };
        // A painter that panics is contained: its element is reported to its error
        // boundary after the frame, and the frame goes on without its drawing.
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tracked = d.paint_watch.as_ref().is_some_and(|w| w.track(|| painter(&mut cx)).is_ok());
            if !tracked {
                painter(&mut cx);
            }
        }));
        match ran {
            Ok(()) => {
                next_frame = cx.next_frame;
                ask = cx.ask;
            }
            Err(p) => failed = Some(remedy_ui_reactive::panic_message(p.as_ref())),
        }
    }
    (list, next_frame, failed, ask)
}

/// Adds a paragraph's glyph runs at `origin`. Run colors come from the paragraph's spans;
/// `default` is the element's text color.
/// What an element tells assistive technology: its own semantics, or what its text or field
/// implies, plus disabled, click and focus from its state.
pub(crate) fn composed_semantics(
    d: &ElData,
    is_root: bool,
    focusable: bool,
    pseudo: Option<remedy_ui_text::Pseudo>,
) -> Semantics {
    let mut s = match &d.semantics {
        // What the app names an element is a string it translates (unless it said the
        // element's text is data), so a pseudo-locale rewrites it for readers too.
        Some(s) => match pseudo.filter(|_| !d.data_text) {
            Some(p) => s.clone().map_text(|t| p.apply_str(t)),
            None => s.clone(),
        },
        None if is_root => Semantics::new(Role::Window),
        None => match (&d.text, &d.field) {
            (Some(t), _) => Semantics::new(Role::Label).label(crate::element::with_alts(
                &t.borrow().shown(),
                &d.inline_images,
                None,
            )),
            (None, Some(f)) => match pseudo {
                // The field's label is the app's; its value is what people typed.
                Some(p) => f.borrow().semantics().map_text(|t| p.apply_str(t)),
                None => f.borrow().semantics(),
            },
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

thread_local! {
    /// The hinting and edges glyphs are drawn with in the frame being built.
    static GLYPH_STYLE: std::cell::Cell<(Hinting, Antialias)> = const { std::cell::Cell::new((Hinting::Full, Antialias::Grayscale)) };
}

pub(crate) fn glyphs(list: &mut DisplayList, p: &Paragraph, origin: Point, default: Color) {
    draw_runs(list, p.glyph_runs(), origin, default);
}

/// Draws the lines of `p` that reach into the band `ys` of its own y: for a huge paragraph
/// laid out lazily, the part that shows.
pub(crate) fn glyphs_in(
    list: &mut DisplayList,
    p: &Paragraph,
    origin: Point,
    default: Color,
    ys: std::ops::Range<f32>,
) {
    draw_runs(list, p.glyph_runs_in(ys), origin, default);
}

fn draw_runs(list: &mut DisplayList, runs: Vec<remedy_ui_text::GlyphRunInfo>, origin: Point, default: Color) {
    let (hinting, antialias) = GLYPH_STYLE.with(std::cell::Cell::get);
    for run in runs {
        let color = if run.color == Color::BLACK { default } else { run.color };
        for bar in [run.underline, run.strikethrough].into_iter().flatten() {
            list.fill_rect(bar.translate(origin.to_vec2()), Brush::Solid(color));
        }
        list.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(origin.x + g.x, origin.y + g.y))).collect(),
            hinting,
            antialias,
        }));
    }
}

struct Builder {
    id: LayerId,
    bounds: Rect,
    scroll: Option<Size>,
    origin: Vec2,
    follow: Option<remedy_ui_compositor::ScrollFollow>,
    snap: Option<ScrollSnap>,
    list: DisplayList,
    /// Push items open in `list`, re-opened when a segment starts.
    open: Vec<Item>,
    children: Vec<Layer>,
    /// Sticky elements' layers: drawn over the rest, as CSS paints positioned boxes after
    /// the flow, so content scrolling under a stuck header stays under it.
    stuck: Vec<Layer>,
    segments: u64,
}

impl Builder {
    fn new(id: LayerId, bounds: Rect) -> Self {
        Builder {
            id,
            bounds,
            scroll: None,
            origin: Vec2::ZERO,
            follow: None,
            snap: None,
            list: DisplayList::new(),
            open: Vec::new(),
            children: Vec::new(),
            stuck: Vec::new(),
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
                Item::PushFilter(_) => Item::PopFilter,
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
                Item::PushFilter(_) => Item::PopFilter,
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
        self.children.append(&mut self.stuck);
        layer.children = self.children.into_iter().map(Arc::new).collect();
        if let Some(c) = self.scroll {
            layer = layer.scrolling(c);
            layer.scroll_origin = self.origin;
        }
        layer.scroll_follow = self.follow;
        layer.scroll_snap = self.snap;
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
    stickies: &'a HashMap<ElementId, remedy_ui_compositor::Sticky>,
    /// [`Walk::layers_below`] answers this frame, so nested filtered elements each walk a
    /// subtree once, not once per filtered ancestor.
    layered: HashMap<ElementId, bool>,
}

impl Walk<'_> {
    /// Paints `id` (at `rect`, relative to its parent) and its subtree into `b`. `origin` is
    /// where the parent sits in `b`'s layer: a scroll container inside becomes a layer at
    /// its full offset there, while drawing nests transforms relative to each parent.
    fn element(&mut self, b: &mut Builder, id: ElementId, origin: Point) {
        crate::deep::deeper(|| self.element_here(b, id, origin));
    }

    /// [`Walk::element`], on this stack.
    fn element_here(&mut self, b: &mut Builder, id: ElementId, origin: Point) {
        if let Some(st) = self.stickies.get(&id).copied() {
            // A layer the compositor moves as the container scrolls, holding all the element
            // draws (its shadow and focus ring too), in `b`'s space and with what is open there.
            let mut sb = Builder::new(sticky_id(id), Rect::from_origin_size(Point::ZERO, b.bounds.size()));
            for item in b.open.clone() {
                sb.push(item);
            }
            self.element_placed(&mut sb, id, origin);
            let mut layer = sb.finish_into(self.state);
            layer.sticky = Some(st);
            b.stuck.push(layer);
            return;
        }
        self.element_placed(b, id, origin);
    }

    /// [`Walk::element_here`] for an element drawn where layout put it.
    fn element_placed(&mut self, b: &mut Builder, id: ElementId, origin: Point) {
        let Some(d) = self.inner.els.get(&id) else { return };
        let Some(rect) = self.rects.get(&id).copied() else { return };
        let at = Point::new(origin.x + rect.min.x, origin.y + rect.min.y);
        let opacity = d.resolved.opacity;
        if opacity <= 0.0 || d.hidden {
            return;
        }
        // Frosted glass needs a layer of its own: the compositor blurs what lies under it.
        let glass = d.resolved.backdrop_blur;
        // So does a filter over children that are layers of their own: the compositor draws
        // the whole subtree through it.
        let own_filters = filters(&d.resolved);
        let layer_filters = !d.scroll && !own_filters.is_empty() && self.layers_below(id);
        let isolate = d
            .isolate
            .or(d.ambient_layer.then_some(Affine::IDENTITY))
            .or((glass > 0.0 || layer_filters).then_some(Affine::IDENTITY));
        if let Some(transform) = isolate.filter(|_| !d.scroll) {
            // A layer of its own: a clip at the element's box, and inside it the element's
            // paint and children in a layer the compositor transforms. The box's shadow and
            // the focus ring go around the box, outside the clip, in the layer below.
            // Filtered, the shadow is filtered with the rest, inside the layer.
            if !layer_filters && let Some(shadow) = box_shadow(&d.resolved, rect.size(), opacity) {
                b.push(Item::PushTransform(Affine::translate(rect.min.to_vec2())));
                b.list.push(shadow);
                b.pop(Item::PopTransform);
            }
            b.split(self.state);
            let (outer, inner) = isolate_ids(id);
            let size = rect.size();
            let mut ob = Builder::new(outer, Rect::from_origin_size(at, size));
            let mut ib = Builder::new(inner, Rect::from_origin_size(Point::ZERO, size));
            // Glass fades with its element: the opacity goes on the layer holding the glass, so the
            // compositor fades the blurred backdrop and the content as one.
            let faded = glass > 0.0 && opacity < 1.0;
            let paint =
                Paint { ring: false, shadow: layer_filters, opacity: !faded, filters: !layer_filters };
            self.body(&mut ib, id, Rect::from_origin_size(Point::ZERO, size), Point::ZERO, paint);
            let mut layer = ib.finish_into(self.state);
            layer.transform = transform;
            layer.ambient = d.loops.clone();
            ob.children.push(layer);
            let radii = CornerRadii::all(d.resolved.radius);
            let mut outer = ob.finish_into(self.state);
            // A filtered element draws past its box (a blur, a shadow): only an element that
            // clips its children, or one the compositor moves, keeps the clip at its box.
            if layer_filters {
                if d.clip || d.isolate.is_some() || d.ambient_layer {
                    outer = outer.clipped(radii);
                }
                outer = outer.with_filters(own_filters);
            } else {
                outer = outer.clipped(radii);
            }
            if glass > 0.0 {
                outer = outer.with_backdrop(glass, radii);
            }
            if faded {
                outer.opacity = opacity;
            }
            b.children.push(outer);
            b.push(Item::PushTransform(Affine::translate(rect.min.to_vec2())));
            self.focus_ring(b, id, Rect::from_origin_size(Point::ZERO, size));
            b.pop(Item::PopTransform);
            return;
        }
        let (offset, s) = d.shift;
        if offset != Vec2::ZERO || s != 1.0 {
            // Animated offset and scale (about the center) move its drawing, not its layout.
            let c = Vec2::new(at.x + rect.width() / 2.0, at.y + rect.height() / 2.0);
            let t = Affine::translate(Vec2::new(-c.x, -c.y))
                .then(Affine::scale(s, s))
                .then(Affine::translate(Vec2::new(c.x + offset.x, c.y + offset.y)));
            b.push(Item::PushTransform(t));
            self.body(b, id, rect, at, Paint::ALL);
            b.pop(Item::PopTransform);
            return;
        }
        self.body(b, id, rect, at, Paint::ALL);
    }

    /// True when something under `id` is drawn in a compositor layer of its own (a scroll
    /// container, an isolated or ambient element, glass, or a filtered element over such).
    fn layers_below(&mut self, id: ElementId) -> bool {
        if let Some(&known) = self.layered.get(&id) {
            return known;
        }
        let inner = self.inner;
        let found = inner.tree.children(id).into_iter().any(|k| {
            inner.els.get(&k).is_some_and(|d| {
                !d.hidden
                    && (d.scroll
                        || d.isolate.is_some()
                        || d.ambient_layer
                        || d.resolved.backdrop_blur > 0.0
                        || crate::deep::deeper(|| self.layers_below(k)))
            })
        });
        self.layered.insert(id, found);
        found
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

    /// The snap points of a snapping scroll container (`id`, a `view`-sized viewport with its
    /// content shifted by `lead`): one for each descendant with
    /// [`snap_align`](crate::El::snap_align), not looking inside nested scroll containers.
    fn snap_points(
        &self,
        id: ElementId,
        spec: &crate::element::SnapSpec,
        view: Size,
        lead: f32,
    ) -> ScrollSnap {
        let mut snap = ScrollSnap { x: Vec::new(), y: Vec::new(), ..spec.tuning.clone() };
        let Some(axis) = spec.axis else { return snap };
        let rtl = self.inner.els.get(&id).is_some_and(|d| self.inner.layout.is_rtl(d.node));
        // Physical left and right of logical insets: start is on the right right to left.
        let lr = |i: Insets| if rtl { (i.end, i.start) } else { (i.start, i.end) };
        let (pad_l, pad_r) = lr(spec.padding);
        // The offset that lines up an area (`min`..`max`, grown by margins `m0`, `m1`) with the
        // padded viewport (`p0`, `p1` from each edge, `extent` long).
        let at = |a: crate::SnapAlign,
                  min: f32,
                  max: f32,
                  (m0, m1): (f32, f32),
                  (p0, p1): (f32, f32),
                  extent: f32| {
            let (min, max) = (min - m0, max + m1);
            match a {
                crate::SnapAlign::Start => min - p0,
                crate::SnapAlign::Center => (min + max) / 2.0 - (p0 + extent - p1) / 2.0,
                crate::SnapAlign::End => max + p1 - extent,
            }
        };
        // Start and end of the inline axis swap right to left.
        let flip = |a: crate::SnapAlign| match a {
            crate::SnapAlign::Start if rtl => crate::SnapAlign::End,
            crate::SnapAlign::End if rtl => crate::SnapAlign::Start,
            a => a,
        };
        let mut stack: Vec<(ElementId, Vec2)> =
            self.inner.tree.children(id).into_iter().rev().map(|k| (k, Vec2::new(lead, 0.0))).collect();
        while let Some((k, shift)) = stack.pop() {
            let (Some(d), Some(r)) = (self.inner.els.get(&k), self.rects.get(&k)) else { continue };
            if d.hidden {
                continue;
            }
            let r = r.translate(shift);
            if let Some(a) = d.snap_align {
                let key = root_key(k);
                let point = |offset| SnapPoint { offset, key, always: d.snap_stop };
                if axis != crate::SnapAxis::Y {
                    let m = lr(d.snap_margin);
                    snap.x.push(point(at(flip(a), r.min.x, r.max.x, m, (pad_l, pad_r), view.width)));
                }
                if axis != crate::SnapAxis::X {
                    let (m, p) = (d.snap_margin, spec.padding);
                    snap.y.push(point(at(
                        a,
                        r.min.y,
                        r.max.y,
                        (m.top, m.bottom),
                        (p.top, p.bottom),
                        view.height,
                    )));
                }
            }
            if !d.scroll {
                let inner = Vec2::new(r.min.x, r.min.y);
                stack.extend(self.inner.tree.children(k).into_iter().rev().map(|c| (c, inner)));
            }
        }
        snap
    }

    /// Paints `id` at `rect` (in its parent's space; `at` in `b`'s layer) and its subtree
    /// into `b`, with what `paint` says.
    fn body(&mut self, b: &mut Builder, id: ElementId, rect: Rect, at: Point, paint: Paint) {
        let Paint { ring, shadow, opacity: own_opacity, filters: own_filters } = paint;
        let Some(d) = self.inner.els.get(&id) else { return };
        let opacity = if own_opacity { d.resolved.opacity } else { 1.0 };
        // A scroll container with frosted glass: its own drawing goes in the viewport layer,
        // over the glass the compositor puts under that layer, and that layer fades it all.
        let glass_scroll = d.scroll && d.resolved.backdrop_blur > 0.0;
        let opacity = if glass_scroll { 1.0 } else { opacity };
        b.push(Item::PushTransform(Affine::translate(rect.min.to_vec2())));
        if opacity < 1.0 {
            b.push(Item::PushLayer { opacity, blend: remedy_ui_paint::Blend::Normal });
        }
        // Filters take the element's drawing, its box shadow and its children (those drawn in
        // its layer) as one picture: a drop shadow of the shape, then a blur.
        let filters = if own_filters { filters(&d.resolved) } else { Vec::new() };
        for f in &filters {
            b.push(Item::PushFilter(*f));
        }
        if shadow && let Some(s) = box_shadow(&d.resolved, rect.size(), 1.0) {
            b.list.push(s);
        }
        if let Some((_, chunk)) = &d.chunk
            && !glass_scroll
        {
            b.list.extend(chunk);
        }
        let kids = self.inner.tree.children(id);
        let local = Rect::from_origin_size(Point::ZERO, rect.size());
        if d.scroll {
            // The viewport becomes a scroll layer; its content scrolls on the compositor.
            b.split(self.state);
            let (viewport, content_id) = scroll_ids(id);
            let content = kids.iter().filter_map(|k| self.rects.get(k)).fold(local, |u, r| u.union(*r));
            let lead = d.scroll_lead;
            let width = if d.scroll_x { content.max.x.max(rect.width()) + lead } else { rect.width() };
            let content_size = Size::new(width, content.max.y.max(rect.height()));
            let mut vb = Builder::new(viewport, Rect::from_origin_size(at, rect.size()));
            if glass_scroll && let Some((_, chunk)) = &d.chunk {
                vb.list.extend(chunk);
            }
            vb.scroll = Some(content_size);
            vb.origin = Vec2::new(lead, 0.0);
            vb.follow = d.scroll_follow.map(|(leader, x, y)| remedy_ui_compositor::ScrollFollow {
                leader: scroll_ids(leader).0,
                x,
                y,
            });
            vb.snap = d.scroll_snap.as_deref().map(|spec| self.snap_points(id, spec, rect.size(), lead));
            let mut cb = Builder::new(content_id, Rect::from_origin_size(Point::ZERO, content_size));
            if lead > 0.0 {
                cb.push(Item::PushTransform(Affine::translate(Vec2::new(lead, 0.0))));
            }
            for k in kids {
                self.element(&mut cb, k, Point::new(lead, 0.0));
            }
            if lead > 0.0 {
                cb.pop(Item::PopTransform);
            }
            vb.children.push(cb.finish_into(self.state));
            let mut layer = vb.finish_into(self.state);
            if glass_scroll {
                layer = layer.with_backdrop(d.resolved.backdrop_blur, CornerRadii::all(d.resolved.radius));
                layer.opacity = if own_opacity { d.resolved.opacity } else { 1.0 };
            }
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
        for _ in &filters {
            b.pop(Item::PopFilter);
        }
        if ring {
            self.focus_ring(b, id, local);
        }
        if opacity < 1.0 {
            b.pop(Item::PopLayer);
        }
        b.pop(Item::PopTransform);
    }
}

/// What [`Walk::body`] draws besides the element and its children.
#[derive(Clone, Copy)]
struct Paint {
    /// The focus ring.
    ring: bool,
    /// The box's shadow.
    shadow: bool,
    /// The element's own opacity (off when the layer holding it fades it).
    opacity: bool,
    /// The element's filters (off when the layer holding it filters it).
    filters: bool,
}

impl Paint {
    const ALL: Paint = Paint { ring: true, shadow: true, opacity: true, filters: true };
}

/// The shadow a box styled `s` of `size` casts, faded by `opacity`, as a display item at the
/// box's origin; `None` for none.
fn box_shadow(s: &remedy_ui_style::ResolvedStyle, size: Size, opacity: f32) -> Option<Item> {
    let sh = s.shadow;
    if !sh.is_visible() || opacity <= 0.0 {
        return None;
    }
    let mut color = sh.color;
    color.a *= opacity.min(1.0);
    Some(Item::Shadow {
        rect: Rect::from_origin_size(Point::ZERO, size),
        radii: CornerRadii::all(s.radius),
        offset: sh.offset,
        blur: if sh.blur.is_finite() { sh.blur.max(0.0) } else { 0.0 },
        spread: if sh.spread.is_finite() { sh.spread } else { 0.0 },
        color,
    })
}

/// The filters an element styled `s` is drawn through, outermost first.
fn filters(s: &remedy_ui_style::ResolvedStyle) -> Vec<remedy_ui_paint::Filter> {
    let mut out = Vec::new();
    let d = s.drop_shadow;
    if d.is_visible() {
        out.push(remedy_ui_paint::Filter::DropShadow { offset: d.offset, blur: d.blur, color: d.color });
    }
    if s.blur > 0.0 {
        out.push(remedy_ui_paint::Filter::Blur { radius: s.blur });
    }
    // Innermost: the element's own colours change first, and a drop shadow keeps its colour.
    if !s.color_matrix.is_identity() {
        out.push(remedy_ui_paint::Filter::ColorMatrix { matrix: s.color_matrix });
    }
    out
}

impl Ui {
    /// Runs the pipeline for a window of `size` logical pixels at `scale` device pixels per
    /// logical pixel.
    pub fn frame(&self, size: Size, scale: f32) -> Frame {
        let style = self.glyph_style();
        GLYPH_STYLE.with(|g| g.set(style));
        // Animations move to this frame's time first, so what they changed is styled, laid
        // out and painted in this frame.
        let finished = self.tick_animations();
        for f in finished {
            f();
        }
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

        let mut stats = FrameStats::default();
        // Style.
        let style_span = remedy_ui_trace::phase!("style").entered();
        let theme = i.theme.clone();
        let reshape = std::mem::take(&mut i.reshape_text);
        let base = i.base.clone();
        // Parents before children: text properties inherit, so a child resolves against its
        // parent's new style, and a parent whose inherited values changed restyles its
        // children.
        // Each element's depth, remembered on the way up: a path to the root is walked once
        // for all the dirty elements on it, not once for each.
        let mut depths: HashMap<ElementId, usize> = HashMap::new();
        let mut depth = |tree: &remedy_ui_tree::Tree, id: ElementId| {
            let mut path = Vec::new();
            let mut cur = Some(id);
            let mut base = 0usize;
            while let Some(e) = cur {
                if let Some(&d) = depths.get(&e) {
                    base = d + 1;
                    break;
                }
                path.push(e);
                cur = tree.parent(e);
            }
            // `path` runs from `id` up to the first element whose depth was known (or the
            // root): its last element sits at `base`.
            for (k, e) in path.iter().rev().enumerate() {
                depths.insert(*e, base + k);
            }
            depths.get(&id).copied().unwrap_or(0)
        };
        let mut queue: std::collections::BTreeMap<(usize, u64), ElementId> =
            std::collections::BTreeMap::new();
        for id in i.tree.take_dirty(Dirty::STYLE) {
            queue.insert((depth(&i.tree, id), root_key(id)), id);
        }
        while let Some(((level, _), id)) = queue.pop_first() {
            let Some(d) = i.els.get(&id) else { continue };
            stats.restyled += 1;
            let key = StyleKey { widget: d.widget, variant: d.variant, states: d.states };
            let inherited = match i.tree.parent(id).and_then(|p| i.els.get(&p)) {
                Some(parent) => parent.resolved.inherited_by_children(&base),
                None => base.clone(),
            };
            // Padding the element sets only some sides of comes from the widget's own on the
            // others; animated values go over the element's own style.
            let own = if crate::tokens::partial_padding(d) {
                let mut without = d.local.clone();
                without.padding = None;
                let under = i.sheet.resolve(&key, &inherited, &without).padding;
                crate::tokens::resolve_local_over(d, &theme, under)
            } else {
                d.local.clone()
            };
            let resolved = i.sheet.resolve(&key, &inherited, &own.merge(&d.anim_style));
            let Some(d) = i.els.get_mut(&id) else { continue };
            let passes_on = resolved.foreground != d.resolved.foreground
                || resolved.text_size != d.resolved.text_size
                || resolved.weight != d.resolved.weight
                || resolved.mono != d.resolved.mono
                || resolved.heading != d.resolved.heading
                || resolved.letter_spacing != d.resolved.letter_spacing
                || resolved.text_transform != d.resolved.text_transform;
            let text_changed = reshape
                || resolved.text_transform != d.resolved.text_transform
                || resolved.max_lines != d.resolved.max_lines
                || resolved.text_size != d.resolved.text_size
                || resolved.weight != d.resolved.weight
                || resolved.mono != d.resolved.mono
                || resolved.heading != d.resolved.heading
                || resolved.letter_spacing != d.resolved.letter_spacing
                || resolved.foreground != d.resolved.foreground;
            let layout_changed = (reshape && (d.text.is_some() || d.field.is_some()))
                || resolved.padding != d.resolved.padding
                || resolved.mono != d.resolved.mono
                || resolved.heading != d.resolved.heading
                || resolved.letter_spacing != d.resolved.letter_spacing
                || resolved.text_transform != d.resolved.text_transform
                || resolved.max_lines != d.resolved.max_lines
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
                    t.max_lines = d.resolved.max_lines;
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

        drop(style_span);

        // Layout.
        let layout_span = remedy_ui_trace::phase!("layout").entered();
        let layout_dirty = i.tree.take_dirty(Dirty::LAYOUT | Dirty::CHILD_LAYOUT);
        stats.laid_out = layout_dirty.len();
        if !layout_dirty.is_empty() {
            for id in &layout_dirty {
                let Some(d) = i.els.get(id) else { continue };
                let mut style = d.layout.clone();
                // Hidden elements take no space, as the `hidden` attribute does on the web, and
                // neither do the engine's empty wrappers (a `show` showing nothing).
                if d.hidden || d.collapsed {
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
        // Sticky elements, in their scroll container's content space: the compositor moves
        // their layers as it scrolls, and their bounds move here by the same offset.
        let mut stickies: HashMap<ElementId, remedy_ui_compositor::Sticky> = HashMap::new();
        // Each sticky element and whether it is moved from its place now ([`States::STUCK`]).
        let mut stuck: Vec<(ElementId, bool)> = Vec::new();
        let window_cx = StickCx {
            scroller: None,
            viewport: size,
            to_content: Vec2::ZERO,
            offset: Vec2::ZERO,
            within: Rect::from_origin_size(Point::ZERO, size),
        };
        let mut stack = vec![(root, Point::ZERO, Vec2::ZERO, window_cx)];
        while let Some((id, parent_at, scroll, cx)) = stack.pop() {
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
            let mut abs = Point::new(parent_at.x + r.min.x - scroll.x, parent_at.y + r.min.y - scroll.y);
            let parent_rtl =
                || i.tree.parent(id).and_then(|p| i.els.get(&p)).is_some_and(|p| i.layout.is_rtl(p.node));
            if id != root
                && let Some(sides) = sticky_sides(d, parent_rtl)
            {
                let to = cx.to_content;
                let st = remedy_ui_compositor::Sticky {
                    scroller: cx.scroller,
                    viewport: cx.viewport,
                    rect: Rect::from_origin_size(abs, r.size()).translate(to),
                    within: cx.within.translate(to),
                    top: sides.top,
                    right: sides.end,
                    bottom: sides.bottom,
                    left: sides.start,
                };
                let off = st.offset(cx.offset);
                abs = Point::new(abs.x + off.x, abs.y + off.y);
                stuck.push((id, off != Vec2::ZERO));
                stickies.insert(id, st);
            }
            let (bw, p, scrolls) = (d.resolved.border_width, d.resolved.padding, d.scroll);
            let pad = Insets { top: p.top + bw, end: p.end + bw, bottom: p.bottom + bw, start: p.start + bw };
            let pad = if i.layout.is_rtl(d.node) { pad.mirrored() } else { pad };
            let own = Rect::from_origin_size(Point::ZERO, r.size()).deflate(pad);
            if i.frame.bounds_stale {
                let _ = i.tree.set_bounds(id, Rect::from_origin_size(abs, r.size()));
            }
            let mut child_scroll =
                if d.scroll { i.scroll_offsets.get(&id).copied().unwrap_or(Vec2::ZERO) } else { Vec2::ZERO };
            // Right to left, sideways-scrolling content starts at the right and overflows to
            // the left: the compositor's offsets count from its far left edge, so the start
            // shows at an offset as large as the overflow.
            if d.scroll {
                let lead = if d.scroll_x && i.layout.is_rtl(d.node) {
                    i.tree
                        .children(id)
                        .iter()
                        .filter_map(|k| i.els.get(k).and_then(|c| i.layout.rect(c.node)))
                        .fold(0.0f32, |m, r| m.max(-r.min.x))
                } else {
                    0.0
                };
                child_scroll.x -= lead;
                if (d.scroll_lead - lead).abs() > 0.01 {
                    if let Some(dm) = i.els.get_mut(&id) {
                        dm.scroll_lead = lead;
                    }
                    i.layers_changed = true;
                    i.frame.bounds_stale = true;
                }
            }
            // What a sticky child keeps within: this element's content box, or all of its
            // scrolled content in a scroll container, which then becomes the viewport.
            let kid_cx = if scrolls {
                let content = i
                    .tree
                    .children(id)
                    .iter()
                    .filter_map(|k| i.els.get(k).and_then(|c| i.layout.rect(c.node)))
                    .fold(own, Rect::union);
                let origin = Vec2::new(abs.x - child_scroll.x, abs.y - child_scroll.y);
                let offset = i.scroll_offsets.get(&id).copied().unwrap_or(Vec2::ZERO);
                StickCx {
                    scroller: Some(scroll_ids(id).0),
                    viewport: r.size(),
                    to_content: Vec2::new(offset.x - abs.x, offset.y - abs.y),
                    offset,
                    within: content.translate(origin),
                }
            } else {
                StickCx { within: own.translate(abs.to_vec2()), ..cx }
            };
            for k in i.tree.children(id).into_iter().rev() {
                stack.push((k, abs, child_scroll, kid_cx));
            }
        }
        // A sticky element whose place in its container changed goes in a new layer.
        // Window bounds follow in the next frame when only its style made it sticky.
        let resticky = i.frame.stickies != stickies;
        if resticky {
            for id in i.frame.stickies.keys().filter(|k| !stickies.contains_key(k)) {
                let _ = i.tree.set_raised(*id, false);
                stuck.push((*id, false));
            }
            for id in stickies.keys() {
                let _ = i.tree.set_raised(*id, true);
            }
            i.frame.stickies = stickies;
            i.layers_changed = true;
        }
        // Elements gliding through this layout change start from where they were.
        let flipping: Vec<ElementId> =
            i.els.iter().filter(|(_, d)| d.flip.is_some()).map(|(id, _)| *id).collect();
        let mut flipped = false;
        let now_t = i.clock.now();
        for id in flipping {
            let at = i.tree.bounds(id).map(|b| b.min);
            if let (Some(at), Some(d)) = (at, i.els.get_mut(&id)) {
                flipped |= crate::motion::start_flip(d, at, now_t);
            }
        }
        if flipped {
            i.layers_changed = true;
        }
        let moved = i.frame.bounds_stale;
        i.frame.bounds_stale = resticky && !moved;
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

        drop(layout_span);

        // Paint chunks: re-record what is dirty or changed size.
        let paint_span = remedy_ui_trace::phase!("paint").entered();
        // Only elements marked themselves record again: a parent whose child changed keeps
        // its own chunk (layers are assembled from every chunk anyway), so a firefly's tick
        // never re-runs the backdrop painter around it.
        let remeasure = lazy_text_moved(i, &rects, size);
        let mut repaint = i.tree.take_dirty(Dirty::PAINT);
        let _ = i.tree.take_dirty(Dirty::CHILD_PAINT);
        let mut painted = false;
        let focused = i.tree.focused();
        let engine = i.engine.clone();
        let mut engine = engine.borrow_mut();
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
        // Painters that asked to run again next frame.
        let mut again: Vec<ElementId> = Vec::new();
        let mut paint_failures: Vec<(ElementId, String)> = Vec::new();
        let now = i.clock.now();
        let reduced = i.reduced_motion.get_untracked().unwrap_or(false);
        let rate = i.ambient.policy.rate(crate::ambient::activity(i));
        let mut asks = Vec::new();
        for &id in &repaint {
            let sz = rects.get(&id).map(|r| r.size()).unwrap_or(Size::ZERO);
            let Some(d) = i.els.get(&id) else { continue };
            let rtl = i.layout.is_rtl(d.node);
            let shown = if d.text.is_some() {
                shown_band(i, id, Rect::from_origin_size(Point::ZERO, size))
            } else {
                None
            };
            let (chunk, next, failed, ask) = record(
                d,
                rtl,
                sz,
                shown,
                &theme,
                &mut engine,
                focused == Some(id),
                i.scale,
                now,
                reduced,
                rate,
            );
            if let Some(why) = failed {
                paint_failures.push((id, why));
            }
            if let Some(d) = i.els.get_mut(&id) {
                d.chunk = Some((sz, Rc::new(chunk)));
                d.ambient_layer |= ask.ambient;
                if ask.ticked {
                    d.ambient_last = Some(now);
                }
            }
            asks.push((id, next, ask));
            painted = true;
        }
        drop(engine);
        // Painters that want to be drawn again: now, later, or once seen. Nothing unseen
        // keeps frames coming.
        let window = Rect::from_origin_size(Point::ZERO, size);
        for (id, next, ask) in asks {
            if !next && ask.wake_in.is_none() && !ask.ambient {
                i.ambient.wakes.remove(&id);
                continue;
            }
            let kind = if ask.ambient { crate::TickKind::Ambient } else { crate::TickKind::Seen };
            if !is_seen(i, id, window) || (ask.ambient && !ask.ticked) {
                i.ambient.wakes.remove(&id);
                i.ambient.parked_paint.insert(id, kind);
                continue;
            }
            if next {
                again.push(id);
            } else if let Some(w) = ask.wake_in {
                i.ambient.wakes.insert(id, now + w);
            }
        }
        for id in &again {
            i.tree.mark(*id, Dirty::PAINT);
        }
        repaint.retain(|id| i.els.contains_key(id));
        stats.repainted = repaint;
        drop(paint_span);

        // Layers.
        let layers_span = remedy_ui_trace::phase!("layers").entered();
        let transformed = std::mem::take(&mut i.layers_changed);
        let layers = (painted || moved || transformed).then(|| {
            let mut state = std::mem::take(&mut i.frame);
            state.scrollers.clear();
            let mut b = Builder::new(LayerId(0), Rect::from_origin_size(Point::ZERO, size));
            let stickies = std::mem::take(&mut state.stickies);
            let mut walk = Walk {
                inner: i,
                state: &mut state,
                rects: &rects,
                stickies: &stickies,
                layered: HashMap::new(),
            };
            walk.element(&mut b, root, Point::ZERO);
            let layer = b.finish_into(&mut state);
            state.stickies = stickies;
            let live: std::collections::HashSet<LayerId> = collect_ids(&layer);
            state.lists.retain(|k, _| live.contains(k));
            i.frame = state;
            LayerTree { roots: vec![Arc::new(layer)] }
        });
        stats.layers = layers.is_some();
        drop(layers_span);

        // Semantics.
        let semantics_span = remedy_ui_trace::phase!("semantics").entered();
        let sem_dirty = !i.tree.take_dirty(Dirty::SEMANTICS).is_empty();
        if sem_dirty || i.structure_changed || moved {
            i.structure_changed = false;
            sync_semantics(i, &rects);
            stats.semantics = true;
        }
        drop(semantics_span);
        // Elements that follow their place on the window (embedded native views) hear where
        // they are now, after layout, scrolling and overlays settled.
        let mut placed: Vec<(PlacedHandler, Option<Placement>)> = Vec::new();
        let watchers: Vec<ElementId> =
            i.els.iter().filter(|(_, d)| d.on_placed.is_some()).map(|(id, _)| *id).collect();
        for id in watchers {
            let now = placement(i, id, window);
            if let Some(d) = i.els.get_mut(&id)
                && d.last_placed.as_ref() != Some(&now)
                && let Some(cb) = d.on_placed.clone()
            {
                d.last_placed = Some(now.clone());
                placed.push((cb, now));
            }
        }
        i.last_stats = stats;
        let scrollers = i.frame.scrollers.clone();
        drop(guard);
        if !again.is_empty() || (resticky && !moved) {
            self.request(crate::Request::Frame);
        }
        // Stuck or back in place: styled for it from the next frame (a no-op when unchanged).
        for (id, on) in stuck {
            self.set_state(id, States::STUCK, on);
        }
        // Lazy paragraphs whose height came out different once more lines were laid out are
        // measured again.
        for id in remeasure {
            let node = self.0.borrow().els.get(&id).map(|d| d.node);
            if let Some(n) = node {
                let _ = self.0.borrow_mut().layout.mark_dirty(n);
            }
            self.mark(id, Dirty::LAYOUT | Dirty::PAINT);
        }
        self.arm_wakes(now);
        self.wake_parked();
        for (cb, size) in resized {
            cb(size);
        }
        for (cb, now) in seen {
            cb(now);
        }
        for (cb, now) in placed {
            cb(now.as_ref());
        }
        // Laid out with the keyboard's inset: scroll what has focus back above it.
        if std::mem::take(&mut self.0.borrow_mut().reveal_focus)
            && let Some(f) = self.tree().focused()
        {
            crate::El { ui: self.clone(), id: f }.scroll_into_view();
            self.request(crate::Request::Frame);
        }
        for (id, why) in paint_failures {
            self.report_failure(id, crate::FailurePhase::Paint, why, None);
        }
        if flipped {
            self.request(crate::Request::Frame);
        }
        Frame { layers, scrollers }
    }
}

/// The part of `id` that shows on the window, as a band of its own y (from its top): inside
/// the window and every scroll container or clipping ancestor. `None` when none of it does.
fn shown_band(i: &Inner, id: ElementId, window: Rect) -> Option<std::ops::Range<f32>> {
    let bounds = i.tree.bounds(id)?;
    let mut visible = bounds.intersect(window)?;
    let mut cur = i.tree.parent(id);
    while let Some(c) = cur {
        let d = i.els.get(&c)?;
        if d.scroll || d.clip {
            visible = visible.intersect(i.tree.bounds(c)?)?;
        }
        cur = i.tree.parent(c);
    }
    Some(visible.min.y - bounds.min.y..visible.max.y - bounds.min.y)
}

/// Text elements with a lazily laid-out paragraph: those scrolled past the lines they drew are
/// marked to paint again. Returns those whose paragraph's height (estimated below the lines
/// laid out) no longer matches the box it was measured into, to measure again.
fn lazy_text_moved(i: &Inner, rects: &HashMap<ElementId, Rect>, size: Size) -> Vec<ElementId> {
    let window = Rect::from_origin_size(Point::ZERO, size);
    let mut remeasure = Vec::new();
    for (id, d) in &i.els {
        let Some(slot) = &d.text else { continue };
        let Some((painted, height)) = slot.borrow().painted.clone() else { continue };
        let inset = d.resolved.padding.top + d.resolved.border_width;
        let shown = shown_band(i, *id, window).map(|b| b.start - inset..b.end - inset);
        if shown.is_some_and(|b| b.start < painted.start || b.end > painted.end) {
            i.tree.mark(*id, Dirty::PAINT);
        }
        let boxed = rects.get(id).map_or(0.0, |r| r.height())
            - inset
            - d.resolved.padding.bottom
            - d.resolved.border_width;
        if (height.ceil() - boxed).abs() > 0.5 {
            remeasure.push(*id);
        }
    }
    remeasure
}

/// True when some of `id` shows: it and its ancestors are not hidden or transparent, the window
/// is not covered, and its box overlaps the window and every clipping ancestor.
pub(crate) fn is_seen(i: &Inner, id: ElementId, window: Rect) -> bool {
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

/// Where `id` is on the window, or `None` while none of it is seen ([`is_seen`]).
fn placement(i: &Inner, id: ElementId, window: Rect) -> Option<Placement> {
    if !is_seen(i, id, window) {
        return None;
    }
    let bounds = i.tree.bounds(id)?;
    let mut visible = bounds.intersect(window)?;
    let mut cur = i.tree.parent(id);
    while let Some(c) = cur {
        let d = i.els.get(&c)?;
        if d.scroll || d.clip {
            visible = visible.intersect(i.tree.bounds(c)?)?;
        }
        cur = i.tree.parent(c);
    }
    // What the overlay layers above the element draw over it: each shown overlay's box.
    let mut layers = vec![i.overlays, i.trusted];
    let mut at = Some(id);
    while let Some(c) = at {
        if c == i.overlays {
            // An element inside the overlay layer is covered only by the trusted layer.
            layers = vec![i.trusted];
        }
        if c == i.trusted {
            layers.clear();
        }
        at = i.tree.parent(c);
    }
    let mut covered = Vec::new();
    for layer in layers {
        for k in i.tree.children(layer) {
            let Some(kd) = i.els.get(&k) else { continue };
            if kd.hidden || kd.resolved.opacity <= 0.0 || at_or_below(i, k, id) {
                continue;
            }
            if let Some(r) = i.tree.bounds(k).and_then(|b| b.intersect(visible)) {
                covered.push(r);
            }
        }
    }
    Some(Placement { bounds, visible, covered, scale: i.scale })
}

/// True when `id` is `top` or inside it.
fn at_or_below(i: &Inner, top: ElementId, id: ElementId) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if c == top {
            return true;
        }
        cur = i.tree.parent(c);
    }
    false
}

/// Where an element is on its window after a frame, for content the platform draws itself
/// over the window (an embedded web view): [`El::on_placed`](crate::El::on_placed).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Placement {
    /// The element's box in window logical pixels, scrolled where it is shown.
    pub bounds: Rect,
    /// The part of it that shows: inside the window and every scroll container or clipping
    /// element around it.
    pub visible: Rect,
    /// Parts of `visible` drawn over by the window's overlays (menus, dialogs, tooltips, the
    /// privacy cover), which content placed there must leave uncovered.
    pub covered: Vec<Rect>,
    /// Device pixels per logical pixel.
    pub scale: f32,
}

impl Placement {
    /// An element at `bounds`, showing `visible`, with `covered` parts under overlays, at
    /// `scale` device pixels per logical pixel (bridges and tests that place content).
    pub fn new(bounds: Rect, visible: Rect, covered: Vec<Rect>, scale: f32) -> Self {
        Placement { bounds, visible, covered, scale }
    }
}

/// An element's placement callback.
pub(crate) type PlacedHandler = Rc<dyn Fn(Option<&Placement>)>;

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
        let s = composed_semantics(d, id == root, i.tree.is_focusable(id), i.pseudo);
        i.a11y.set(root_key(id), s.bounds(bounds));
        let child_keys: Vec<u64> = kids.iter().map(|k| root_key(*k)).collect();
        i.a11y.set_children(root_key(id), &child_keys);
        stack.extend(kids);
    }
    // With nothing focused, accessibility focus is the window (not the element that last
    // had focus).
    let focus = i.tree.focused().unwrap_or(root);
    i.a11y.set_focus(root_key(focus));
}
