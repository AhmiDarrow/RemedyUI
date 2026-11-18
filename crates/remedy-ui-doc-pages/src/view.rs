//! The page view: a document's pages laid out virtually, drawn from tiles rendered off the UI
//! thread at the zoom shown, zoomed and scrolled by pointer, touch, wheel and keyboard.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_input::{
    Buttons, Key, KeyEvent, PinchEvent, PinchPhase, PointerEvent, PointerKind, PointerPhase, WheelEvent,
};
use remedy_ui_paint::{Affine, Brush, Item, Sampling};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{ElementId, PaintCx, WeakUi};

use crate::annot::{self, Annotation, AnnotationKind, AnnotationStyle, Shape, Tool};
use crate::layout::{Fit, PageLayout, Rotation, Spread};
use crate::raster::{
    self, Done, Draw, FrameCx, PageStats, TileId, Tiles, Visible, class, level_of, scale_of,
};
use crate::source::{Cancel, PageSource, PageText};
use crate::text::{self, Match, SearchStatus, TextPos, TextState};

/// The smallest zoom: 1%.
pub const MIN_ZOOM: f32 = 0.01;
/// The largest zoom: 6400%.
pub const MAX_ZOOM: f32 = 64.0;

/// The zooms Ctrl+Plus and Ctrl+Minus step through, as other document viewers do.
const ZOOM_STEPS: [f32; 27] = [
    0.01, 0.02, 0.03, 0.05, 0.08, 0.12, 0.17, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0,
    3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 64.0,
];

/// How long after the last scroll or zoom change the view counts as settled: a pinch, a drag
/// or a run of wheel notches is one gesture. Until then the compositor moves and scales the
/// view's last picture and nothing new renders; then the view redraws at the new zoom.
pub const SETTLE: Duration = Duration::from_millis(150);

/// One wheel notch (48 logical pixels on desktop bridges) zooms by 10%.
const WHEEL_ZOOM_PER_PIXEL: f32 = 0.001_985_6; // ln(1.1) / 48

/// Arrow keys scroll by one wheel notch.
const KEY_SCROLL: f32 = 48.0;

/// Scroll bars: width, and the shortest thumb (a target a pointer can hold).
const BAR: f32 = 8.0;
const BAR_MIN: f32 = 24.0;

/// The gap between pages and the margin around them, logical pixels, unless the app sets
/// its own ([`PageView::set_spacing`]).
const GAP: f32 = 8.0;
const MARGIN: f32 = 16.0;

/// A page view's share of the machine's memory for tiles: 1/64 (256 MB on a 16 GB machine).
/// Never less than three screens of tiles (the shown scale, a stand-in scale, prefetch).
const MEMORY_SHARE: u64 = 64;

#[derive(Clone, Copy, Debug)]
enum Gesture {
    None,
    Pan {
        last: Point,
    },
    Select,
    Bar {
        vertical: bool,
        grab: f32,
    },
    Pinch {
        distance: f32,
        zoom: f32,
        mid: Point,
    },
    /// Making or moving an annotation (what is in [`State::drawing`]).
    Annotate,
}

/// An annotation being made or moved by a drag, in its page's points.
#[derive(Clone, Debug)]
enum Drawing {
    Ink { page: usize, points: Vec<Point> },
    Shape { page: usize, shape: Shape, from: Point, to: Point },
    Text { page: usize, from: Point, to: Point },
    Move { id: u64, last: Point, before: Vec<Annotation>, moved: bool },
}

/// While a scroll or zoom moves, the view keeps its last picture and the compositor moves
/// and scales it (progressively re-rastering within its budget); the picture is redrawn when
/// the movement settles, or sooner when it no longer covers the view.
#[derive(Clone, Copy, Debug)]
struct Frozen {
    /// The zoom and scroll the picture shows.
    zoom: f32,
    scroll: Vec2,
}

struct State {
    frozen: Option<Frozen>,
    /// The area the latest picture covers, in the view's coordinates when it was drawn.
    drawn: Rect,
    layout: PageLayout,
    spread: Spread,
    rotation: Rotation,
    gap: f32,
    margin: f32,
    zoom: f32,
    fit: Fit,
    scroll: Vec2,
    viewport: Size,
    dpr: f32,
    tiles: Tiles,
    text: TextState,
    moved_at: Option<Instant>,
    settle_timer: Option<u64>,
    gesture: Gesture,
    touches: HashMap<u32, Point>,
    thumb_flight: HashMap<usize, Cancel>,
    annots: Vec<Annotation>,
    next_annot: u64,
    tool: Tool,
    style: AnnotationStyle,
    picked: Option<u64>,
    drawing: Option<Drawing>,
}

/// The app's handler for opening an annotation (to read or edit its comment).
type OpenHandler = Rc<dyn Fn(Annotation)>;

struct Shared {
    viewer: u64,
    source: Arc<dyn PageSource>,
    sizes: Rc<Vec<Size>>,
    st: RefCell<State>,
    zoom: Signal<f32>,
    fit: Signal<Fit>,
    page: Signal<usize>,
    page_text: Signal<String>,
    search: Signal<SearchStatus>,
    host: RefCell<Option<(WeakUi, ElementId)>>,
    thumbs: RefCell<HashMap<usize, (WeakUi, ElementId)>>,
    annotations: Signal<Vec<Annotation>>,
    tool: Signal<Tool>,
    picked: Signal<Option<u64>>,
    on_open: RefCell<Option<OpenHandler>>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        raster::unregister(self.viewer);
        let st = self.st.get_mut();
        st.tiles.clear();
        for (_, c) in st.thumb_flight.drain() {
            c.cancel();
        }
        if let Some(c) = st.text.search.cancel.take() {
            c.cancel();
        }
    }
}

/// A paged document on screen: its zoom, scroll, page, selection and search. Clone it to
/// hand it to the widgets that show it ([`page_view`](crate::page_view),
/// [`thumbnails`](crate::thumbnails)) and to the app's own controls.
///
/// Create it on the UI thread.
#[derive(Clone)]
pub struct PageView(Rc<Shared>);

impl std::fmt::Debug for PageView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.0.st.borrow();
        write!(f, "PageView({} pages, zoom {}, {:?})", st.layout.len(), st.zoom, st.tiles.stats)
    }
}

impl PageView {
    /// A view of `source`'s pages, fitted to the width, one page per row.
    pub fn new(source: Arc<dyn PageSource>) -> PageView {
        let n = source.page_count();
        let sizes: Vec<Size> = (0..n).map(|p| source.page_size(p)).collect();
        let layout = PageLayout::new(sizes.clone(), Spread::Single, Rotation::None, GAP, MARGIN);
        let shared = Rc::new_cyclic(|weak: &Weak<Shared>| {
            let w = weak.clone();
            let viewer = raster::register(Rc::new(move |done| {
                if let Some(s) = w.upgrade() {
                    PageView(s).deliver(done);
                }
            }));
            let w = weak.clone();
            remedy_ui_view::device::on_memory_pressure(move |level| {
                if let Some(s) = w.upgrade()
                    && level != remedy_ui_view::device::MemoryPressure::Normal
                {
                    s.st.borrow_mut().tiles.release_unused();
                }
            });
            Shared {
                viewer,
                source,
                sizes: Rc::new(sizes),
                st: RefCell::new(State {
                    frozen: None,
                    drawn: Rect::ZERO,
                    layout,
                    spread: Spread::Single,
                    rotation: Rotation::None,
                    gap: GAP,
                    margin: MARGIN,
                    zoom: 1.0,
                    fit: Fit::Width,
                    scroll: Vec2::ZERO,
                    viewport: Size::ZERO,
                    dpr: 1.0,
                    tiles: Tiles::new(),
                    text: TextState::default(),
                    moved_at: None,
                    settle_timer: None,
                    gesture: Gesture::None,
                    touches: HashMap::new(),
                    thumb_flight: HashMap::new(),
                    annots: Vec::new(),
                    next_annot: 1,
                    tool: Tool::Select,
                    style: AnnotationStyle::default(),
                    picked: None,
                    drawing: None,
                }),
                zoom: Signal::new(1.0),
                fit: Signal::new(Fit::Width),
                page: Signal::new(0),
                page_text: Signal::new(String::new()),
                search: Signal::new(SearchStatus::default()),
                host: RefCell::new(None),
                thumbs: RefCell::new(HashMap::new()),
                annotations: Signal::new(Vec::new()),
                tool: Signal::new(Tool::Select),
                picked: Signal::new(None),
                on_open: RefCell::new(None),
            }
        });
        PageView(shared)
    }

    /// Number of pages.
    pub fn page_count(&self) -> usize {
        self.0.sizes.len()
    }

    /// The zoom (logical pixels per point; 1.0 is 100%). Read it in bindings; change it
    /// with [`set_zoom`](Self::set_zoom).
    pub fn zoom(&self) -> Signal<f32> {
        self.0.zoom
    }

    /// How the zoom follows the window.
    pub fn fit(&self) -> Signal<Fit> {
        self.0.fit
    }

    /// The page in the middle of the view, from 0.
    pub fn current_page(&self) -> Signal<usize> {
        self.0.page
    }

    /// Where the search is.
    pub fn search_status(&self) -> Signal<SearchStatus> {
        self.0.search
    }

    /// What the view's work has cost so far.
    pub fn stats(&self) -> PageStats {
        self.0.st.borrow().tiles.stats
    }

