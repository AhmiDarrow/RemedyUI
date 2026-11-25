//! A stage: art of a fixed design size scaled to fit its box, with children at named anchors.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_core::input::PointerPhase;
use remedy_ui_paint::{Affine, Image, Item, Sampling};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_view::{El, IntoProp, PaintCx, Prop, Ui};

use crate::common::{Place, Slot, layer};

/// How a stage's design fills the box it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fit {
    /// All of the design shows, as large as fits, centered; the rest of the box is empty
    /// (letterboxed).
    #[default]
    Contain,
    /// The design covers the whole box, centered; what does not fit is cut off.
    Cover,
    /// The design is stretched to the box on each axis separately.
    Stretch,
}

/// How design coordinates map to the box: `screen = design * scale + offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageTransform {
    /// Horizontal scale.
    pub scale_x: f32,
    /// Vertical scale.
    pub scale_y: f32,
    /// Where the design's origin lands in the box.
    pub offset: Vec2,
}

impl Default for StageTransform {
    fn default() -> Self {
        StageTransform { scale_x: 1.0, scale_y: 1.0, offset: Vec2::ZERO }
    }
}

impl StageTransform {
    /// The transform that fits `design` into a box of `area` by `fit`. With `pixel_perfect`,
    /// a scale of one or more is rounded down to a whole number and the offset to whole
    /// pixels, so pixel art stays crisp (a smaller box still shrinks it smoothly).
    ///
    /// ```
    /// use remedy_ui_core::{Size, Vec2};
    /// use remedy_ui_widgets_stage::{Fit, StageTransform};
    ///
    /// let t = StageTransform::fit(Size::new(320.0, 180.0), Size::new(1000.0, 600.0), Fit::Contain, true);
    /// assert_eq!((t.scale_x, t.scale_y), (3.0, 3.0));
    /// assert_eq!(t.offset, Vec2::new(20.0, 30.0));
    /// ```
    pub fn fit(design: Size, area: Size, fit: Fit, pixel_perfect: bool) -> StageTransform {
        let ratio = |a: f32, d: f32| if d > 0.0 && a.is_finite() { (a / d).max(0.0) } else { 1.0 };
        let (rx, ry) = (ratio(area.width, design.width), ratio(area.height, design.height));
        let whole = |s: f32| if pixel_perfect && s >= 1.0 { s.floor() } else { s };
        let (sx, sy) = match fit {
            Fit::Contain => (whole(rx.min(ry)), whole(rx.min(ry))),
            Fit::Cover => (whole(rx.max(ry)), whole(rx.max(ry))),
            Fit::Stretch => (whole(rx), whole(ry)),
        };
        let mut offset =
            Vec2::new((area.width - design.width * sx) / 2.0, (area.height - design.height * sy) / 2.0);
        if pixel_perfect {
            offset = Vec2::new(offset.x.round(), offset.y.round());
        }
        StageTransform { scale_x: sx, scale_y: sy, offset }
    }

    /// A design point in the box.
    pub fn to_screen(&self, p: Point) -> Point {
        Point::new(p.x * self.scale_x + self.offset.x, p.y * self.scale_y + self.offset.y)
    }

    /// A design rectangle in the box.
    pub fn rect_to_screen(&self, r: Rect) -> Rect {
        Rect::from_points(self.to_screen(r.min), self.to_screen(r.max))
    }

    /// A point in the box in design coordinates (for hit testing art).
    pub fn to_design(&self, p: Point) -> Point {
        let inv = |v: f32, o: f32, s: f32| if s != 0.0 { (v - o) / s } else { 0.0 };
        Point::new(inv(p.x, self.offset.x, self.scale_x), inv(p.y, self.offset.y, self.scale_y))
    }

    /// The transform as an affine, for drawing in design coordinates.
    pub fn affine(&self) -> Affine {
        Affine::scale(self.scale_x, self.scale_y).then(Affine::translate(self.offset))
    }
}

