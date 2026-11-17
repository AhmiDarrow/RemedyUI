//! Elements and the builder components use.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use remedy_ui_a11y::{A11yTree, Semantics};
use remedy_ui_core::{Color, Insets, Size, Vec2};
use remedy_ui_layout::{Available, LayoutNode, LayoutStyle, LayoutTree, MeasureInput};
use remedy_ui_paint::DisplayList;
use remedy_ui_reactive::Effect;
use remedy_ui_style::{ResolvedStyle, States, StyleKey, StylePatch, StyleSheet, Theme};
use remedy_ui_text::{FontCollection, Paragraph, ParagraphOptions, RichText, TextEngine, TextStyle};
use remedy_ui_tree::{Dirty, ElementId, HitMode, Phase, Tree};

use crate::Request;
use crate::events::Click;
use crate::field::Field;

/// What a custom painter gets: the list to record into (local coordinates, origin at the
/// element's top-left), the element's size, its resolved style and the theme.
pub struct PaintCx<'a> {
    /// Record here.
    pub list: &'a mut DisplayList,
    /// The element's size in logical pixels.
    pub size: Size,
    /// Its resolved style.
    pub style: &'a ResolvedStyle,
    /// The window's theme.
    pub theme: &'a Theme,
}

pub(crate) type Painter = Rc<dyn Fn(&mut PaintCx<'_>)>;

/// A text element's content and its last layout.
pub(crate) struct TextSlot {
    pub(crate) text: RichText,
    pub(crate) style: TextStyle,
    pub(crate) wrap: bool,
    pub(crate) scale: f32,
    /// The paragraph at the last wrap width asked for.
    pub(crate) cache: Option<(Option<u32>, Rc<Paragraph>)>,
}

impl TextSlot {
    pub(crate) fn paragraph(&mut self, engine: &mut TextEngine, max_width: Option<f32>) -> Rc<Paragraph> {
        let max_width = if self.wrap { max_width } else { None };
        let key = max_width.map(f32::to_bits);
        if let Some((k, p)) = &self.cache
            && *k == key
        {
            return p.clone();
        }
        let options = ParagraphOptions { max_width, scale: self.scale, ..ParagraphOptions::default() };
        let p = Rc::new(engine.layout(&self.text, &self.style, options));
        self.cache = Some((key, p.clone()));
        p
    }
}

pub(crate) struct ElData {
    pub(crate) widget: &'static str,
    pub(crate) variant: &'static str,
    pub(crate) states: States,
    pub(crate) local: StylePatch,
    pub(crate) resolved: ResolvedStyle,
    pub(crate) layout: LayoutStyle,
    pub(crate) node: LayoutNode,
    pub(crate) text: Option<Rc<RefCell<TextSlot>>>,
    pub(crate) field: Option<Rc<RefCell<Field>>>,
    pub(crate) painter: Option<Painter>,
    pub(crate) semantics: Option<Semantics>,
    pub(crate) scroll: bool,
    pub(crate) clip: bool,
    pub(crate) click: Option<Rc<dyn Fn()>>,
    /// The last recorded paint chunk, and the size it was recorded at.
    pub(crate) chunk: Option<(Size, Rc<DisplayList>)>,
    /// Bindings owned by this element, disposed with it.
    pub(crate) effects: Vec<Effect>,
}

pub(crate) struct Inner {
    pub(crate) tree: Tree,
    pub(crate) layout: LayoutTree,
    pub(crate) els: HashMap<ElementId, ElData>,
    pub(crate) sheet: StyleSheet,
    pub(crate) theme: Theme,
    pub(crate) engine: Rc<RefCell<TextEngine>>,
    pub(crate) size: Size,
    pub(crate) scale: f32,
    pub(crate) requests: Vec<Request>,
    pub(crate) scroll_offsets: HashMap<ElementId, Vec2>,
    pub(crate) hovered: Vec<ElementId>,
    pub(crate) pressed: Option<ElementId>,
    pub(crate) focus_visible: bool,
    pub(crate) a11y: A11yTree,
    pub(crate) structure_changed: bool,
    pub(crate) frame: crate::frame::FrameState,
    pub(crate) now_ms: u64,
    pub(crate) field_session: Option<ElementId>,
    pub(crate) focus_shown: Option<ElementId>,
    pub(crate) platform: remedy_ui_input::Platform,
    pub(crate) content: ElementId,
    pub(crate) overlays: ElementId,
    pub(crate) timers: HashMap<u64, Box<dyn FnOnce()>>,
    pub(crate) next_timer: u64,
}

/// One window's UI. Cloning gives another handle to the same UI.
#[derive(Clone)]
pub struct Ui(pub(crate) Rc<RefCell<Inner>>);

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.try_borrow() {
            Ok(i) => write!(f, "Ui({} elements)", i.els.len()),
            Err(_) => f.write_str("Ui(busy)"),
        }
    }
}