    /// True when the latest frame showed every visible tile at the zoom's own scale and
    /// nothing is rendering.
    pub fn is_sharp(&self) -> bool {
        let st = self.0.st.borrow();
        // Every tile on screen is counted as shown exactly, standing in, or blank.
        st.frozen.is_none()
            && !st.tiles.busy()
            && st.tiles.stats.standing_in == 0
            && st.tiles.stats.blank == 0
    }

    /// The scroll offset, logical pixels from the top-left of the content.
    pub fn scroll_offset(&self) -> Vec2 {
        self.0.st.borrow().scroll
    }

    /// The size of everything at the current zoom, logical pixels.
    pub fn content_size(&self) -> Size {
        let st = self.0.st.borrow();
        let c = st.layout.content_size(st.eff_zoom());
        Size::new(c.width.max(st.viewport.width), c.height)
    }

    /// Where a page shows, in the view's own logical pixels (after scrolling).
    pub fn page_rect(&self, page: usize) -> Rect {
        let st = self.0.st.borrow();
        st.layout
            .page_rect(page, st.eff_zoom(), st.viewport.width)
            .translate(Vec2::new(-st.scroll.x, -st.scroll.y))
    }

    fn host(&self) -> Option<remedy_ui_view::El> {
        let h = self.0.host.borrow();
        let (ui, id) = h.as_ref()?;
        ui.upgrade().map(|ui| ui.element(*id))
    }

    fn repaint(&self) {
        if let Some(el) = self.host() {
            el.repaint();
        }
    }

    /// Sets the zoom (clamped to 1%–6400%) keeping what is at `anchor` (the view's own
    /// logical pixels; `None`: its middle) where it is. Stops fitting.
    pub fn set_zoom(&self, zoom: f32, anchor: Option<Point>) {
        self.zoom_to(zoom, anchor, Fit::Free);
    }

    fn zoom_to(&self, zoom: f32, anchor: Option<Point>, fit: Fit) {
        if !zoom.is_finite() {
            return;
        }
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let gesture = fit == Fit::Free;
        if gesture {
            self.freeze();
        }
        {
            let mut st = self.0.st.borrow_mut();
            let anchor = anchor.unwrap_or(Point::new(st.viewport.width / 2.0, st.viewport.height / 2.0));
            let vw = st.viewport.width;
            let before = st.eff_zoom();
            let at = Point::new(st.scroll.x + anchor.x, st.scroll.y + anchor.y);
            let hit = st.layout.page_at(at, before, vw);
            let old_level = level_of(before * st.dpr);
            st.zoom = zoom;
            st.fit = fit;
            let after = st.eff_zoom();
            if let Some((page, p)) = hit {
                let c = st.layout.page_transform(page, after, vw).apply(p);
                st.scroll = Vec2::new(c.x - anchor.x, c.y - anchor.y);
            }
            if level_of(after * st.dpr) != old_level {
                st.moved_at = Some(Instant::now());
            }
            st.clamp();
        }
        self.settle_later();
        self.changed();
    }

    /// Starts a moving gesture (wheel, pinch, drag, keys): the picture stays, and the
    /// compositor moves it, until the movement settles.
    fn freeze(&self) {
        if self.host().is_none() {
            return;
        }
        let mut st = self.0.st.borrow_mut();
        if st.frozen.is_none() && st.drawn.width() > 0.0 {
            st.frozen = Some(Frozen { zoom: st.zoom, scroll: st.scroll });
        }
    }

    /// Ends a gesture: the picture is redrawn at the live zoom and scroll.
    fn thaw(&self) {
        let was = self.0.st.borrow_mut().frozen.take();
        if was.is_some()
            && let Some(el) = self.host()
        {
            el.set_layer_transform(Affine::IDENTITY);
            el.repaint();
        }
    }

    /// Repaints once the zoom settles, so the exact scale renders.
    fn settle_later(&self) {
        let Some(el) = self.host() else { return };
        let ui = el.ui().clone();
        if let Some(t) = self.0.st.borrow_mut().settle_timer.take() {
            ui.cancel_timer(t);
        }
        let me = Rc::downgrade(&self.0);
        let id = ui.after(SETTLE, move || {
            if let Some(s) = me.upgrade() {
                s.st.borrow_mut().settle_timer = None;
                let v = PageView(s);
                v.thaw();
                v.repaint();
            }
        });
        self.0.st.borrow_mut().settle_timer = Some(id);
    }

    /// Zooms in one step (Ctrl+Plus).
    pub fn zoom_in(&self) {
        let z = self.0.st.borrow().zoom;
        let next = ZOOM_STEPS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(MAX_ZOOM);
        self.set_zoom(next, self.caret_anchor());
    }

    /// Zooms out one step (Ctrl+Minus).
    pub fn zoom_out(&self) {
        let z = self.0.st.borrow().zoom;
        let next = ZOOM_STEPS.iter().rev().copied().find(|s| *s < z / 1.001).unwrap_or(MIN_ZOOM);
        self.set_zoom(next, self.caret_anchor());
    }

    /// Fits the zoom to the window (and keeps fitting as it resizes).
    pub fn set_fit(&self, fit: Fit) {
        let z = {
            let st = self.0.st.borrow();
            st.layout.fit_zoom(fit, st.viewport, self.0.page.get_untracked().unwrap_or(0))
        };
        match z {
            Some(z) => {
                let top = Point::new(self.0.st.borrow().viewport.width / 2.0, 0.0);
                self.zoom_to(z, Some(top), fit);
            }
            None => {
                self.0.st.borrow_mut().fit = fit;
                let _ = self.0.fit.try_set(fit);
            }
        }
    }

    /// Sets how pages sit side by side, keeping the current page in view.
    pub fn set_spread(&self, spread: Spread) {
        let page = self.0.page.get_untracked().unwrap_or(0);
        {
            let mut st = self.0.st.borrow_mut();
            st.spread = spread;
            st.relayout(&self.0.sizes);
        }
        self.refit();
        self.go_to_page(page);
    }

    /// Turns every page, keeping the current page in view.
    pub fn set_rotation(&self, rotation: Rotation) {
        let page = self.0.page.get_untracked().unwrap_or(0);
        {
            let mut st = self.0.st.borrow_mut();
            st.rotation = rotation;
            st.relayout(&self.0.sizes);
        }
        self.refit();
        self.go_to_page(page);
    }

    /// The rotation.
    pub fn rotation(&self) -> Rotation {
        self.0.st.borrow().rotation
    }

    /// The spread.
    pub fn spread(&self) -> Spread {
        self.0.st.borrow().spread
    }

    /// Sets the gap between pages and the margin around them, logical pixels.
    pub fn set_spacing(&self, gap: f32, margin: f32) {
        {
            let mut st = self.0.st.borrow_mut();
            st.gap = gap.max(0.0);
            st.margin = margin.max(0.0);
            st.relayout(&self.0.sizes);
        }
        self.refit();
        self.changed();
    }

    fn refit(&self) {
        self.thaw();
        let fit = self.0.st.borrow().fit;
        if fit != Fit::Free {
            self.set_fit(fit);
        }
    }

    /// Scrolls so `page` starts at the top of the view.
    pub fn go_to_page(&self, page: usize) {
        self.thaw();
        {
            let mut st = self.0.st.borrow_mut();
            let page = page.min(st.layout.len().saturating_sub(1));
            let z = st.eff_zoom();
            let r = st.layout.page_rect(page, z, st.viewport.width);
            let row_top = st.layout.row_top(st.layout.row_of(page), z);
            st.scroll.y = row_top.min(r.min.y) - st.margin;
            // Keep the page across when it is narrower than the view.
            if r.width() <= st.viewport.width {
                st.scroll.x = 0.0;
            } else {
                st.scroll.x = r.min.x - st.margin;
            }
            st.clamp();
        }
        self.changed();
    }

    /// Scrolls by `delta` logical pixels.
    pub fn scroll_by(&self, delta: Vec2) {
        self.freeze();
        {
            let mut st = self.0.st.borrow_mut();
            st.scroll = Vec2::new(st.scroll.x + delta.x, st.scroll.y + delta.y);
            st.clamp();
        }
        self.settle_later();
        self.changed();
    }

    /// After a move: keeps signals in step and redraws.
    fn changed(&self) {
        let (zoom, fit, page) = {
            let st = self.0.st.borrow();
            let c = Point::new(st.scroll.x + st.viewport.width / 2.0, st.scroll.y + st.viewport.height / 2.0);
            let page = st.layout.page_at(c, st.eff_zoom(), st.viewport.width).map_or(0, |(p, _)| p);
            (st.zoom, st.fit, page)
        };
        let _ = self.0.zoom.try_set(zoom);
        let _ = self.0.fit.try_set(fit);
        if self.0.page.get_untracked().ok() != Some(page) {
            let _ = self.0.page.try_set(page);
            self.update_page_text();
            self.repaint_thumbs();
        }
        // While frozen the compositor moves the picture, unless it no longer covers the view.
        let moved = self.0.st.borrow().frozen_transform();
        match moved {
            Some(Some(t)) => {
                if let Some(el) = self.host() {
                    el.set_layer_transform(t);
                }
            }
            Some(None) => self.thaw(),
            None => self.repaint(),
        }
    }