/// Where a child goes on a stage: a rectangle in design coordinates (a point is an empty
/// one) and how the child sits in it on each axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// The box, in design coordinates.
    pub rect: Rect,
    /// Horizontal placement in the box.
    pub x: Place,
    /// Vertical placement in the box.
    pub y: Place,
}

impl Anchor {
    /// A point: the child is centered on it, at its own size.
    pub fn point(x: f32, y: f32) -> Anchor {
        Anchor { rect: Rect::new(x, y, x, y), x: Place::Center, y: Place::Center }
    }

    /// A rectangle the child fills.
    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Anchor {
        Anchor { rect: Rect::new(x, y, x + width, y + height), x: Place::Fill, y: Place::Fill }
    }

    /// Places the child in the box: `Start` hangs it after a point (or at a box's start edge),
    /// `End` before it, `Center` centers it and `Fill` stretches it.
    pub fn place(mut self, x: Place, y: Place) -> Anchor {
        self.x = x;
        self.y = y;
        self
    }
}

impl IntoProp<Anchor> for Anchor {
    fn into_prop(self) -> Prop<Anchor> {
        Prop::Value(self)
    }
}

type Painter = Rc<dyn Fn(&mut PaintCx<'_>)>;

/// A layer of art on a stage, drawn in design coordinates behind the children (or in front
/// with [`Layer::front`]). Layers are decoration: they are not in the accessibility tree and
/// take no input.
#[derive(Clone)]
pub struct Layer {
    painter: Option<Painter>,
    image: Option<(Image, Sampling)>,
    depth: f32,
    front: bool,
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("image", &self.image)
            .field("depth", &self.depth)
            .field("front", &self.front)
            .finish_non_exhaustive()
    }
}

impl Layer {
    /// Art drawn by `f` in design coordinates: `cx.size` is the design size while it runs.
    /// Take colors from `cx.theme` (or [`crate::Ink`]s) so a skin restyles the art. Signals it
    /// reads repaint the stage when they change.
    pub fn paint(f: impl Fn(&mut PaintCx<'_>) + 'static) -> Layer {
        Layer { painter: Some(Rc::new(f)), image: None, depth: 0.0, front: false }
    }

    /// A picture stretched over the whole design, smoothly sampled. Use
    /// [`Layer::sampling`] with `Sampling::Nearest` for pixel art.
    pub fn image(image: Image) -> Layer {
        Layer { painter: None, image: Some((image, Sampling::Linear)), depth: 0.0, front: false }
    }

    /// How an image layer is sampled when scaled.
    pub fn sampling(mut self, sampling: Sampling) -> Layer {
        if let Some((_, s)) = &mut self.image {
            *s = sampling;
        }
        self
    }

    /// How far the layer moves with the pointer, when the stage has parallax: 0 stays put,
    /// 1 moves the stage's full parallax distance, negative moves against the pointer.
    pub fn depth(mut self, depth: f32) -> Layer {
        self.depth = if depth.is_finite() { depth } else { 0.0 };
        self
    }

    /// Draws the layer in front of the children (foliage, a window frame, rain).
    pub fn front(mut self) -> Layer {
        self.front = true;
        self
    }

    fn draw(&self, cx: &mut PaintCx<'_>, t: StageTransform, design: Size, shift: Vec2) {
        let t = StageTransform { offset: Vec2::new(t.offset.x + shift.x, t.offset.y + shift.y), ..t };
        cx.list.push(Item::PushTransform(t.affine()));
        let size = cx.size;
        cx.size = design;
        if let Some((image, sampling)) = &self.image {
            cx.list.push(Item::Image {
                image: image.clone(),
                rect: Rect::from_origin_size(Point::ZERO, design),
                radii: CornerRadii::ZERO,
                sampling: *sampling,
            });
        }
        if let Some(p) = &self.painter {
            p(cx);
        }
        cx.size = size;
        cx.list.push(Item::PopTransform);
    }
}

/// How a [`scene`] fits and moves.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SceneOptions {
    /// How the design fills the box.
    pub fit: Fit,
    /// Whole-number scaling and whole-pixel offsets, for pixel art.
    pub pixel_perfect: bool,
    /// How far (in design units) a layer of depth 1 moves as the pointer crosses the stage.
    /// Zero turns parallax off. It is always off under reduced motion.
    pub parallax: f32,
}