/// An element being built or bound. Cheap to clone.
#[derive(Clone, Debug)]
pub struct El {
    pub(crate) ui: Ui,
    pub(crate) id: ElementId,
}

fn root_style() -> LayoutStyle {
    LayoutStyle {
        width: remedy_ui_layout::Length::Percent(1.0),
        height: remedy_ui_layout::Length::Percent(1.0),
        ..LayoutStyle::column()
    }
}

pub(crate) fn defaults(theme: &Theme) -> StylePatch {
    StylePatch {
        background: Some(Color::TRANSPARENT),
        foreground: Some(theme.derive().text),
        border: Some(Color::TRANSPARENT),
        border_width: Some(0.0),
        radius: Some(0.0),
        padding: Some(Insets::ZERO),
        text_size: Some(theme.type_scale.body),
        weight: Some(400.0),
        opacity: Some(1.0),
        mono: Some(false),
    }
}

impl Ui {
    /// A UI with `theme`, styled by `sheet`, drawing text with `fonts`.
    pub fn new(theme: Theme, sheet: StyleSheet, fonts: FontCollection) -> Ui {
        let tree = Tree::new();
        let mut layout = LayoutTree::new();
        let root = tree.root();
        let node = layout.node(&root_style()).unwrap_or_else(|_| unreachable_root());
        let mut sheet = sheet;
        let resolved = sheet.resolve(
            &StyleKey { widget: "window", variant: "", states: States::NONE },
            &defaults(&theme),
            &StylePatch::default(),
        );
        let mut els = HashMap::new();
        els.insert(root, ElData::new("window", root_style(), node, resolved));
        tree.mark(root, Dirty::STYLE | Dirty::LAYOUT | Dirty::PAINT);
        let ui = Ui(Rc::new(RefCell::new(Inner {
            tree,
            layout,
            els,
            sheet,
            theme,
            engine: Rc::new(RefCell::new(TextEngine::new(fonts))),
            size: Size::new(1.0, 1.0),
            scale: 1.0,
            requests: Vec::new(),
            scroll_offsets: HashMap::new(),
            hovered: Vec::new(),
            pressed: None,
            focus_visible: false,
            a11y: A11yTree::new(root_key(root)),
            structure_changed: true,
            frame: crate::frame::FrameState::default(),
            now_ms: 0,
            field_session: None,
            focus_shown: None,
            platform: remedy_ui_input::Platform::Standard,
            content: root,
            overlays: root,
            timers: HashMap::new(),
            next_timer: 0,
        })));
        // The window holds the app's content, then a click-through layer for overlays
        // (dialogs, menus, toasts) that always draws above it.
        let content = ui.el("content").layout(|l| {
            l.grow = 1.0;
            l.width = remedy_ui_layout::Length::Percent(1.0);
            l.display = remedy_ui_layout::Display::Flex;
            l.axis = remedy_ui_layout::Axis::Column;
        });
        let overlays = ui.el("overlays").layout(|l| {
            l.position = remedy_ui_layout::Position::Absolute;
            l.inset = Insets::ZERO;
            l.width = remedy_ui_layout::Length::Percent(1.0);
            l.height = remedy_ui_layout::Length::Percent(1.0);
        });
        let _ = ui.tree().set_hit_mode(overlays.id, HitMode::PassThrough);
        let window = El { ui: ui.clone(), id: root };
        let _ = window.child(content.clone()).child(overlays.clone());
        {
            let mut i = ui.0.borrow_mut();
            i.content = content.id;
            i.overlays = overlays.id;
        }
        ui
    }