    fn update_page_text(&self) {
        let page = self.0.page.get_untracked().unwrap_or(0);
        let known = self.0.st.borrow().text.pages.get(&page).cloned();
        let notes = {
            let st = self.0.st.borrow();
            let mut lines: Vec<String> = st
                .annots
                .iter()
                .filter(|a| a.page == page)
                .map(|a| {
                    if st.picked == Some(a.id) { format!("Selected: {}", a.describe()) } else { a.describe() }
                })
                .collect();
            if !lines.is_empty() {
                lines.insert(
                    0,
                    format!("{} annotation{}:", lines.len(), if lines.len() == 1 { "" } else { "s" }),
                );
            }
            lines.join("\n")
        };
        let with_notes = |body: String| {
            if notes.is_empty() {
                body
            } else if body.is_empty() {
                notes.clone()
            } else {
                format!("{body}\n\n{notes}")
            }
        };
        match known {
            Some(Some(t)) => {
                let _ = self.0.page_text.try_set(with_notes(text::whole(&t)));
            }
            Some(None) => {
                let _ = self.0.page_text.try_set(with_notes(String::new()));
            }
            None => {
                let _ = self.0.page_text.try_set(with_notes(String::new()));
                self.request_text(page);
            }
        }
    }

    fn request_text(&self, page: usize) {
        let mut st = self.0.st.borrow_mut();
        if st.text.pages.contains_key(&page) || !st.text.pending.insert(page) {
            return;
        }
        drop(st);
        let source = self.0.source.clone();
        let p = Arc::new(std::sync::atomic::AtomicU64::new(raster::priority(class::TEXT, page as f32)));
        raster::run(self.0.viewer, Cancel::new(), p, move |_| Done::Text(page, source.text(page)));
    }

    /// The caret's place in the view, when there is a selection showing; zooms by keyboard
    /// keep it still.
    fn caret_anchor(&self) -> Option<Point> {
        let st = self.0.st.borrow();
        let (_, focus) = st.text.selection?;
        let Some(Some(t)) = st.text.pages.get(&focus.page) else { return None };
        let run = t.runs.get(focus.run)?;
        let r = run.char_rect(focus.offset.min(run.len().saturating_sub(1)));
        let p = st.to_view(focus.page, r.center());
        let inside = p.x >= 0.0 && p.y >= 0.0 && p.x <= st.viewport.width && p.y <= st.viewport.height;
        inside.then_some(p)
    }

    // ---------- results from the workers ----------

    fn deliver(&self, done: Done) {
        match done {
            Done::Tile(id, result) => {
                {
                    let mut st = self.0.st.borrow_mut();
                    let size = self.0.sizes.get(id.page).copied().unwrap_or(Size::ZERO);
                    let tile = st.tile_edge();
                    st.tiles.adopt(id, result, tile, size);
                    st.thumb_flight.remove(&id.page);
                }
                // A moving picture is redrawn when it settles, with what arrived meanwhile.
                if self.0.st.borrow().frozen.is_none() {
                    self.repaint();
                }
                let thumb = self.0.thumbs.borrow().get(&id.page).cloned();
                if let Some((ui, el)) = thumb
                    && let Some(ui) = ui.upgrade()
                {
                    ui.element(el).repaint();
                }
            }
            Done::Text(page, t) => {
                {
                    let mut st = self.0.st.borrow_mut();
                    st.text.pending.remove(&page);
                    st.text.pages.insert(page, t.map(Arc::new));
                }
                if self.0.page.get_untracked().ok() == Some(page) {
                    self.update_page_text();
                }
                self.repaint();
            }
            Done::Found(generation, page, t) => self.found(generation, page, t),
        }
    }

    // ---------- annotations ----------

    /// The document's annotations, in the order they were added (later ones draw on top).
    /// The app reads this to save them; it changes with every edit.
    pub fn annotations(&self) -> Signal<Vec<Annotation>> {
        self.0.annotations
    }

    /// The annotation with `id`.
    pub fn annotation(&self, id: u64) -> Option<Annotation> {
        self.0.st.borrow().annots.iter().find(|a| a.id == id).cloned()
    }

    /// Replaces every annotation, as when the app opens a document. Annotations without an
    /// id (0) are given one. Not an edit: nothing goes into the undo history.
    pub fn set_annotations(&self, list: Vec<Annotation>) {
        {
            let mut st = self.0.st.borrow_mut();
            let mut next =
                list.iter().map(|a| a.id).max().unwrap_or(0).max(st.next_annot.saturating_sub(1)) + 1;
            st.annots = list
                .into_iter()
                .map(|mut a| {
                    if a.id == 0 {
                        a.id = next;
                        next += 1;
                    }
                    a
                })
                .collect();
            st.next_annot = next;
            if st.picked.is_some_and(|p| !st.annots.iter().any(|a| a.id == p)) {
                st.picked = None;
            }
        }
        self.publish_annotations();
    }

    /// Adds `a` (on its page, in page points) and returns its id. Undoable.
    pub fn add_annotation(&self, a: Annotation) -> u64 {
        let before = self.0.st.borrow().annots.clone();
        let id = {
            let mut st = self.0.st.borrow_mut();
            let id = st.next_annot;
            st.next_annot += 1;
            let mut a = a;
            a.id = id;
            st.annots.push(a);
            id
        };
        let label = self
            .annotation(id)
            .map_or("Add annotation".to_owned(), |a| format!("Add {}", a.kind_name().to_lowercase()));
        self.commit(label, before);
        id
    }

    /// Changes the annotation with `id` through `f`. Undoable. False when there is none.
    pub fn update_annotation(&self, id: u64, f: impl FnOnce(&mut Annotation)) -> bool {
        let before = self.0.st.borrow().annots.clone();
        let label = {
            let mut st = self.0.st.borrow_mut();
            let Some(a) = st.annots.iter_mut().find(|a| a.id == id) else { return false };
            f(a);
            a.id = id;
            format!("Change {}", a.kind_name().to_lowercase())
        };
        self.commit(label, before);
        true
    }

    /// Removes the annotation with `id`. Undoable. False when there is none.
    pub fn remove_annotation(&self, id: u64) -> bool {
        let before = self.0.st.borrow().annots.clone();
        let label = {
            let mut st = self.0.st.borrow_mut();
            let Some(i) = st.annots.iter().position(|a| a.id == id) else { return false };
            let a = st.annots.remove(i);
            if st.picked == Some(id) {
                st.picked = None;
            }
            format!("Delete {}", a.kind_name().to_lowercase())
        };
        self.commit(label, before);
        true
    }

    /// What pressing and dragging on the pages does.
    pub fn tool(&self) -> Signal<Tool> {
        self.0.tool
    }

    /// Chooses what pressing and dragging does. A text tool marks the text already selected,
    /// if any.
    pub fn set_tool(&self, tool: Tool) {
        self.0.st.borrow_mut().tool = tool;
        let _ = self.0.tool.try_set(tool);
        if matches!(tool, Tool::Highlight | Tool::Underline | Tool::Strikeout | Tool::Squiggly) {
            self.mark_selection(tool);
        }
        self.repaint();
    }

    /// How new annotations look.
    pub fn annotation_style(&self) -> AnnotationStyle {
        self.0.st.borrow().style
    }

    /// Sets how new annotations look.
    pub fn set_annotation_style(&self, style: AnnotationStyle) {
        self.0.st.borrow_mut().style = style;
    }

    /// The selected annotation.
    pub fn selected_annotation(&self) -> Signal<Option<u64>> {
        self.0.picked
    }

    /// Selects the annotation `id` (or none) and brings it into view.
    pub fn select_annotation(&self, id: Option<u64>) {
        let target = {
            let mut st = self.0.st.borrow_mut();
            st.picked = id.filter(|i| st.annots.iter().any(|a| a.id == *i));
            st.picked.and_then(|i| st.annots.iter().find(|a| a.id == i)).map(|a| (a.page, a.bounds()))
        };
        let _ = self.0.picked.try_set(target.and(id));
        if let Some((page, b)) = target {
            self.reveal(page, b);
        }
        self.publish_annotations();
    }

    /// Selects the next (or previous) annotation in reading order: by page, then top to
    /// bottom, then left to right.
    pub fn select_next_annotation(&self, forward: bool) -> bool {
        let order: Vec<u64> = {
            let st = self.0.st.borrow();
            let mut v: Vec<&Annotation> = st.annots.iter().collect();
            v.sort_by(|a, b| {
                let (ba, bb) = (a.bounds(), b.bounds());
                a.page.cmp(&b.page).then(ba.min.y.total_cmp(&bb.min.y)).then(ba.min.x.total_cmp(&bb.min.x))
            });
            v.into_iter().map(|a| a.id).collect()
        };
        if order.is_empty() {
            return false;
        }
        let at = self.0.st.borrow().picked.and_then(|p| order.iter().position(|i| *i == p));
        let next = match (at, forward) {
            (None, true) => order.first(),
            (None, false) => order.last(),
            (Some(i), true) => order.get((i + 1) % order.len()),
            (Some(i), false) => order.get((i + order.len() - 1) % order.len()),
        };
        self.select_annotation(next.copied());
        true
    }

    /// Calls `f` when someone opens an annotation (double-click, or Enter on the selected
    /// one, or placing a note): the app shows its comment for reading or editing.
    pub fn on_open_annotation(&self, f: impl Fn(Annotation) + 'static) {
        *self.0.on_open.borrow_mut() = Some(Rc::new(f));
    }