impl Default for SceneOptions {
    fn default() -> Self {
        SceneOptions { fit: Fit::Contain, pixel_perfect: false, parallax: 0.0 }
    }
}

impl SceneOptions {
    /// Sets `fit`: how the design fills the box.
    pub fn fit(mut self, value: Fit) -> Self {
        self.fit = value;
        self
    }

    /// Sets `pixel_perfect`: whole-number scaling and whole-pixel offsets, for pixel art.
    pub fn pixel_perfect(mut self, value: bool) -> Self {
        self.pixel_perfect = value;
        self
    }

    /// Sets `parallax`: how far a layer of depth 1 moves with the pointer, in design units.
    pub fn parallax(mut self, value: f32) -> Self {
        self.parallax = if value.is_finite() { value.max(0.0) } else { 0.0 };
        self
    }
}

struct Placed {
    slot: Slot,
    anchor: Cell<Anchor>,
    name: Option<String>,
}

struct State {
    design: Size,
    options: SceneOptions,
    anchors: RefCell<Vec<(String, Anchor)>>,
    placed: RefCell<Vec<Rc<Placed>>>,
    transform: Cell<StageTransform>,
    size: Cell<Option<Size>>,
    rtl: Cell<bool>,
    back: RefCell<Vec<Layer>>,
    front: RefCell<Vec<Layer>>,
    pointer: Signal<Vec2>,
    content: El,
    front_el: El,
    el: El,
}

impl State {
    fn put(&self, p: &Placed) {
        let Some(size) = self.size.get() else { return };
        let a = p.anchor.get();
        let r = self.transform.get().rect_to_screen(a.rect);
        p.slot.put(r, a.x, a.y, self.rtl.get(), size.width);
    }

    fn put_all(&self) {
        for p in self.placed.borrow().iter() {
            self.put(p);
        }
    }

    fn shift(&self, depth: f32, reduced: bool) -> Vec2 {
        if reduced || self.options.parallax == 0.0 || depth == 0.0 {
            return Vec2::ZERO;
        }
        let p = self.pointer.get();
        let t = self.transform.get();
        let d = self.options.parallax * depth;
        Vec2::new(p.x * d * t.scale_x, p.y * d * t.scale_y)
    }
}

/// A stage made by [`scene`]: add layers, anchors and children, then use it as an element.
#[derive(Clone)]
pub struct Scene(Rc<State>);

impl std::fmt::Debug for Scene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene").field("design", &self.0.design).finish_non_exhaustive()
    }
}