    /// Shows `el` above the window's content until it is removed. Positions inside are
    /// absolute in window coordinates when the element asks for it.
    pub fn overlay(&self, el: El) -> El {
        let host = self.0.borrow().overlays;
        let _ = El { ui: self.clone(), id: host }.child(el.clone());
        el
    }

    /// Runs `f` once after `delay`. Returns an id for [`Ui::cancel_timer`].
    pub fn after(&self, delay: std::time::Duration, f: impl FnOnce() + 'static) -> u64 {
        let id = {
            let mut i = self.0.borrow_mut();
            i.next_timer += 1;
            let id = i.next_timer;
            i.timers.insert(id, Box::new(f));
            id
        };
        self.request(Request::Timer { id, delay });
        id
    }

    /// Cancels a timer that has not fired.
    pub fn cancel_timer(&self, id: u64) {
        if self.0.borrow_mut().timers.remove(&id).is_some() {
            self.request(Request::CancelTimer(id));
        }
    }

    /// Runs a due timer (the runner calls this).
    pub fn fire_timer(&self, id: u64) {
        let f = self.0.borrow_mut().timers.remove(&id);
        if let Some(f) = f {
            f();
        }
    }

    /// The window's content element (a column filling the window, below overlays).
    pub fn root(&self) -> El {
        let id = self.0.borrow().content;
        El { ui: self.clone(), id }
    }

    /// A new element styled as `widget`. It shows once added to a parent.
    pub fn el(&self, widget: &'static str) -> El {
        let mut i = self.0.borrow_mut();
        let id = i.tree.create(widget);
        let style = LayoutStyle::default();
        let node = i.layout.node(&style).unwrap_or_else(|_| unreachable_root());
        let base = defaults(&i.theme);
        let resolved = i.sheet.resolve(
            &StyleKey { widget, variant: "", states: States::NONE },
            &base,
            &StylePatch::default(),
        );
        i.els.insert(id, ElData::new(widget, style, node, resolved));
        i.tree.mark(id, Dirty::STYLE | Dirty::LAYOUT | Dirty::PAINT);
        El { ui: self.clone(), id }
    }

    /// The element tree, for events, focus and hit testing.
    pub fn tree(&self) -> Tree {
        self.0.borrow().tree.clone()
    }

    /// The theme.
    pub fn theme(&self) -> Theme {
        self.0.borrow().theme.clone()
    }

    /// Changes the theme and its style sheet; every element restyles.
    pub fn set_theme(&self, theme: Theme, sheet: StyleSheet) {
        let mut i = self.0.borrow_mut();
        i.theme = theme;
        i.sheet = sheet;
        let ids: Vec<ElementId> = i.els.keys().copied().collect();
        for id in ids {
            i.tree.mark(id, Dirty::STYLE);
        }
    }

    /// Requests for the host since the last call.
    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut self.0.borrow_mut().requests)
    }

    /// The milliseconds clock edits use to group undo steps; the runner advances it.
    pub fn set_time(&self, ms: u64) {
        self.0.borrow_mut().now_ms = ms;
    }

    /// The scroll offset the compositor reports for a scroll container, so hit testing
    /// matches what is shown.
    pub fn set_scroll_offset(&self, el: ElementId, offset: Vec2) {
        let mut i = self.0.borrow_mut();
        if i.scroll_offsets.get(&el) != Some(&offset) {
            i.scroll_offsets.insert(el, offset);
            i.frame.bounds_stale = true;
        }
    }

    /// Accessibility: start producing AccessKit updates (a screen reader connected).
    pub fn connect_accessibility(&self) {
        let mut i = self.0.borrow_mut();
        i.a11y.connect();
        i.structure_changed = true;
    }

    /// The next accessibility update, when connected and something changed.
    pub fn take_accessibility_update(&self) -> Option<remedy_ui_a11y::AccessKitUpdate> {
        self.0.borrow_mut().a11y.take_update()
    }

    /// Asks for a frame (the next frame redraws only what changed).
    pub fn request_frame(&self) {
        self.request(Request::Frame);
    }

    pub(crate) fn request(&self, r: Request) {
        let mut i = self.0.borrow_mut();
        if r == Request::Frame && i.requests.contains(&Request::Frame) {
            return;
        }
        i.requests.push(r);
    }

    pub(crate) fn mark(&self, id: ElementId, flags: Dirty) {
        let i = self.0.borrow();
        i.tree.mark(id, flags);
        drop(i);
        self.request(Request::Frame);
    }
}