    /// Marks the selected text with a text tool's mark (one annotation per page it spans),
    /// clearing the selection. False with nothing selected or a tool that does not mark text.
    pub fn mark_selection(&self, tool: Tool) -> bool {
        let make = |quads: Vec<Rect>| match tool {
            Tool::Highlight => Some(AnnotationKind::Highlight { quads }),
            Tool::Underline => Some(AnnotationKind::Underline { quads }),
            Tool::Strikeout => Some(AnnotationKind::Strikeout { quads }),
            Tool::Squiggly => Some(AnnotationKind::Squiggly { quads }),
            _ => None,
        };
        if make(Vec::new()).is_none() {
            return false;
        }
        let (marks, color) = {
            let st = self.0.st.borrow();
            let Some((a, b)) = st.text.selection else { return false };
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let mut marks = Vec::new();
            for page in lo.page..=hi.page {
                let Some(Some(t)) = st.text.pages.get(&page) else { continue };
                let quads = text::rects(page, t, lo, hi);
                if quads.is_empty() {
                    continue;
                }
                let from = if page == lo.page { lo } else { TextPos { page, run: 0, offset: 0 } };
                let to = if page == hi.page {
                    hi
                } else {
                    let last = t.runs.len().saturating_sub(1);
                    TextPos { page, run: last, offset: t.runs.get(last).map_or(0, |r| r.len()) }
                };
                marks.push((page, quads, text::between(&st.text.pages, from, to)));
            }
            (marks, st.style.color)
        };
        if marks.is_empty() {
            return false;
        }
        let before = self.0.st.borrow().annots.clone();
        let label = {
            let mut st = self.0.st.borrow_mut();
            let mut name = String::new();
            for (page, quads, words) in marks {
                let Some(kind) = make(quads) else { continue };
                let mut a = Annotation::new(page, kind, color);
                a.marked_text = words;
                a.id = st.next_annot;
                st.next_annot += 1;
                name = format!("Add {}", a.kind_name().to_lowercase());
                st.annots.push(a);
            }
            st.text.selection = None;
            name
        };
        self.commit(label, before);
        true
    }

    /// Shows the annotations' current list, redraws, and updates what screen readers hear.
    fn publish_annotations(&self) {
        let (list, picked) = {
            let st = self.0.st.borrow();
            (st.annots.clone(), st.picked)
        };
        let _ = self.0.annotations.try_set(list);
        if self.0.picked.get_untracked().ok() != Some(picked) {
            let _ = self.0.picked.try_set(picked);
        }
        self.update_page_text();
        self.repaint();
        self.repaint_thumbs();
    }

    /// Publishes an edit and records it in the window's undo history: undo puts `before`
    /// back, redo the list as it is now.
    fn commit(&self, label: String, before: Vec<Annotation>) {
        let after = self.0.st.borrow().annots.clone();
        self.publish_annotations();
        let Some(ui) = self.host().map(|el| el.ui().clone()) else { return };
        let (u, r) = (Rc::downgrade(&self.0), Rc::downgrade(&self.0));
        ui.record_change(
            label,
            None,
            move || {
                if let Some(s) = u.upgrade() {
                    PageView(s).restore(before.clone());
                }
            },
            move || {
                if let Some(s) = r.upgrade() {
                    PageView(s).restore(after.clone());
                }
            },
        );
    }

    fn restore(&self, list: Vec<Annotation>) {
        {
            let mut st = self.0.st.borrow_mut();
            if st.picked.is_some_and(|p| !list.iter().any(|a| a.id == p)) {
                st.picked = None;
            }
            st.annots = list;
        }
        self.publish_annotations();
    }

    fn open(&self, id: u64) {
        let handler = self.0.on_open.borrow().clone();
        if let (Some(f), Some(a)) = (handler, self.annotation(id)) {
            f(a);
        }
    }

    /// Scrolls so `bounds` (page points on `page`) shows.
    fn reveal(&self, page: usize, bounds: Rect) {
        {
            let mut st = self.0.st.borrow_mut();
            let a = st.to_content(page, bounds.min);
            let b = st.to_content(page, bounds.max);
            let r = annot::box_of(a, b);
            let (vw, vh) = (st.viewport.width, st.viewport.height);
            let pad = 24.0;
            if r.min.x < st.scroll.x || r.max.x > st.scroll.x + vw {
                st.scroll.x = r.min.x - pad;
            }
            if r.min.y < st.scroll.y || r.max.y > st.scroll.y + vh {
                st.scroll.y = r.min.y - vh / 3.0;
            }
            st.clamp();
        }
        self.changed();
    }

    /// Where a point of the view is on `page`, in that page's points (even off the page).
    fn page_point(&self, page: usize, p: Point) -> Option<Point> {
        let st = self.0.st.borrow();
        let t = st.layout.page_transform(page, st.eff_zoom(), st.viewport.width);
        let inv = t.inverse()?;
        Some(inv.apply(Point::new(p.x + st.scroll.x, p.y + st.scroll.y)))
    }

    /// The page under a point of the view, and the point in its points.
    fn page_under(&self, p: Point) -> Option<(usize, Point)> {
        let st = self.0.st.borrow();
        let c = Point::new(p.x + st.scroll.x, p.y + st.scroll.y);
        st.layout.page_at(c, st.eff_zoom(), st.viewport.width)
    }

    /// The topmost annotation under a point of the view.
    fn annotation_at(&self, p: Point) -> Option<u64> {
        let (page, at) = self.page_under(p)?;
        let st = self.0.st.borrow();
        // A few pixels of slop on screen, whatever the zoom.
        let slop = 4.0 / st.eff_zoom().max(MIN_ZOOM);
        st.annots.iter().rev().find(|a| a.page == page && a.hit(at, slop)).map(|a| a.id)
    }

    /// Starts making or moving an annotation with the current tool. True when the press
    /// was taken.
    fn annotate_down(&self, p: Point, double: bool) -> bool {
        let tool = self.0.st.borrow().tool;
        match tool {
            Tool::Select => {
                let Some(id) = self.annotation_at(p) else {
                    if self.0.st.borrow().picked.is_some() {
                        self.select_annotation(None);
                    }
                    return false;
                };
                if double {
                    self.open(id);
                }
                let at = self.annotation(id).and_then(|a| self.page_point(a.page, p));
                let before = self.0.st.borrow().annots.clone();
                {
                    let mut st = self.0.st.borrow_mut();
                    st.picked = Some(id);
                    st.drawing = at.map(|last| Drawing::Move { id, last, before, moved: false });
                }
                self.publish_annotations();
                true
            }
            Tool::Hand | Tool::Highlight | Tool::Underline | Tool::Strikeout | Tool::Squiggly => false,
            Tool::Ink | Tool::Shape(_) | Tool::FreeText | Tool::Note => {
                let Some((page, at)) = self.page_under(p) else { return false };
                if tool == Tool::Note {
                    let style = self.0.st.borrow().style;
                    let half = annot::NOTE / 2.0;
                    let id = self.add_annotation(Annotation::new(
                        page,
                        AnnotationKind::Note { at: Point::new(at.x - half, at.y - half) },
                        style.color,
                    ));
                    self.select_annotation(Some(id));
                    self.open(id);
                    return true;
                }
                let drawing = match tool {
                    Tool::Ink => Drawing::Ink { page, points: vec![at] },
                    Tool::Shape(shape) => Drawing::Shape { page, shape, from: at, to: at },
                    _ => Drawing::Text { page, from: at, to: at },
                };
                self.0.st.borrow_mut().drawing = Some(drawing);
                self.repaint();
                true
            }
        }
    }

    fn annotate_move(&self, p: Point) {
        let drawing = self.0.st.borrow_mut().drawing.take();
        let next = match drawing {
            Some(Drawing::Ink { page, mut points }) => {
                if let Some(q) = self.page_point(page, p) {
                    points.push(q);
                }
                Some(Drawing::Ink { page, points })
            }
            Some(Drawing::Shape { page, shape, from, to }) => {
                Some(Drawing::Shape { page, shape, from, to: self.page_point(page, p).unwrap_or(to) })
            }
            Some(Drawing::Text { page, from, to }) => {
                Some(Drawing::Text { page, from, to: self.page_point(page, p).unwrap_or(to) })
            }
            Some(Drawing::Move { id, last, before, moved }) => {
                let page = self.annotation(id).map(|a| a.page);
                match page.and_then(|pg| self.page_point(pg, p)) {
                    Some(q) => {
                        {
                            let mut st = self.0.st.borrow_mut();
                            if let Some(a) = st.annots.iter_mut().find(|a| a.id == id) {
                                a.translate(Vec2::new(q.x - last.x, q.y - last.y));
                            }
                        }
                        Some(Drawing::Move { id, last: q, before, moved: true })
                    }
                    None => Some(Drawing::Move { id, last, before, moved }),
                }
            }
            None => None,
        };
        self.0.st.borrow_mut().drawing = next;
        self.repaint();
    }