/// A stage of a fixed design size (say 1280 by 720) that scales to fit its box.
///
/// Art is drawn in [`Layer`]s in design coordinates; children are placed at [`Anchor`]s in
/// design coordinates (by name with [`Scene::anchor`] and [`Scene::place`], or directly with
/// [`Scene::place_at`]) and stay on their spot of the art as the window resizes. Children keep
/// their own size (text stays crisp and readable); a child at a rectangle anchor that fills it
/// is sized to the scaled rectangle.
///
/// Give the stage a size or let it grow like any element. It is a group named `label` to
/// assistive technology, and its children are read and focused in the order they were placed,
/// whatever their position in the art. With [`SceneOptions::parallax`], layers with a depth
/// drift with the pointer; never under reduced motion. The stage does nothing while nothing
/// changes.
///
/// ```
/// use remedy_ui_widgets_stage::{Anchor, Layer, SceneOptions, scene};
/// # fn demo(ui: &remedy_ui_view::Ui, door: remedy_ui_view::El) {
/// let stage = scene(ui, "Harbor", remedy_ui_core::Size::new(1280.0, 720.0), SceneOptions::default())
///     .layer(Layer::paint(|cx| {
///         let sky = cx.theme.colors.panel;
///         cx.list.fill_rect(remedy_ui_core::Rect::new(0.0, 0.0, 1280.0, 400.0), remedy_ui_paint::Brush::Solid(sky));
///     }))
///     .anchor("pier", Anchor::point(640.0, 560.0))
///     .place("pier", door);
/// let el: remedy_ui_view::El = stage.into();
/// # let _ = el;
/// # }
/// ```
#[track_caller]
pub fn scene(ui: &Ui, label: impl Into<String>, design: Size, options: SceneOptions) -> Scene {
    let label = label.into();
    let design = Size::new(design.width.max(1.0), design.height.max(1.0));
    let content = layer(ui, "scene_content");
    let front_el = layer(ui, "scene_front");
    let el = ui.el("scene").clip().child(content.clone()).child(front_el.clone());
    let state = Rc::new(State {
        design,
        options,
        anchors: RefCell::new(Vec::new()),
        placed: RefCell::new(Vec::new()),
        transform: Cell::new(StageTransform::default()),
        size: Cell::new(None),
        rtl: Cell::new(false),
        back: RefCell::new(Vec::new()),
        front: RefCell::new(Vec::new()),
        pointer: Signal::new(Vec2::ZERO),
        content,
        front_el: front_el.clone(),
        el: el.clone(),
    });

    // The element owns the stage's state through its painters (the `Scene` handle may be
    // dropped once the stage is built).
    let painter = |front: bool| {
        let s = state.clone();
        move |cx: &mut PaintCx<'_>| {
            let t = s.transform.get();
            let layers = if front { s.front.borrow() } else { s.back.borrow() };
            if layers.is_empty() {
                return;
            }
            cx.list.push_clip(Rect::from_origin_size(Point::ZERO, cx.size), CornerRadii::ZERO);
            for l in layers.iter() {
                let shift = s.shift(l.depth, cx.reduced_motion);
                l.draw(cx, t, s.design, shift);
            }
            cx.list.pop_clip();
        }
    };
    let _ = front_el.paint(painter(true));

    let weak = Rc::downgrade(&state);
    let w2 = weak.clone();
    let w3 = weak.clone();
    let ui = ui.clone();
    let pointer = state.pointer;
    let parallax = state.options.parallax > 0.0;
    let el = el.paint(painter(false)).on_resize(move |size| {
        let Some(s) = weak.upgrade() else { return };
        s.size.set(Some(size));
        s.rtl.set(s.el.is_rtl());
        let t = StageTransform::fit(s.design, size, s.options.fit, s.options.pixel_perfect);
        if s.transform.replace(t) != t {
            s.el.repaint();
            s.front_el.repaint();
        }
        s.put_all();
    });
    let el = if parallax {
        // Parallax only decorates: nothing is reached through it, so the stage stays a
        // plain group to the keyboard.
        el.on_pointer(move |e| {
            if e.phase != PointerPhase::Move {
                return false;
            }
            if let Some(b) = w2.upgrade().and_then(|s| s.el.bounds()) {
                let half = Vec2::new(b.width() / 2.0, b.height() / 2.0);
                let c = b.center();
                let n = |d: f32, h: f32| if h > 0.0 { (d / h).clamp(-1.0, 1.0) } else { 0.0 };
                pointer
                    .set_if_changed(Vec2::new(n(e.position.x - c.x, half.x), n(e.position.y - c.y, half.y)));
            }
            false
        })
        .on_hover(move |inside| {
            if !inside {
                pointer.set_if_changed(Vec2::ZERO);
            }
        })
        .keyboard_alternative({
            let u = ui.clone();
            move || crate::STRINGS.get(&u, "scene-parallax-keys")
        })
        // Turning reduced motion on settles the layers.
        .bind_named("reduced_motion", move || {
            let _ = ui.reduced_motion();
            if let Some(s) = w3.upgrade() {
                s.el.repaint();
                s.front_el.repaint();
            }
        })
    } else {
        el
    };
    if !label.is_empty() {
        let _ = el.semantics(Semantics::new(Role::Group).label(label));
    }
    Scene(state)
}