pub(crate) fn root_key(id: ElementId) -> u64 {
    (u64::from(id.generation()) << 32) | u64::from(id.index())
}

fn unreachable_root() -> LayoutNode {
    // Creating a plain node on a fresh taffy tree cannot fail; this keeps the signature total.
    unreachable!("taffy refused a plain node")
}

impl ElData {
    fn new(widget: &'static str, layout: LayoutStyle, node: LayoutNode, resolved: ResolvedStyle) -> Self {
        ElData {
            widget,
            variant: "",
            states: States::NONE,
            local: StylePatch::default(),
            resolved,
            layout,
            node,
            text: None,
            field: None,
            painter: None,
            semantics: None,
            scroll: false,
            clip: false,
            click: None,
            chunk: None,
            effects: Vec::new(),
        }
    }
}

impl El {
    /// The element's id in the tree.
    pub fn id(&self) -> ElementId {
        self.id
    }

    /// The UI it belongs to.
    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    fn edit(&self, flags: Dirty, f: impl FnOnce(&mut ElData)) {
        {
            let mut i = self.ui.0.borrow_mut();
            if let Some(d) = i.els.get_mut(&self.id) {
                f(d);
            }
        }
        self.ui.mark(self.id, flags);
    }

    /// Runs `f` now and again whenever the signals it reads change, for as long as the
    /// element lives.
    pub fn bind(self, f: impl FnMut() + 'static) -> Self {
        let effect = Effect::new(f);
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.effects.push(effect);
        }
        self
    }

    /// Adds a child at the end.
    pub fn child(self, child: El) -> Self {
        {
            let mut i = self.ui.0.borrow_mut();
            let _ = i.tree.append(self.id, child.id);
            let nodes = (i.els.get(&self.id).map(|d| d.node), i.els.get(&child.id).map(|d| d.node));
            if let (Some(p), Some(c)) = nodes {
                let _ = i.layout.add_child(p, c);
            }
            i.structure_changed = true;
        }
        self.ui.mark(self.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        self
    }

    /// Adds children in order.
    pub fn children(self, children: impl IntoIterator<Item = El>) -> Self {
        children.into_iter().fold(self, El::child)
    }

    /// Removes the element and everything under it.
    pub fn remove(&self) {
        let ids = {
            let i = self.ui.0.borrow();
            let mut ids = i.tree.descendants(self.id);
            ids.push(self.id);
            ids
        };
        let parent = self.ui.0.borrow().tree.parent(self.id);
        let mut effects = Vec::new();
        {
            let mut i = self.ui.0.borrow_mut();
            for id in &ids {
                if let Some(d) = i.els.remove(id) {
                    let _ = i.layout.remove(d.node);
                    effects.extend(d.effects);
                }
                i.scroll_offsets.remove(id);
                i.hovered.retain(|h| h != id);
                if i.pressed == Some(*id) {
                    i.pressed = None;
                }
            }
            let _ = i.tree.remove(self.id);
            i.structure_changed = true;
        }
        for e in effects {
            e.dispose();
        }
        if let Some(p) = parent {
            self.ui.mark(p, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        }
    }

    /// Removes every child.
    pub fn clear(&self) {
        let kids = self.ui.0.borrow().tree.children(self.id);
        for k in kids {
            El { ui: self.ui.clone(), id: k }.remove();
        }
    }

    /// Sets the theme variant (`primary`, `danger`, …).
    pub fn variant(self, variant: &'static str) -> Self {
        self.edit(Dirty::STYLE, |d| d.variant = variant);
        self
    }

    /// Overrides style properties on this element.
    pub fn style(self, patch: StylePatch) -> Self {
        self.edit(Dirty::STYLE, |d| d.local = d.local.merge(&patch));
        self
    }

    /// Replaces the local style whenever `f`'s signals change.
    pub fn style_dyn(self, f: impl Fn() -> StylePatch + 'static) -> Self {
        let me = self.clone();
        self.bind(move || {
            let p = f();
            me.edit(Dirty::STYLE, |d| d.local = p);
        })
    }

    /// Edits the layout style.
    pub fn layout(self, f: impl FnOnce(&mut LayoutStyle)) -> Self {
        self.edit(Dirty::LAYOUT, |d| f(&mut d.layout));
        self
    }

    /// Lays children out in a row.
    pub fn row(self) -> Self {
        self.layout(|l| {
            l.display = remedy_ui_layout::Display::Flex;
            l.axis = remedy_ui_layout::Axis::Row;
        })
    }

    /// Lays children out in a column.
    pub fn column(self) -> Self {
        self.layout(|l| {
            l.display = remedy_ui_layout::Display::Flex;
            l.axis = remedy_ui_layout::Axis::Column;
        })
    }

    /// Space between children.
    pub fn gap(self, gap: f32) -> Self {
        self.layout(|l| l.gap = (gap, gap))
    }

    /// Fixed width in logical pixels.
    pub fn width(self, w: f32) -> Self {
        self.layout(|l| l.width = remedy_ui_layout::Length::Px(w))
    }

    /// Fixed height in logical pixels.
    pub fn height(self, h: f32) -> Self {
        self.layout(|l| l.height = remedy_ui_layout::Length::Px(h))
    }

    /// Takes a share of the free space along the parent's axis.
    pub fn grow(self, g: f32) -> Self {
        self.layout(|l| l.grow = g)
    }

    /// Fills the parent's width.
    pub fn fill_width(self) -> Self {
        self.layout(|l| l.width = remedy_ui_layout::Length::Percent(1.0))
    }

    /// How children sit across the axis.
    pub fn align(self, a: remedy_ui_layout::Align) -> Self {
        self.layout(|l| l.align_items = a)
    }

    /// How children spread along the axis.
    pub fn justify(self, j: remedy_ui_layout::Justify) -> Self {
        self.layout(|l| l.justify = j)
    }

    /// Calls `f` for key presses while the element or something inside it has focus; `f`
    /// returns true when it handled the key.
    pub fn on_key(self, f: impl Fn(&remedy_ui_input::KeyEvent) -> bool + 'static) -> Self {
        let _ = self.ui.tree().on::<remedy_ui_input::KeyEvent>(self.id, Phase::Bubble, move |cx, e| {
            if f(e) {
                cx.set_handled();
                cx.stop_propagation();
            }
        });
        self
    }

    /// Calls `f` with pointer input on the element (window logical pixels).
    pub fn on_pointer(self, f: impl Fn(&remedy_ui_input::PointerEvent) -> bool + 'static) -> Self {
        let _ = self.ui.tree().on::<remedy_ui_input::PointerEvent>(self.id, Phase::Bubble, move |cx, e| {
            if f(e) {
                cx.set_handled();
                cx.stop_propagation();
            }
        });
        self
    }

    /// Moves focus to the first focusable element inside this one (a dialog opening).
    /// Returns false when there is none.
    pub fn focus_first(&self) -> bool {
        let tree = self.ui.tree();
        let first = tree.descendants(self.id).into_iter().find(|d| tree.is_focusable(*d));
        match first {
            Some(f) => {
                El { ui: self.ui.clone(), id: f }.focus();
                true
            }
            None => false,
        }
    }

    /// Keeps Tab focus inside the element (modals, menus).
    pub fn focus_scope(self) -> Self {
        let _ = self.ui.tree().set_focus_scope(self.id, true);
        self
    }

    /// Draws `image` scaled into the element's content box, keeping its aspect ratio.
    pub fn image(self, image: remedy_ui_paint::Image) -> Self {
        let (w, h) = (image.width.max(1) as f32, image.height.max(1) as f32);
        self.layout(|l| {
            if l.aspect_ratio.is_none() {
                l.aspect_ratio = Some(w / h);
            }
        })
        .paint(move |cx| {
            let s = (cx.size.width / w).min(cx.size.height / h);
            let (dw, dh) = (w * s, h * s);
            let x = (cx.size.width - dw) / 2.0;
            let y = (cx.size.height - dh) / 2.0;
            cx.list.push(remedy_ui_paint::Item::Image {
                image: image.clone(),
                rect: remedy_ui_core::Rect::new(x, y, x + dw, y + dh),
                radii: remedy_ui_core::CornerRadii::ZERO,
                sampling: remedy_ui_paint::Sampling::Linear,
            });
        })
    }

    /// Padding (a style property, so themes can change it).
    pub fn padding(self, p: Insets) -> Self {
        self.style(StylePatch { padding: Some(p), ..StylePatch::default() })
    }

    /// Sets a state flag (selected, checked, invalid, …).
    pub fn state(&self, s: States, on: bool) {
        self.edit(Dirty::STYLE, |d| d.states = d.states.with(s, on));
    }

    /// The element's current states.
    pub fn states(&self) -> States {
        self.ui.0.borrow().els.get(&self.id).map_or(States::NONE, |d| d.states)
    }

    /// Disables the element: no pointer or keyboard interaction, and the disabled look.
    pub fn disabled(self, on: bool) -> Self {
        self.state(States::DISABLED, on);
        let tree = self.ui.tree();
        let _ = tree.set_hit_mode(self.id, if on { HitMode::None } else { HitMode::Normal });
        self
    }

    /// Binds the disabled state to `f`.
    pub fn disabled_dyn(self, f: impl Fn() -> bool + 'static) -> Self {
        let me = self.clone();
        self.bind(move || {
            let on = f();
            let _ = me.clone().disabled(on);
        })
    }

    /// Makes the element a text element showing `text`.
    pub fn text(self, text: impl Into<RichText>) -> Self {
        let text = text.into();
        let engine_scale = self.ui.0.borrow().scale;
        let slot = {
            let i = self.ui.0.borrow();
            i.els.get(&self.id).and_then(|d| d.text.clone())
        };
        match slot {
            Some(s) => {
                let mut s = s.borrow_mut();
                s.text = text;
                s.cache = None;
            }
            None => {
                let slot = Rc::new(RefCell::new(TextSlot {
                    text,
                    style: TextStyle::default(),
                    wrap: true,
                    scale: engine_scale,
                    cache: None,
                }));
                self.make_measured(slot.clone());
                let mut i = self.ui.0.borrow_mut();
                if let Some(d) = i.els.get_mut(&self.id) {
                    d.text = Some(slot);
                }
            }
        }
        let node = self.ui.0.borrow().els.get(&self.id).map(|d| d.node);
        if let Some(n) = node {
            let _ = self.ui.0.borrow_mut().layout.mark_dirty(n);
        }
        self.ui.mark(self.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        self
    }

    /// Shows the text `f` returns, updated when its signals change.
    pub fn text_dyn(self, f: impl Fn() -> String + 'static) -> Self {
        let me = self.clone();
        self.bind(move || {
            let t = f();
            let _ = me.clone().text(RichText::plain(t));
        })
    }

    /// Keeps the text on one line instead of wrapping.
    pub fn nowrap(self) -> Self {
        if let Some(s) = self.ui.0.borrow().els.get(&self.id).and_then(|d| d.text.clone()) {
            let mut s = s.borrow_mut();
            s.wrap = false;
            s.cache = None;
        }
        self.edit(Dirty::LAYOUT, |_| {});
        self
    }

    /// Replaces the layout node with one measured by `slot`'s paragraph.
    pub(crate) fn make_measured(&self, slot: Rc<RefCell<TextSlot>>) {
        let engine = self.ui.0.borrow().engine.clone();
        let mut i = self.ui.0.borrow_mut();
        let Some(style) = i.els.get(&self.id).map(|d| d.layout.clone()) else { return };
        let measure = move |m: MeasureInput| {
            let width = m.known_width.or(match m.available_width {
                Available::Px(w) => Some(w),
                Available::MinContent => Some(0.0),
                Available::MaxContent => None,
            });
            let p = slot.borrow_mut().paragraph(&mut engine.borrow_mut(), width);
            Size::new(p.width().ceil(), p.height().ceil())
        };
        let Ok(node) = i.layout.measured(&style, measure) else { return };
        let parent = i.tree.parent(self.id).and_then(|p| i.els.get(&p).map(|d| d.node));
        let old = i.els.get(&self.id).map(|d| d.node);
        if let (Some(p), Some(old)) = (parent, old) {
            let kids: Vec<LayoutNode> =
                i.layout.children(p).into_iter().map(|k| if k == old { node } else { k }).collect();
            let _ = i.layout.set_children(p, &kids);
        }
        if let Some(old) = old {
            let _ = i.layout.remove(old);
        }
        if let Some(d) = i.els.get_mut(&self.id) {
            d.node = node;
        }
    }

    /// Draws with `f` after the element's background and border.
    pub fn paint(self, f: impl Fn(&mut PaintCx<'_>) + 'static) -> Self {
        let f: Painter = Rc::new(f);
        self.edit(Dirty::PAINT, |d| d.painter = Some(f));
        self
    }

    /// Calls `f` on click: a primary press and release on the element, or Enter or Space
    /// while it has focus. Makes the element focusable.
    pub fn on_click(self, f: impl Fn() + 'static) -> Self {
        let f: Rc<dyn Fn()> = Rc::new(f);
        let tree = self.ui.tree();
        let _ = tree.set_focusable(self.id, true);
        let g = f.clone();
        let _ = tree.on::<Click>(self.id, Phase::Bubble, move |cx, _| {
            g();
            cx.stop_propagation();
        });
        self.edit(Dirty::SEMANTICS, |d| d.click = Some(f));
        self
    }

    /// Lets the element take keyboard focus.
    pub fn focusable(self, on: bool) -> Self {
        let _ = self.ui.tree().set_focusable(self.id, on);
        self
    }

    /// Moves keyboard focus here.
    pub fn focus(&self) {
        let tree = self.ui.tree();
        let _ = tree.focus(self.id);
        crate::events::focus_changed(&self.ui);
    }

    /// Describes the element to assistive technology.
    pub fn semantics(self, s: Semantics) -> Self {
        self.edit(Dirty::SEMANTICS, |d| d.semantics = Some(s));
        self
    }

    /// Updates semantics when `f`'s signals change.
    pub fn semantics_dyn(self, f: impl Fn() -> Semantics + 'static) -> Self {
        let me = self.clone();
        self.bind(move || {
            let s = f();
            me.edit(Dirty::SEMANTICS, |d| d.semantics = Some(s));
        })
    }

    /// Makes the element a vertical scroll container: its content scrolls on the compositor.
    pub fn scroll(self) -> Self {
        self.edit(Dirty::LAYOUT | Dirty::PAINT, |d| {
            d.scroll = true;
            d.clip = true;
            d.layout.max_height = remedy_ui_layout::Length::Percent(1.0);
        });
        let _ = self.ui.tree().set_clips(self.id, true);
        self
    }

    /// Clips children to the element's box.
    pub fn clip(self) -> Self {
        let _ = self.ui.tree().set_clips(self.id, true);
        self.edit(Dirty::PAINT, |d| d.clip = true);
        self
    }

    /// The element's bounds in window logical pixels after the last frame.
    pub fn bounds(&self) -> Option<remedy_ui_core::Rect> {
        self.ui.tree().bounds(self.id)
    }

    /// Whether the element has keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.ui.tree().focused() == Some(self.id)
    }
}