    fn annotate_up(&self) {
        let drawing = self.0.st.borrow_mut().drawing.take();
        let (style, zoom) = {
            let st = self.0.st.borrow();
            (st.style, st.eff_zoom().max(MIN_ZOOM))
        };
        let made = match drawing {
            Some(Drawing::Ink { page, points }) => {
                // Half a screen pixel of tolerance keeps the stroke as drawn.
                let points = annot::simplify(&points, 0.5 / zoom);
                Some(Annotation::new(
                    page,
                    AnnotationKind::Ink { strokes: vec![points], width: style.width },
                    style.color,
                ))
            }
            Some(Drawing::Shape { page, shape, from, to }) => {
                let tiny = ((to.x - from.x).abs() + (to.y - from.y).abs()) * zoom < 3.0;
                (!tiny).then(|| {
                    Annotation::new(
                        page,
                        AnnotationKind::Shape { shape, from, to, width: style.width },
                        style.color,
                    )
                })
            }
            Some(Drawing::Text { page, from, to }) => {
                // A click without a drag makes a box sized for a few words.
                let r = annot::box_of(from, to);
                let r = if r.width() * zoom < 6.0 || r.height() * zoom < 6.0 {
                    Rect::new(from.x, from.y, from.x + style.text_size * 12.0, from.y + style.text_size * 2.0)
                } else {
                    r
                };
                Some(Annotation::new(
                    page,
                    AnnotationKind::FreeText { rect: r, text: String::new(), size: style.text_size },
                    style.color,
                ))
            }
            Some(Drawing::Move { id, before, moved, .. }) => {
                if moved {
                    let label = self.annotation(id).map_or("Move annotation".to_owned(), |a| {
                        format!("Move {}", a.kind_name().to_lowercase())
                    });
                    self.commit(label, before);
                }
                None
            }
            None => None,
        };
        if let Some(a) = made {
            let text_box = matches!(a.kind, AnnotationKind::FreeText { .. });
            let id = self.add_annotation(a);
            self.select_annotation(Some(id));
            if text_box {
                self.open(id);
            }
        }
        self.repaint();
    }