impl Scene {
    /// Names a spot on the stage. Naming it again moves it, and every child placed there.
    pub fn anchor(self, name: impl Into<String>, anchor: Anchor) -> Self {
        self.set_anchor(name, anchor);
        self
    }

    /// Moves a named anchor (or adds it) and the children placed at it.
    pub fn set_anchor(&self, name: impl Into<String>, anchor: Anchor) {
        let name = name.into();
        {
            let mut anchors = self.0.anchors.borrow_mut();
            match anchors.iter_mut().find(|(n, _)| *n == name) {
                Some((_, a)) => *a = anchor,
                None => anchors.push((name.clone(), anchor)),
            }
        }
        for p in self.0.placed.borrow().iter() {
            if p.name.as_deref() == Some(name.as_str()) {
                p.anchor.set(anchor);
                self.0.put(p);
            }
        }
    }

    /// The named anchor, if there is one.
    pub fn anchor_named(&self, name: &str) -> Option<Anchor> {
        self.0.anchors.borrow().iter().find(|(n, _)| n == name).map(|(_, a)| *a)
    }

    /// Adds a layer of art. Layers draw in the order added; front layers after the
    /// children.
    pub fn layer(self, layer: Layer) -> Self {
        if layer.front {
            self.0.front.borrow_mut().push(layer);
            self.0.front_el.repaint();
        } else {
            self.0.back.borrow_mut().push(layer);
            self.0.el.repaint();
        }
        self
    }

    /// Places `child` at the anchor called `name`. A name not given yet places it at the
    /// design's center until [`Scene::anchor`] names it.
    pub fn place(self, name: impl Into<String>, child: impl Into<El>) -> Self {
        let name = name.into();
        let anchor = self
            .anchor_named(&name)
            .unwrap_or(Anchor::point(self.0.design.width / 2.0, self.0.design.height / 2.0));
        let _ = self.add(child.into(), anchor, Some(name));
        self
    }

    /// Places `child` at an anchor of its own: a constant, a signal or a closure, so a child
    /// can move over the art (a character walking to a fire).
    pub fn place_at(self, anchor: impl IntoProp<Anchor>, child: impl Into<El>) -> Self {
        let anchor = anchor.into_prop();
        let start = untrack(|| anchor.get());
        let placed = self.add(child.into(), start, None);
        let weak = Rc::downgrade(&self.0);
        let slot_el = placed.slot.el.clone();
        slot_el.prop("anchor", anchor, move |_, a| {
            if let Some(s) = weak.upgrade() {
                placed.anchor.set(a);
                s.put(&placed);
            }
        });
        self
    }

    fn add(&self, child: El, anchor: Anchor, name: Option<String>) -> Rc<Placed> {
        let slot = Slot::new(self.0.el.ui(), child, anchor.x);
        let _ = self.0.content.clone().child(slot.el.clone());
        let placed = Rc::new(Placed { slot, anchor: Cell::new(anchor), name });
        self.0.put(&placed);
        self.0.placed.borrow_mut().push(placed.clone());
        placed
    }

    /// The stage's element.
    pub fn el(&self) -> &El {
        &self.0.el
    }

    /// How design coordinates map to the stage's box as of the last frame.
    pub fn transform(&self) -> StageTransform {
        self.0.transform.get()
    }

    /// The design size.
    pub fn design(&self) -> Size {
        self.0.design
    }
}

impl From<Scene> for El {
    fn from(s: Scene) -> El {
        s.0.el.clone()
    }
}