    // ---------- selection ----------

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let st = self.0.st.borrow();
        let (a, b) = st.text.selection?;
        let s = text::between(&st.text.pages, a, b);
        (!s.is_empty()).then_some(s)
    }

    /// Selects from `anchor` to `focus`.
    pub fn select(&self, anchor: TextPos, focus: TextPos) {
        self.0.st.borrow_mut().text.selection = Some((anchor, focus));
        self.repaint();
    }

    /// Selects all the text of the current page (Ctrl+A).
    pub fn select_page(&self) {
        let page = self.0.page.get_untracked().unwrap_or(0);
        let range = {
            let st = self.0.st.borrow();
            match st.text.pages.get(&page) {
                Some(Some(t)) if !t.runs.is_empty() => {
                    let last = t.runs.len() - 1;
                    let len = t.runs.get(last).map_or(0, |r| r.len());
                    Some((TextPos { page, run: 0, offset: 0 }, TextPos { page, run: last, offset: len }))
                }
                _ => None,
            }
        };
        if let Some((a, b)) = range {
            self.select(a, b);
        } else {
            self.request_text(page);
        }
    }

    /// Clears the selection.
    pub fn clear_selection(&self) {
        self.0.st.borrow_mut().text.selection = None;
        self.repaint();
    }

    /// Copies the selection to the clipboard. Returns false with nothing selected.
    pub fn copy(&self) -> bool {
        let Some(text) = self.selected_text() else { return false };
        match self.host() {
            Some(el) => {
                el.ui().copy_text(text);
                true
            }
            None => false,
        }
    }

    // ---------- search ----------

    /// Searches every page for `query` (ignoring case), off the UI thread; hits arrive in
    /// [`search_status`](Self::search_status) as pages are searched, and the first one is
    /// shown. An empty query ends the search.
    pub fn find(&self, query: &str) {
        let generation = {
            let mut st = self.0.st.borrow_mut();
            if let Some(c) = st.text.search.cancel.take() {
                c.cancel();
            }
            let s = &mut st.text.search;
            s.generation += 1;
            s.query = text::fold_query(query);
            s.text = query.to_string();
            s.matches.clear();
            s.current = None;
            s.searched = 0;
            s.done = query.is_empty();
            s.generation
        };
        self.publish_search();
        self.repaint();
        if query.is_empty() {
            return;
        }
        let cancel = Cancel::new();
        self.0.st.borrow_mut().text.search.cancel = Some(cancel.clone());
        let start = self.0.page.get_untracked().unwrap_or(0);
        self.search_page(generation, start, cancel);
    }

    /// Searches one page on a worker, from the current page on, wrapping; the next page goes
    /// when it comes back, so a search never floods the workers and stops when cancelled.
    fn search_page(&self, generation: u64, page: usize, cancel: Cancel) {
        let source = self.0.source.clone();
        let p = Arc::new(std::sync::atomic::AtomicU64::new(raster::priority(class::SEARCH, 0.0)));
        raster::run(self.0.viewer, cancel, p, move |_| Done::Found(generation, page, source.text(page)));
    }

    fn found(&self, generation: u64, page: usize, t: Option<PageText>) {
        let next = {
            let mut st = self.0.st.borrow_mut();
            if st.text.search.generation != generation {
                return;
            }
            let t = t.map(Arc::new);
            if let Some(t) = &t {
                let hits = text::find_on_page(page, t, &st.text.search.query);
                let s = &mut st.text.search;
                s.matches.extend(hits);
                s.matches.sort();
            }
            st.text.pages.entry(page).or_insert(t);
            st.text.pending.remove(&page);
            let n = st.layout.len();
            let s = &mut st.text.search;
            s.searched += 1;
            s.done = s.searched >= n;
            let first = s.current.is_none() && !s.matches.is_empty();
            if first {
                // The first hit at or after where the search started.
                let from = self.0.page.get_untracked().unwrap_or(0);
                s.current = Some(s.matches.iter().position(|m| m.page() >= from).unwrap_or(0));
            }
            let cancel = s.cancel.clone();
            (first, s.done, cancel)
        };
        let (first, done, cancel) = next;
        if !done && let Some(c) = cancel {
            let n = self.page_count().max(1);
            self.search_page(generation, (page + 1) % n, c);
        }
        self.publish_search();
        if first {
            self.show_match();
        }
        self.repaint();
    }

    fn publish_search(&self) {
        let status = {
            let st = self.0.st.borrow();
            let s = &st.text.search;
            SearchStatus {
                query: s.text.clone(),
                matches: s.matches.len(),
                current: s.current,
                searched: s.searched,
                done: s.done,
            }
        };
        let _ = self.0.search.try_set(status);
    }

    /// Shows the next hit (wrapping).
    pub fn find_next(&self) {
        self.step_match(1);
    }

    /// Shows the previous hit (wrapping).
    pub fn find_previous(&self) {
        self.step_match(-1);
    }

    fn step_match(&self, by: isize) {
        {
            let mut st = self.0.st.borrow_mut();
            let n = st.text.search.matches.len();
            if n == 0 {
                return;
            }
            let cur = st.text.search.current.unwrap_or(0) as isize;
            st.text.search.current = Some((cur + by).rem_euclid(n as isize) as usize);
        }
        self.publish_search();
        self.show_match();
    }

    /// The hits found so far, in document order.
    pub fn matches(&self) -> Vec<Match> {
        self.0.st.borrow().text.search.matches.clone()
    }

    /// Scrolls the current hit into the middle of the view, selecting it.
    fn show_match(&self) {
        self.thaw();
        let target = {
            let st = self.0.st.borrow();
            let s = &st.text.search;
            let Some(m) = s.current.and_then(|i| s.matches.get(i)).copied() else { return };
            let Some(Some(t)) = st.text.pages.get(&m.page()) else { return };
            let rs = text::rects(m.page(), t, m.start, m.end);
            let Some(r) = rs.into_iter().reduce(Rect::union) else { return };
            (m, st.to_content(m.page(), r.center()))
        };
        let (m, c) = target;
        {
            let mut st = self.0.st.borrow_mut();
            st.text.selection = Some((m.start, m.end));
            st.scroll = Vec2::new(c.x - st.viewport.width / 2.0, c.y - st.viewport.height / 2.0);
            st.clamp();
        }
        self.changed();
    }

    // ---------- input ----------

    fn local(&self, p: Point) -> Point {
        let at = self.host().and_then(|e| e.bounds()).map_or(Point::ZERO, |b| b.min);
        Point::new(p.x - at.x, p.y - at.y)
    }

    pub(crate) fn wheel(&self, e: &WheelEvent) -> bool {
        let apple = self.host().is_some_and(|el| el.ui().platform() == remedy_ui_input::Platform::Apple);
        let zoom_key = if apple { e.modifiers.meta } else { e.modifiers.ctrl };
        if zoom_key {
            let z = self.0.st.borrow().zoom;
            let factor = (-e.delta.y * WHEEL_ZOOM_PER_PIXEL).exp();
            self.set_zoom(z * factor, Some(self.local(e.position)));
            return true;
        }
        let d = if e.modifiers.shift && e.delta.x == 0.0 { Vec2::new(e.delta.y, 0.0) } else { e.delta };
        let before = self.0.st.borrow().scroll;
        self.scroll_by(d);
        // A scroll that moved nothing goes on to what is around the view.
        self.0.st.borrow().scroll != before
    }

    pub(crate) fn pinch(&self, e: &PinchEvent) -> bool {
        if matches!(e.phase, PinchPhase::Change) && e.scale.is_finite() && e.scale > 0.0 {
            let z = self.0.st.borrow().zoom;
            self.set_zoom(z * e.scale, Some(self.local(e.position)));
        }
        true
    }

    pub(crate) fn pointer(&self, e: &PointerEvent) -> bool {
        let p = self.local(e.position);
        if e.kind == PointerKind::Touch {
            return self.touch(e, p);
        }
        match e.phase {
            PointerPhase::Down => {
                let middle = e.buttons.contains(Buttons::MIDDLE);
                if !middle && !e.buttons.contains(Buttons::PRIMARY) {
                    return false;
                }
                let bar = self.0.st.borrow().bar_at(p);
                let tool = self.0.st.borrow().tool;
                let gesture = if let Some((vertical, grab)) = bar.filter(|_| !middle) {
                    Gesture::Bar { vertical, grab }
                } else if middle || tool == Tool::Hand {
                    Gesture::Pan { last: p }
                } else if self.annotate_down(p, self.host().is_some_and(|el| el.ui().click_count() >= 2)) {
                    Gesture::Annotate
                } else if let Some(pos) = self.text_at(p) {
                    let keep = e.modifiers.shift;
                    let mut st = self.0.st.borrow_mut();
                    let anchor = match (keep, st.text.selection) {
                        (true, Some((a, _))) => a,
                        _ => pos,
                    };
                    st.text.selection = Some((anchor, pos));
                    Gesture::Select
                } else {
                    // No text under the press: the hand tool.
                    self.0.st.borrow_mut().text.selection = None;
                    Gesture::Pan { last: p }
                };
                self.0.st.borrow_mut().gesture = gesture;
                self.repaint();
                true
            }
            PointerPhase::Move => {
                let g = self.0.st.borrow().gesture;
                match g {
                    Gesture::Pan { last } => {
                        self.0.st.borrow_mut().gesture = Gesture::Pan { last: p };
                        self.scroll_by(Vec2::new(last.x - p.x, last.y - p.y));
                        true
                    }
                    Gesture::Select => {
                        if let Some(pos) = self.text_at(p) {
                            let mut st = self.0.st.borrow_mut();
                            if let Some((a, _)) = st.text.selection {
                                st.text.selection = Some((a, pos));
                            }
                        }
                        self.repaint();
                        true
                    }
                    Gesture::Bar { vertical, grab } => {
                        self.0.st.borrow_mut().drag_bar(vertical, grab, p);
                        self.changed();
                        true
                    }
                    Gesture::Annotate => {
                        self.annotate_move(p);
                        true
                    }
                    Gesture::None | Gesture::Pinch { .. } => false,
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let was = std::mem::replace(&mut self.0.st.borrow_mut().gesture, Gesture::None);
                match was {
                    Gesture::Annotate if e.phase == PointerPhase::Cancel => {
                        // A cancelled drag makes nothing; a cancelled move goes back.
                        let d = self.0.st.borrow_mut().drawing.take();
                        if let Some(Drawing::Move { before, .. }) = d {
                            self.restore(before);
                        }
                        self.repaint();
                    }
                    Gesture::Annotate => self.annotate_up(),
                    Gesture::Select => {
                        // A text tool marks what was just selected.
                        let tool = self.0.st.borrow().tool;
                        self.mark_selection(tool);
                    }
                    _ => {}
                }
                !matches!(was, Gesture::None)
            }
        }
    }

    fn touch(&self, e: &PointerEvent, p: Point) -> bool {
        let mut st = self.0.st.borrow_mut();
        match e.phase {
            PointerPhase::Down | PointerPhase::Move => {
                st.touches.insert(e.id, p);
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                st.touches.remove(&e.id);
            }
        }
        let pts: Vec<Point> = st.touches.values().copied().collect();
        match (pts.as_slice(), st.gesture) {
            ([a, b], Gesture::Pinch { distance, zoom, mid }) => {
                let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                let m = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                st.gesture = Gesture::Pinch { distance, zoom, mid: m };
                drop(st);
                // Two fingers pan as they pinch.
                self.scroll_by(Vec2::new(mid.x - m.x, mid.y - m.y));
                if distance > 0.0 {
                    self.set_zoom(zoom * d / distance, Some(m));
                }
            }
            ([a, b], _) => {
                let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                let m = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                st.gesture = Gesture::Pinch { distance: d, zoom: st.zoom, mid: m };
            }
            ([one], Gesture::Pan { last }) => {
                let one = *one;
                st.gesture = Gesture::Pan { last: one };
                drop(st);
                self.scroll_by(Vec2::new(last.x - one.x, last.y - one.y));
            }
            ([one], _) => st.gesture = Gesture::Pan { last: *one },
            _ => st.gesture = Gesture::None,
        }
        true
    }

    /// The text place under a point of the view, when that page's text is known.
    fn text_at(&self, p: Point) -> Option<TextPos> {
        let (page, at) = {
            let st = self.0.st.borrow();
            let c = Point::new(p.x + st.scroll.x, p.y + st.scroll.y);
            st.layout.page_at(c, st.eff_zoom(), st.viewport.width)?
        };
        let st = self.0.st.borrow();
        match st.text.pages.get(&page) {
            Some(Some(t)) if !t.runs.is_empty() => text::hit(page, t, at),
            _ => None,
        }
    }

    pub(crate) fn key(&self, e: &KeyEvent) -> bool {
        if !e.pressed {
            return false;
        }
        let apple = self.host().is_some_and(|el| el.ui().platform() == remedy_ui_input::Platform::Apple);
        let cmd = if apple { e.modifiers.meta } else { e.modifiers.ctrl };
        let ch = match &e.key {
            Key::Character(c) => Some(c.as_str()),
            _ => None,
        };
        let (vw, vh) = {
            let st = self.0.st.borrow();
            (st.viewport.width, st.viewport.height)
        };
        let picked = self.0.st.borrow().picked;
        if !cmd && !e.modifiers.alt {
            match (&e.key, ch) {
                (_, Some("]")) => return self.select_next_annotation(true),
                (_, Some("[")) => return self.select_next_annotation(false),
                _ => {}
            }
        }
        if let Some(id) = picked {
            let step = if e.modifiers.shift { 10.0 } else { 1.0 };
            let nudge = match &e.key {
                Key::ArrowLeft => Some(Vec2::new(-step, 0.0)),
                Key::ArrowRight => Some(Vec2::new(step, 0.0)),
                Key::ArrowUp => Some(Vec2::new(0.0, -step)),
                Key::ArrowDown => Some(Vec2::new(0.0, step)),
                _ => None,
            };
            if let Some(d) = nudge {
                return self.update_annotation(id, |a| a.translate(d));
            }
            match &e.key {
                Key::Delete | Key::Backspace => return self.remove_annotation(id),
                Key::Escape => {
                    self.select_annotation(None);
                    return true;
                }
                Key::Enter => {
                    self.open(id);
                    return true;
                }
                _ => {}
            }
        }
        match (&e.key, ch) {
            (_, Some("+" | "=")) if cmd => self.zoom_in(),
            (_, Some("-" | "_")) if cmd => self.zoom_out(),
            (_, Some("0")) if cmd => self.set_zoom(1.0, self.caret_anchor()),
            (_, Some("a")) if cmd => self.select_page(),
            (_, Some("c")) if cmd => return self.copy(),
            (Key::Escape, _) => {
                if self.0.st.borrow().text.selection.is_none() {
                    return false;
                }
                self.clear_selection();
            }
            (Key::ArrowDown, _) => self.scroll_by(Vec2::new(0.0, KEY_SCROLL)),
            (Key::ArrowUp, _) => self.scroll_by(Vec2::new(0.0, -KEY_SCROLL)),
            (Key::ArrowRight, _) => self.scroll_by(Vec2::new(KEY_SCROLL, 0.0)),
            (Key::ArrowLeft, _) => self.scroll_by(Vec2::new(-KEY_SCROLL, 0.0)),
            (Key::PageDown, _) => self.scroll_by(Vec2::new(0.0, vh * 0.9)),
            (Key::PageUp, _) => self.scroll_by(Vec2::new(0.0, -vh * 0.9)),
            (Key::Space, _) => {
                let d = if e.modifiers.shift { -vh * 0.9 } else { vh * 0.9 };
                self.scroll_by(Vec2::new(0.0, d));
            }
            (Key::Home, _) => self.go_to_page(0),
            (Key::End, _) => self.go_to_page(self.page_count().saturating_sub(1)),
            _ => {
                let _ = vw;
                return false;
            }
        }
        true
    }

    pub(crate) fn resized(&self, size: Size) {
        self.thaw();
        {
            let mut st = self.0.st.borrow_mut();
            st.viewport = size;
        }
        let fit = self.0.st.borrow().fit;
        if fit != Fit::Free {
            self.set_fit(fit);
        } else {
            self.0.st.borrow_mut().clamp();
            self.changed();
        }
    }

    pub(crate) fn attach(&self, ui: WeakUi, id: ElementId) {
        *self.0.host.borrow_mut() = Some((ui, id));
    }

    pub(crate) fn page_text(&self) -> Signal<String> {
        self.0.page_text
    }

    // ---------- painting ----------

    pub(crate) fn paint(&self, cx: &mut PaintCx<'_>) {
        let mut st = self.0.st.borrow_mut();
        st.viewport = cx.size;
        st.dpr = if cx.scale > 0.0 { cx.scale } else { 1.0 };
        st.clamp();
        let colors = &cx.theme.colors;
        let tokens = cx.theme.derive();
        cx.list.fill_rect(Rect::from_origin_size(Point::ZERO, cx.size), Brush::Solid(colors.panel));
        let (z, scroll) = st.drawn_state();
        let dpr = st.dpr;
        let vw = st.viewport.width;
        let (w, h) = (cx.size.width, cx.size.height);
        let view = Rect::from_origin_size(Point::new(scroll.x, scroll.y), cx.size);
        // Pages around the view are drawn too (paper and preview), so a scroll or zoom the
        // compositor moves finds them there.
        let around = Rect::new(view.min.x - w, view.min.y - h, view.max.x + w, view.max.y + h);
        st.drawn = Rect::new(-w, -h, 2.0 * w, 2.0 * h);
        let mid = view.center();
        let pages = st.layout.pages_between(around.min.y, around.max.y, z);
        let mut visible = Vec::new();
        let mut shown = Vec::new();
        for page in pages {
            let r = st.layout.page_rect(page, z, vw);
            let near = r.intersect(view).is_none_or(|r| r.is_empty());
            let Some(seen) = r.intersect(if near { around } else { view }).filter(|r| !r.is_empty()) else {
                continue;
            };
            let t = st.page_to_view(page);
            let Some(inv) = t.inverse() else { continue };
            let size = self.0.sizes.get(page).copied().unwrap_or(Size::ZERO);
            let whole = Rect::new(0.0, 0.0, size.width, size.height);
            let local = seen.translate(Vec2::new(-scroll.x, -scroll.y));
            let Some(region) = inv.apply_rect(local).intersect(whole).filter(|r| !r.is_empty()) else {
                continue;
            };
            let c = seen.center();
            let distance = ((c.x - mid.x).powi(2) + (c.y - mid.y).powi(2)).sqrt();
            visible.push(Visible { page, region, distance, near });
            shown.push((page, t, r.translate(Vec2::new(-scroll.x, -scroll.y))));
        }
        // Pages: a shadow and paper under the tiles.
        for (_, _, r) in &shown {
            cx.list.push(Item::Shadow {
                rect: *r,
                radii: CornerRadii::ZERO,
                offset: Vec2::new(0.0, 1.0),
                blur: 3.0,
                spread: 0.0,
                color: Color::rgba8(0, 0, 0, 60),
            });
            cx.list.fill_rect(*r, Brush::Solid(Color::WHITE));
        }
        let level = level_of(z * dpr);
        let settled = st.frozen.is_none() && st.moved_at.is_none_or(|t| t.elapsed() >= SETTLE);
        let budget = st.cache_budget();
        let tile = st.tile_edge();
        let sizes = self.0.sizes.clone();
        let size_of = move |p: usize| sizes.get(p).copied().unwrap_or(Size::ZERO);
        let fcx = FrameCx {
            viewer: self.0.viewer,
            source: &self.0.source,
            sizes: &size_of,
            level,
            tile,
            settled,
            budget,
        };
        let draws = st.tiles.frame(&fcx, &visible);
        let mut by_page: HashMap<usize, Vec<Draw>> = HashMap::new();
        for d in draws {
            by_page.entry(d.page).or_default().push(d);
        }
        for (page, t, r) in &shown {
            cx.list.push(Item::PushTransform(*t));
            for d in by_page.remove(page).unwrap_or_default() {
                if let Some(c) = d.clip {
                    cx.list.push_clip(c, CornerRadii::ZERO);
                }
                cx.list.push(Item::Image {
                    image: d.image,
                    rect: d.region,
                    radii: CornerRadii::ZERO,
                    sampling: Sampling::Linear,
                });
                if d.clip.is_some() {
                    cx.list.pop_clip();
                }
            }
            // A page the source could not draw says so with the theme's danger color.
            if st.tiles.errors.contains_key(page) {
                let size = self.0.sizes.get(*page).copied().unwrap_or(Size::ZERO);
                let mut c = colors.danger;
                c.a = 0.25;
                cx.list.fill_rect(Rect::new(0.0, 0.0, size.width, size.height), Brush::Solid(c));
            }
            // Annotations, then search hits and the selection, over the page.
            let accent = colors.accent;
            for a in st.annots.iter().filter(|a| a.page == *page) {
                annot::paint(cx.list, a, st.picked == Some(a.id), accent);
                if let AnnotationKind::FreeText { rect, text: words, size } = &a.kind
                    && !words.is_empty()
                {
                    let style = {
                        let mut v_ = remedy_ui_text::TextStyle::default();
                        v_.size = *size;
                        v_.color = a.color;
                        v_
                    };
                    let options = {
                        let mut v_ = remedy_ui_text::ParagraphOptions::default();
                        v_.max_width = Some((rect.width() - 4.0).max(1.0));
                        v_.snap = false;
                        v_
                    };
                    let para =
                        cx.layout_text(&remedy_ui_text::RichText::plain(words.clone()), &style, options);
                    cx.list.push_clip(*rect, CornerRadii::ZERO);
                    cx.draw_paragraph(&para, Point::new(rect.min.x + 2.0, rect.min.y + 2.0), a.color);
                    cx.list.pop_clip();
                }
            }
            // What is being drawn.
            if let Some(d) = &st.drawing {
                let preview = match d {
                    Drawing::Ink { page: p, points } if p == page => Some(Annotation::new(
                        *p,
                        AnnotationKind::Ink { strokes: vec![points.clone()], width: st.style.width },
                        st.style.color,
                    )),
                    Drawing::Shape { page: p, shape, from, to } if p == page => Some(Annotation::new(
                        *p,
                        AnnotationKind::Shape { shape: *shape, from: *from, to: *to, width: st.style.width },
                        st.style.color,
                    )),
                    Drawing::Text { page: p, from, to } if p == page => Some(Annotation::new(
                        *p,
                        AnnotationKind::FreeText {
                            rect: annot::box_of(*from, *to),
                            text: String::new(),
                            size: st.style.text_size,
                        },
                        st.style.color,
                    )),
                    _ => None,
                };
                if let Some(a) = preview {
                    annot::paint(cx.list, &a, false, accent);
                }
            }
            if let Some(Some(text)) = st.text.pages.get(page) {
                let s = &st.text.search;
                for (i, m) in s.matches.iter().enumerate().filter(|(_, m)| m.page() == *page) {
                    let color = if Some(i) == s.current { tokens.selected } else { tokens.mark };
                    for rect in text::rects(*page, text, m.start, m.end) {
                        cx.list.fill_rect(rect, Brush::Solid(color));
                    }
                }
                if let Some((a, b)) = st.text.selection {
                    let mut c = colors.accent;
                    c.a = 0.35;
                    for rect in text::rects(*page, text, a, b) {
                        cx.list.fill_rect(rect, Brush::Solid(c));
                    }
                }
            }
            cx.list.push(Item::PopTransform);
            let _ = r;
        }
        // Text of what shows, for selection and screen readers.
        let missing: Vec<usize> = shown
            .iter()
            .map(|(p, _, _)| *p)
            .filter(|p| !st.text.pages.contains_key(p) && !st.text.pending.contains(p))
            .collect();
        // Scroll bars.
        for (bar, thumb) in st.bars() {
            let mut track = colors.muted;
            track.a = 0.12;
            cx.list.fill_rounded(bar, CornerRadii::all(BAR / 2.0), Brush::Solid(track));
            let mut c = colors.muted;
            c.a = 0.6;
            cx.list.fill_rounded(thumb, CornerRadii::all(BAR / 2.0), Brush::Solid(c));
        }
        drop(st);
        for p in missing {
            self.request_text(p);
        }
    }

    /// Draws page `page`'s thumbnail into a canvas: its preview, rendered once and shared
    /// with the page view.
    pub(crate) fn paint_thumb(&self, cx: &mut PaintCx<'_>, page: usize) {
        let mut st = self.0.st.borrow_mut();
        let size = self.0.sizes.get(page).copied().unwrap_or(Size::ZERO);
        if size.is_empty() {
            return;
        }
        let shown = st.rotation.apply(size);
        let fit = (cx.size.width / shown.width).min(cx.size.height / shown.height);
        let (w, h) = (shown.width * fit, shown.height * fit);
        let at = Point::new((cx.size.width - w) / 2.0, (cx.size.height - h) / 2.0);
        let r = Rect::new(at.x, at.y, at.x + w, at.y + h);
        cx.list.fill_rect(r, Brush::Solid(Color::WHITE));
        let level = level_of(raster::PREVIEW_PIXELS / size.width.max(size.height));
        let id = TileId { page, level, col: 0, row: 0 };
        let tile = st.tile_edge();
        match st.tiles.get(&id) {
            Some(image) => {
                let t = st
                    .rotation
                    .transform(size)
                    .then(Affine::scale(fit, fit))
                    .then(Affine::translate(at.to_vec2()));
                cx.list.push(Item::PushTransform(t));
                cx.list.push(Item::Image {
                    image,
                    rect: Rect::new(0.0, 0.0, size.width, size.height),
                    radii: CornerRadii::ZERO,
                    sampling: Sampling::Linear,
                });
                cx.list.push(Item::PopTransform);
            }
            None if !st.thumb_flight.contains_key(&page) => {
                let cancel = Cancel::new();
                st.thumb_flight.insert(page, cancel.clone());
                let sizes = self.0.sizes.clone();
                let size_of = move |p: usize| sizes.get(p).copied().unwrap_or(Size::ZERO);
                let fcx = FrameCx {
                    viewer: self.0.viewer,
                    source: &self.0.source,
                    sizes: &size_of,
                    level,
                    tile: tile.max(raster::PREVIEW_PIXELS as u32),
                    settled: true,
                    budget: 0,
                };
                st.tiles.request_one(&fcx, id, class::THUMB, cancel);
            }
            None => {}
        }
        cx.list.border(r, CornerRadii::ZERO, 1.0, cx.theme.derive().line);
    }

    pub(crate) fn thumb_shown(&self, page: usize, ui: WeakUi, el: ElementId) {
        self.0.thumbs.borrow_mut().insert(page, (ui, el));
    }

    pub(crate) fn thumb_gone(&self, page: usize) {
        self.0.thumbs.borrow_mut().remove(&page);
        if let Some(c) = self.0.st.borrow_mut().thumb_flight.remove(&page) {
            c.cancel();
        }
    }

    fn repaint_thumbs(&self) {
        let all: Vec<(WeakUi, ElementId)> = self.0.thumbs.borrow().values().cloned().collect();
        for (ui, el) in all {
            if let Some(ui) = ui.upgrade() {
                ui.element(el).repaint();
            }
        }
    }

    /// Why the source could not draw `page`, when it failed (the page shows tinted with the
    /// theme's danger color until a render succeeds).
    pub fn page_error(&self, page: usize) -> Option<String> {
        self.0.st.borrow().tiles.errors.get(&page).cloned()
    }

    /// Lets go of every tile not on screen now, as memory pressure does.
    pub fn release_memory(&self) {
        self.0.st.borrow_mut().tiles.release_unused();
    }
}

impl State {
    /// The zoom drawn: the set zoom moved onto the nearest render level, so tiles land on
    /// whole device pixels (at most 0.14% away).
    fn eff_zoom(&self) -> f32 {
        self.eff(self.zoom)
    }

    fn tile_edge(&self) -> u32 {
        remedy_ui_compositor::tile_pixels(self.dpr)
    }

    fn relayout(&mut self, sizes: &[Size]) {
        self.layout = PageLayout::new(sizes.to_vec(), self.spread, self.rotation, self.gap, self.margin);
        self.clamp();
    }

    fn clamp(&mut self) {
        let c = self.layout.content_size(self.eff_zoom());
        let max_x = (c.width - self.viewport.width).max(0.0);
        let max_y = (c.height - self.viewport.height).max(0.0);
        let x = if self.scroll.x.is_finite() { self.scroll.x } else { 0.0 };
        let y = if self.scroll.y.is_finite() { self.scroll.y } else { 0.0 };
        self.scroll = Vec2::new(x.clamp(0.0, max_x), y.clamp(0.0, max_y));
    }

    /// The zoom (as drawn) and scroll of the picture: the frozen ones during a gesture.
    fn drawn_state(&self) -> (f32, Vec2) {
        match self.frozen {
            Some(f) => (self.eff(f.zoom), f.scroll),
            None => (self.eff_zoom(), self.scroll),
        }
    }

    fn eff(&self, zoom: f32) -> f32 {
        let dpr = if self.dpr > 0.0 { self.dpr } else { 1.0 };
        scale_of(level_of(zoom * dpr)) / dpr
    }

    /// While frozen: the transform from the picture to where it belongs now (exact for the
    /// page in the middle of the view), or `Some(None)` when the picture no longer covers the
    /// view. `None` when not frozen.
    fn frozen_transform(&self) -> Option<Option<Affine>> {
        let f = self.frozen?;
        let vw = self.viewport.width;
        let (zr, zc) = (self.eff(f.zoom), self.eff_zoom());
        let c = Point::new(self.scroll.x + vw / 2.0, self.scroll.y + self.viewport.height / 2.0);
        let page = self.layout.page_at(c, zc, vw).map_or(0, |(p, _)| p);
        let now = self
            .layout
            .page_transform(page, zc, vw)
            .then(Affine::translate(Vec2::new(-self.scroll.x, -self.scroll.y)));
        let then = self
            .layout
            .page_transform(page, zr, vw)
            .then(Affine::translate(Vec2::new(-f.scroll.x, -f.scroll.y)));
        let t = then.inverse().map(|inv| inv.then(now))?;
        let covered = t.apply_rect(self.drawn);
        let view = Rect::from_origin_size(Point::ZERO, self.viewport);
        let covers = covered.min.x <= view.min.x + 0.5
            && covered.min.y <= view.min.y + 0.5
            && covered.max.x >= view.max.x - 0.5
            && covered.max.y >= view.max.y - 0.5;
        Some(covers.then_some(t))
    }

    /// Page points to the view's own logical pixels as drawn, snapped so tiles land on whole
    /// device pixels.
    fn page_to_view(&self, page: usize) -> Affine {
        let (z, scroll) = self.drawn_state();
        let t = self.layout.page_transform(page, z, self.viewport.width);
        let [a, b, c, d, e, f] = t.0;
        let snap = |v: f32| (v * self.dpr).round() / self.dpr;
        Affine([a, b, c, d, snap(e - scroll.x), snap(f - scroll.y)])
    }

    /// A page point in the view's own logical pixels, now (not as drawn).
    fn to_view(&self, page: usize, p: Point) -> Point {
        let t = self.layout.page_transform(page, self.eff_zoom(), self.viewport.width);
        let q = t.apply(p);
        Point::new(q.x - self.scroll.x, q.y - self.scroll.y)
    }

    fn to_content(&self, page: usize, p: Point) -> Point {
        self.layout.page_transform(page, self.eff_zoom(), self.viewport.width).apply(p)
    }

    fn cache_budget(&self) -> usize {
        let t = self.tile_edge() as f32;
        let w = self.viewport.width * self.dpr + 2.0 * t;
        let h = self.viewport.height * self.dpr + 2.0 * t;
        let screen = (w.max(t) * h.max(t) * 4.0) as u64;
        let floor = screen.saturating_mul(3);
        let share =
            remedy_ui_view::device::total_memory().map_or(floor.saturating_mul(4), |m| m / MEMORY_SHARE);
        usize::try_from(floor.max(share)).unwrap_or(usize::MAX)
    }

    /// The scroll bars that show: (track, thumb) for each.
    fn bars(&self) -> Vec<(Rect, Rect)> {
        let c = self.layout.content_size(self.eff_zoom());
        let (vw, vh) = (self.viewport.width, self.viewport.height);
        let mut out = Vec::new();
        if c.height > vh && vh > 0.0 {
            let track = Rect::new(vw - BAR - 2.0, 2.0, vw - 2.0, vh - 2.0);
            let len = (track.height() * vh / c.height).max(BAR_MIN).min(track.height());
            let y = track.min.y + (track.height() - len) * self.scroll.y / (c.height - vh);
            out.push((track, Rect::new(track.min.x, y, track.max.x, y + len)));
        }
        if c.width > vw && vw > 0.0 {
            let track = Rect::new(2.0, vh - BAR - 2.0, vw - BAR - 4.0, vh - 2.0);
            let len = (track.width() * vw / c.width).max(BAR_MIN).min(track.width());
            let x = track.min.x + (track.width() - len) * self.scroll.x / (c.width - vw);
            out.push((track, Rect::new(x, track.min.y, x + len, track.max.y)));
        }
        out
    }

    /// The scroll bar under `p`: vertical or not, and where on the thumb it was grabbed.
    fn bar_at(&self, p: Point) -> Option<(bool, f32)> {
        for (track, thumb) in self.bars() {
            if track.contains(p) {
                let vertical = track.height() > track.width();
                let grab = if thumb.contains(p) {
                    if vertical { p.y - thumb.min.y } else { p.x - thumb.min.x }
                } else if vertical {
                    thumb.height() / 2.0
                } else {
                    thumb.width() / 2.0
                };
                return Some((vertical, grab));
            }
        }
        None
    }

    fn drag_bar(&mut self, vertical: bool, grab: f32, p: Point) {
        let c = self.layout.content_size(self.eff_zoom());
        for (track, thumb) in self.bars() {
            if (track.height() > track.width()) != vertical {
                continue;
            }
            if vertical {
                let room = (track.height() - thumb.height()).max(1.0);
                let t = ((p.y - grab - track.min.y) / room).clamp(0.0, 1.0);
                self.scroll.y = t * (c.height - self.viewport.height);
            } else {
                let room = (track.width() - thumb.width()).max(1.0);
                let t = ((p.x - grab - track.min.x) / room).clamp(0.0, 1.0);
                self.scroll.x = t * (c.width - self.viewport.width);
            }
        }
        self.clamp();
    }
}

/// The semantics of a page view.
pub(crate) fn semantics(view: &PageView, label: &str) -> Semantics {
    let n = view.page_count();
    let page = view.0.page.get() + 1;
    let zoom = (view.0.zoom.get() * 100.0).round();
    Semantics::new(Role::Document).label(label).value(format!("Page {page} of {n}, {zoom}%"))
}

/// The paint closure for a page view.
pub(crate) fn painter(view: &PageView) -> impl Fn(&mut PaintCx<'_>) + 'static {
    let v = view.clone();
    move |cx| v.paint(cx)
}
