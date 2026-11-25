//! The page view: a document's pages laid out virtually, drawn from tiles rendered off the UI
//! thread at the zoom shown, zoomed and scrolled by pointer, touch, wheel and keyboard.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, ColorMatrix, CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_input::{
    Buttons, Chord, Key, KeyEvent, PinchEvent, PinchPhase, PointerEvent, PointerKind, PointerPhase, WheelEvent,
};
use remedy_ui_paint::{Affine, Brush, Filter, Item, Sampling};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{ElementId, PaintCx, WeakUi};

use crate::annot::{self, Annotation, AnnotationKind, AnnotationStyle, Shape, Tool};
use crate::input::{PageCommand, PageHit, PageInput, PageMode, Swipe};
use crate::layout::{Fit, PageLayout, Rotation, Spread};
use crate::raster::{
    self, Done, Draw, FrameCx, PageStats, TileId, Tiles, Visible, class, level_of, scale_of,
};
use crate::source::{Cancel, PageSource, PageText};
use crate::text::{self, Match, SearchStatus, TextPos, TextState};

/// The smallest zoom unless the view is told otherwise ([`PageView::set_zoom_range`]): 1%.
pub const MIN_ZOOM: f32 = 0.01;
/// The largest zoom unless the view is told otherwise ([`PageView::set_zoom_range`]): 6400%.
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
const WHEEL_NOTCH: f32 = 48.0;

/// Arrow keys scroll by one wheel notch.
const KEY_SCROLL: f32 = 48.0;
/// After a wheel turned the page, how long the wheel is ignored: a trackpad's glide would
/// otherwise run through several pages.
const TURN_REST: Duration = Duration::from_millis(250);
/// How long a finger rests for a long press, and how far it may wander for a tap.
const LONG_PRESS: Duration = Duration::from_millis(450);
const TOUCH_SLOP: f32 = 12.0;
const MOUSE_SLOP: f32 = 3.0;
/// The longest gap, and the farthest apart, for two taps to be a double tap.
const DOUBLE_TAP: Duration = Duration::from_millis(300);
const DOUBLE_TAP_REACH: f32 = 24.0;
/// A swipe: at least this far sideways, within this long.
const SWIPE_REACH: f32 = 56.0;
const SWIPE_TIME: Duration = Duration::from_millis(400);

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

/// A press that may turn out to be a tap, a double tap, a long press or a swipe.
#[derive(Clone, Copy, Debug)]
struct Press {
    at: Point,
    since: Instant,
    /// The scroll when it began (a swipe turns the page only from the page's edge).
    scroll: Vec2,
    moved: bool,
    /// It was held long enough for a long press, which has been told.
    held: bool,
    /// The second press of a double click.
    double: bool,
    seq: u64,
}

/// What a touch leaves to do once the state is free again.
enum After {
    Nothing,
    Arm(u64),
    Lifted(Press, Point),
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
    /// The interface around the viewer lays out right to left: the vertical scroll bar sits
    /// at the left edge, where such interfaces put it (the pages themselves never mirror).
    rtl: bool,
    dpr: f32,
    tiles: Tiles,
    text: TextState,
    moved_at: Option<Instant>,
    settle_timer: Option<u64>,
    /// The zooms the view keeps to ([`PageView::set_zoom_range`]).
    zoom_range: (f32, f32),
    gesture: Gesture,
    touches: HashMap<u32, Point>,
    thumb_flight: HashMap<usize, Cancel>,
    annots: Vec<Annotation>,
    next_annot: u64,
    tool: Tool,
    style: AnnotationStyle,
    picked: Option<u64>,
    drawing: Option<Drawing>,
    mode: PageMode,
    /// The row of pages shown in [`PageMode::Paged`].
    row: usize,
    /// Wheel travel gathered against the edge of the row, towards a page turn.
    turn_pull: f32,
    turned_at: Option<Instant>,
    press: Option<Press>,
    press_seq: u64,
    last_tap: Option<(Instant, Point)>,
    /// Counts the times pages changed: text read before the latest change is not taken.
    epoch: u64,
    /// The colour change on the pages' own pixels ([`PageView::set_page_filter`]).
    filter: Option<ColorMatrix>,
}

/// The app's handler for opening an annotation (to read or edit its comment).
type OpenHandler = Rc<dyn Fn(Annotation)>;
type HitHandler = Rc<dyn Fn(PageHit)>;
type SwipeHandler = Rc<dyn Fn(Swipe) -> bool>;

struct Shared {
    viewer: u64,
    source: RefCell<Arc<dyn PageSource>>,
    sizes: RefCell<Rc<Vec<Size>>>,
    count: Signal<usize>,
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
    input: Cell<PageInput>,
    /// The shortcuts the app gave commands, over their defaults.
    keys: RefCell<HashMap<PageCommand, Vec<Chord>>>,
    on_tap: RefCell<Option<HitHandler>>,
    on_double_tap: RefCell<Option<HitHandler>>,
    on_long_press: RefCell<Option<HitHandler>>,
    on_swipe: RefCell<Option<SwipeHandler>>,
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
                source: RefCell::new(source),
                count: Signal::new(sizes.len()),
                sizes: RefCell::new(Rc::new(sizes)),
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
                    rtl: false,
                    dpr: 1.0,
                    tiles: Tiles::new(),
                    text: TextState::default(),
                    moved_at: None,
                    settle_timer: None,
                    zoom_range: (MIN_ZOOM, MAX_ZOOM),
                    gesture: Gesture::None,
                    touches: HashMap::new(),
                    thumb_flight: HashMap::new(),
                    annots: Vec::new(),
                    next_annot: 1,
                    tool: Tool::Select,
                    style: AnnotationStyle::default(),
                    picked: None,
                    drawing: None,
                    mode: PageMode::Continuous,
                    row: 0,
                    turn_pull: 0.0,
                    turned_at: None,
                    press: None,
                    press_seq: 0,
                    last_tap: None,
                    epoch: 0,
                    filter: None,
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
                input: Cell::new(PageInput::default()),
                keys: RefCell::new(HashMap::new()),
                on_tap: RefCell::new(None),
                on_double_tap: RefCell::new(None),
                on_long_press: RefCell::new(None),
                on_swipe: RefCell::new(None),
            }
        });
        PageView(shared)
    }

    /// Number of pages.
    pub fn page_count(&self) -> usize {
        self.0.sizes.borrow().len()
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

    /// Sets the zooms the view keeps to, 1 being 100% (default [`MIN_ZOOM`] to [`MAX_ZOOM`],
    /// 1% to 6400%); a zoom outside it now moves inside it. A bound that is not a positive
    /// number takes its default. Past the default range a page's coordinates, kept as `f32`,
    /// lose the precision to place a caret or a selection on a glyph.
    pub fn set_zoom_range(&self, min: f32, max: f32) {
        let ok = |z: f32| z.is_finite() && z > 0.0;
        let (min, max) = (if ok(min) { min } else { MIN_ZOOM }, if ok(max) { max } else { MAX_ZOOM });
        let range = (min.min(max), min.max(max));
        let zoom = {
            let mut st = self.0.st.borrow_mut();
            st.zoom_range = range;
            st.zoom
        };
        if zoom < range.0 || zoom > range.1 {
            self.set_zoom(zoom, None);
        }
    }

    /// Sets the zoom (clamped to the zoom range) keeping what is at `anchor` (the view's own
    /// logical pixels; `None`: its middle) where it is. Stops fitting.
    pub fn set_zoom(&self, zoom: f32, anchor: Option<Point>) {
        self.zoom_to(zoom, anchor, Fit::Free);
    }

    fn zoom_to(&self, zoom: f32, anchor: Option<Point>, fit: Fit) {
        if !zoom.is_finite() {
            return;
        }
        let (min, max) = self.0.st.borrow().zoom_range;
        let zoom = zoom.clamp(min, max);
        let gesture = fit == Fit::Free;
        if gesture {
            self.freeze();
        }
        self.0.st.borrow_mut().apply_zoom(zoom, anchor, fit);
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
        let (z, max) = {
            let st = self.0.st.borrow();
            (st.zoom, st.zoom_range.1)
        };
        let next = ZOOM_STEPS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(max);
        self.set_zoom(next, self.caret_anchor());
    }

    /// Zooms out one step (Ctrl+Minus).
    pub fn zoom_out(&self) {
        let (z, min) = {
            let st = self.0.st.borrow();
            (st.zoom, st.zoom_range.0)
        };
        let next = ZOOM_STEPS.iter().rev().copied().find(|s| *s < z / 1.001).unwrap_or(min);
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

    /// Fits the zoom to the current page (or its spread) once: `fit` is measured on that
    /// row alone, not on the document's widest page, and the view does not go on fitting,
    /// so a later resize leaves the zoom where this put it. [`Fit::Free`] does nothing.
    pub fn fit_once(&self, fit: Fit) {
        let z = {
            let st = self.0.st.borrow();
            st.layout.fit_row_zoom(fit, st.viewport, self.0.page.get_untracked().unwrap_or(0))
        };
        if let Some(z) = z {
            let top = Point::new(self.0.st.borrow().viewport.width / 2.0, 0.0);
            self.zoom_to(z, Some(top), Fit::Free);
        }
    }

    /// Sets how pages sit side by side, keeping the current page in view.
    pub fn set_spread(&self, spread: Spread) {
        let page = self.0.page.get_untracked().unwrap_or(0);
        {
            let mut st = self.0.st.borrow_mut();
            st.spread = spread;
            st.relayout(&self.0.sizes.borrow());
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
            st.relayout(&self.0.sizes.borrow());
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
            st.relayout(&self.0.sizes.borrow());
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
            st.row = st.layout.row_of(page);
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

    /// How pages follow one another.
    pub fn page_mode(&self) -> PageMode {
        self.0.st.borrow().mode
    }

    /// Shows one long scroll of pages, or one row of pages at a time (see [`PageMode`]). The
    /// current page stays the current page.
    pub fn set_page_mode(&self, mode: PageMode) {
        if self.0.st.borrow().mode == mode {
            return;
        }
        let page = self.0.page.get_untracked().unwrap_or(0);
        self.0.st.borrow_mut().mode = mode;
        self.go_to_page(page);
    }

    /// Goes to the next page: in paged mode the next row of pages (the next spread). `false`
    /// at the end of the document.
    pub fn next_page(&self) -> bool {
        self.turn(true, false)
    }

    /// Goes to the previous page, or row of pages. `false` at the start of the document.
    pub fn previous_page(&self) -> bool {
        self.turn(false, false)
    }

    /// Turns a row on or back; `at_end` lands at the bottom of the row turned to (what
    /// scrolling back up into it expects).
    fn turn(&self, forward: bool, at_end: bool) -> bool {
        self.thaw();
        {
            let mut st = self.0.st.borrow_mut();
            let rows = st.layout.rows();
            let now = if st.mode == PageMode::Paged {
                st.row
            } else {
                st.layout.row_of(self.0.page.get_untracked().unwrap_or(0))
            };
            let to = if forward { now + 1 } else { now.wrapping_sub(1) };
            if to >= rows {
                return false;
            }
            st.row = to;
            let (lo, hi) = st.y_range_of(to);
            st.scroll.y = if at_end { hi } else { lo };
            st.turned_at = Some(Instant::now());
            st.turn_pull = 0.0;
            st.clamp();
        }
        self.changed();
        true
    }

    /// Scrolls, and in paged mode turns the page when the scroll is already at that edge.
    fn scroll_or_turn(&self, d: Vec2) -> bool {
        let before = self.0.st.borrow().scroll;
        self.scroll_by(d);
        let st = self.0.st.borrow();
        if st.scroll != before {
            return true;
        }
        let paged = st.mode == PageMode::Paged;
        drop(st);
        paged && self.turn(d.x + d.y > 0.0, d.y < 0.0)
    }

    /// How the view zooms from the wheel and the keyboard.
    pub fn input(&self) -> PageInput {
        self.0.input.get()
    }

    /// Changes how the view zooms from the wheel and the keyboard.
    pub fn set_input(&self, input: PageInput) {
        self.0.input.set(input);
    }

    /// The shortcuts of `command` now: what [`PageView::rebind`] gave it, or its defaults.
    pub fn chords(&self, command: PageCommand) -> Vec<Chord> {
        let platform = self.host().map(|el| el.ui().platform()).unwrap_or_default();
        self.0.keys.borrow().get(&command).cloned().unwrap_or_else(|| command.default_chords(platform))
    }

    /// Gives `command` these shortcuts in place of its defaults (none: no shortcut, for an
    /// app that answers the keys itself and calls [`PageView::run`]).
    pub fn rebind(&self, command: PageCommand, chords: Vec<Chord>) {
        self.0.keys.borrow_mut().insert(command, chords);
    }

    /// Gives `command` its default shortcuts back.
    pub fn reset_binding(&self, command: PageCommand) {
        self.0.keys.borrow_mut().remove(&command);
    }

    /// Does what `command` does. `false` when there was nothing to do (no selection to copy
    /// or clear, no page to turn to).
    pub fn run(&self, command: PageCommand) -> bool {
        let vh = self.0.st.borrow().viewport.height;
        let input = self.0.input.get();
        let zoom = self.0.st.borrow().zoom;
        match command {
            PageCommand::ZoomIn => match input.key_zoom {
                Some(step) => self.set_zoom(step.apply(zoom, 1.0), self.caret_anchor()),
                None => self.zoom_in(),
            },
            PageCommand::ZoomOut => match input.key_zoom {
                Some(step) => self.set_zoom(step.apply(zoom, -1.0), self.caret_anchor()),
                None => self.zoom_out(),
            },
            PageCommand::ResetZoom => self.set_zoom(input.reset_zoom, self.caret_anchor()),
            PageCommand::NextPage => return self.next_page(),
            PageCommand::PreviousPage => return self.previous_page(),
            PageCommand::FirstPage => self.go_to_page(0),
            PageCommand::LastPage => self.go_to_page(self.page_count().saturating_sub(1)),
            PageCommand::ScrollUp => return self.scroll_or_turn(Vec2::new(0.0, -KEY_SCROLL)),
            PageCommand::ScrollDown => return self.scroll_or_turn(Vec2::new(0.0, KEY_SCROLL)),
            PageCommand::ScrollLeft => return self.scroll_or_turn(Vec2::new(-KEY_SCROLL, 0.0)),
            PageCommand::ScrollRight => return self.scroll_or_turn(Vec2::new(KEY_SCROLL, 0.0)),
            PageCommand::PageUp => return self.scroll_or_turn(Vec2::new(0.0, -vh * 0.9)),
            PageCommand::PageDown => return self.scroll_or_turn(Vec2::new(0.0, vh * 0.9)),
            PageCommand::SelectPage => self.select_page(),
            PageCommand::Copy => return self.copy(),
            PageCommand::ClearSelection => {
                if self.0.st.borrow().text.selection.is_none() {
                    return false;
                }
                self.clear_selection();
            }
        }
        true
    }

    /// The command a key press asks for.
    fn command_for(&self, e: &KeyEvent) -> Option<PageCommand> {
        // The key as typed, then the key in its place on a US keyboard (Ctrl+C on a Russian
        // layout), as the command table does.
        let typed = Chord::new(e.modifiers, e.key.clone());
        let placed = Chord::new(e.modifiers, e.shortcut_key());
        PageCommand::ALL.into_iter().find(|c| {
            let chords = self.chords(*c);
            chords.contains(&typed) || chords.contains(&placed)
        })
    }

    /// Hears a tap or a click that did not drag (and was not the second of a double tap).
    pub fn on_tap(&self, f: impl Fn(PageHit) + 'static) {
        *self.0.on_tap.borrow_mut() = Some(Rc::new(f));
    }

    /// Hears a double tap or a double click, unless it opened an annotation.
    pub fn on_double_tap(&self, f: impl Fn(PageHit) + 'static) {
        *self.0.on_double_tap.borrow_mut() = Some(Rc::new(f));
    }

    /// Hears a finger held still on the view for 450 ms (within 12 pixels).
    pub fn on_long_press(&self, f: impl Fn(PageHit) + 'static) {
        *self.0.on_long_press.borrow_mut() = Some(Rc::new(f));
    }

    /// Hears a quick sideways flick of one finger. Answer `true` when it was used; otherwise
    /// in paged mode it turns the page, when the page is at that side's edge.
    pub fn on_swipe(&self, f: impl Fn(Swipe) -> bool + 'static) {
        *self.0.on_swipe.borrow_mut() = Some(Rc::new(f));
    }

    fn tell(&self, handler: &RefCell<Option<HitHandler>>, at: Point) {
        let f = handler.borrow().clone();
        if let Some(f) = f {
            f(PageHit { at, page: self.page_under(at) });
        }
    }

    /// A press ended at `p` without being cancelled: a tap, a double tap or a swipe.
    fn lifted(&self, press: Press, p: Point) {
        if press.held {
            return;
        }
        if !press.moved {
            let again = self.0.st.borrow().last_tap.is_some_and(|(t, at)| {
                t.elapsed() <= DOUBLE_TAP && (at.x - p.x).hypot(at.y - p.y) <= DOUBLE_TAP_REACH
            });
            if press.double || again {
                self.0.st.borrow_mut().last_tap = None;
                self.tell(&self.0.on_double_tap, p);
            } else {
                self.0.st.borrow_mut().last_tap = Some((Instant::now(), p));
                self.tell(&self.0.on_tap, p);
            }
            return;
        }
        let (dx, dy) = (p.x - press.at.x, p.y - press.at.y);
        if press.since.elapsed() > SWIPE_TIME || dx.abs() < SWIPE_REACH || dx.abs() < 2.0 * dy.abs() {
            return;
        }
        let swipe = if dx < 0.0 { Swipe::Left } else { Swipe::Right };
        let f = self.0.on_swipe.borrow().clone();
        if f.is_some_and(|f| f(swipe)) {
            return;
        }
        let at_edge = {
            let st = self.0.st.borrow();
            let max_x = (st.layout.content_size(st.eff_zoom()).width - st.viewport.width).max(0.0);
            st.mode == PageMode::Paged
                && match swipe {
                    Swipe::Left => press.scroll.x >= max_x - 0.5,
                    Swipe::Right => press.scroll.x <= 0.5,
                }
        };
        if at_edge {
            self.turn(swipe == Swipe::Left, false);
        }
    }

    fn arm_long_press(&self, seq: u64) {
        let Some(el) = self.host() else { return };
        if self.0.on_long_press.borrow().is_none() {
            return;
        }
        let me = Rc::downgrade(&self.0);
        // Never cancelled: a press that ended or moved no longer has this number.
        let _ = el.ui().after(LONG_PRESS, move || {
            let Some(s) = me.upgrade() else { return };
            let v = PageView(s);
            let at = match v.0.st.borrow_mut().press.as_mut() {
                Some(pr) if pr.seq == seq && !pr.moved => {
                    pr.held = true;
                    Some(pr.at)
                }
                _ => None,
            };
            if let Some(at) = at {
                v.tell(&v.0.on_long_press, at);
            }
        });
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
            let page = if st.mode == PageMode::Paged {
                st.layout.row_pages(st.row).start
            } else {
                st.layout.page_at(c, st.eff_zoom(), st.viewport.width).map_or(0, |(p, _)| p)
            };
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

    /// The window's language (not tracked), for what the view says; English before it is
    /// shown.
    fn locale(&self) -> String {
        let h = self.0.host.borrow();
        let ui = h.as_ref().and_then(|(ui, _)| ui.upgrade());
        ui.map_or_else(|| "en".to_owned(), |ui| remedy_ui_reactive::untrack(|| ui.locale()))
    }

    pub(crate) fn update_page_text(&self) {
        let locale = self.locale();
        let page = self.0.page.get_untracked().unwrap_or(0);
        let known = self.0.st.borrow().text.pages.get(&page).cloned();
        let notes = {
            let st = self.0.st.borrow();
            let mut lines: Vec<String> = st
                .annots
                .iter()
                .filter(|a| a.page == page)
                .map(|a| {
                    let said = a.describe_in(&locale);
                    if st.picked == Some(a.id) {
                        crate::say(&locale, "annotation-selected", &[("description", said.into())])
                    } else {
                        said
                    }
                })
                .collect();
            if !lines.is_empty() {
                lines.insert(0, crate::say(&locale, "annotation-count", &[("count", lines.len().into())]));
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
        let epoch = st.epoch;
        drop(st);
        let source = self.0.source.borrow().clone();
        let p = Arc::new(std::sync::atomic::AtomicU64::new(raster::priority(class::TEXT, page as f32)));
        raster::run(self.0.viewer, Cancel::new(), p, move |_| Done::Text(page, epoch, source.text(page)));
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
            Done::Tile(id, epoch, result) => {
                {
                    let mut st = self.0.st.borrow_mut();
                    let size = self.0.sizes.borrow().get(id.page).copied().unwrap_or(Size::ZERO);
                    let tile = st.tile_edge();
                    st.tiles.adopt(id, epoch, result, tile, size);
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
            Done::Text(page, epoch, t) => {
                {
                    let mut st = self.0.st.borrow_mut();
                    if epoch != st.epoch {
                        // Read before the page changed; it has been asked for again since.
                        return;
                    }
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
        let label = self.undo_name("undo-add", self.annotation(id).as_ref());
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
            a.kind_id()
        };
        let label = crate::say(&self.locale(), "undo-change", &[("kind", label.into())]);
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
            a.kind_id()
        };
        let label = crate::say(&self.locale(), "undo-delete", &[("kind", label.into())]);
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
            let mut name = "annotation";
            for (page, quads, words) in marks {
                let Some(kind) = make(quads) else { continue };
                let mut a = Annotation::new(page, kind, color);
                a.marked_text = words;
                a.id = st.next_annot;
                st.next_annot += 1;
                name = a.kind_id();
                st.annots.push(a);
            }
            st.text.selection = None;
            name
        };
        let label = crate::say(&self.locale(), "undo-add", &[("kind", label.into())]);
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

    /// The undo history's name for an edit of `a` (message `id`, by its kind).
    fn undo_name(&self, id: &str, a: Option<&Annotation>) -> String {
        let kind = a.map_or("annotation", Annotation::kind_id);
        crate::say(&self.locale(), id, &[("kind", kind.into())])
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
            st.row = st.layout.row_of(page);
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
        st.hit(c)
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
                    let label = self.undo_name("undo-move", self.annotation(id).as_ref());
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
        let source = self.0.source.borrow().clone();
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
            st.row = st.layout.row_of(m.page());
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
            let input = self.0.input.get();
            let step = if e.modifiers.shift { input.wheel_zoom_shift.or(input.wheel_zoom) } else { input.wheel_zoom };
            let Some(step) = step else { return false };
            // Some platforms turn a wheel with Shift held sideways.
            let dy = if e.delta.y != 0.0 { e.delta.y } else { e.delta.x };
            let z = self.0.st.borrow().zoom;
            self.set_zoom(step.apply(z, -dy / WHEEL_NOTCH), Some(self.local(e.position)));
            return true;
        }
        let d = if e.modifiers.shift && e.delta.x == 0.0 { Vec2::new(e.delta.y, 0.0) } else { e.delta };
        let before = self.0.st.borrow().scroll;
        self.scroll_by(d);
        let mut st = self.0.st.borrow_mut();
        if st.scroll != before {
            st.turn_pull = 0.0;
            return true;
        }
        if st.mode != PageMode::Paged {
            // A scroll that moved nothing goes on to what is around the view.
            return false;
        }
        if st.turned_at.is_some_and(|t| t.elapsed() < TURN_REST) {
            return true;
        }
        // Against the edge of the page: a notch's worth of wheel turns it.
        let pull = d.x + d.y;
        st.turn_pull = if pull * st.turn_pull < 0.0 { pull } else { st.turn_pull + pull };
        if st.turn_pull.abs() < WHEEL_NOTCH {
            return true;
        }
        drop(st);
        self.turn(pull > 0.0, d.y < 0.0)
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
                let double = self.host().is_some_and(|el| el.ui().click_count() >= 2);
                {
                    let mut st = self.0.st.borrow_mut();
                    st.gesture = gesture;
                    // A press on the pages themselves may be a click for the app.
                    let scroll = st.scroll;
                    st.press = matches!(gesture, Gesture::Pan { .. } | Gesture::Select).then_some(Press {
                        at: p,
                        since: Instant::now(),
                        scroll,
                        moved: false,
                        held: false,
                        double,
                        seq: 0,
                    });
                }
                self.repaint();
                true
            }
            PointerPhase::Move => {
                if let Some(pr) = self.0.st.borrow_mut().press.as_mut()
                    && (pr.at.x - p.x).hypot(pr.at.y - p.y) > MOUSE_SLOP
                {
                    pr.moved = true;
                }
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
                let press = self.0.st.borrow_mut().press.take();
                if let Some(press) = press.filter(|pr| e.phase == PointerPhase::Up && !pr.moved) {
                    self.lifted(press, p);
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
        let mut after = After::Nothing;
        match (e.phase, st.touches.len()) {
            (PointerPhase::Down, 1) => {
                st.press_seq += 1;
                let seq = st.press_seq;
                st.press = Some(Press {
                    at: p,
                    since: Instant::now(),
                    scroll: st.scroll,
                    moved: false,
                    held: false,
                    double: false,
                    seq,
                });
                after = After::Arm(seq);
            }
            (PointerPhase::Move, 1) => {
                if let Some(pr) = st.press.as_mut()
                    && (pr.at.x - p.x).hypot(pr.at.y - p.y) > TOUCH_SLOP
                {
                    pr.moved = true;
                }
            }
            (PointerPhase::Up, 0) => {
                if let Some(pr) = st.press.take() {
                    after = After::Lifted(pr, p);
                }
            }
            // A second finger, or a cancelled touch: not a tap.
            _ => st.press = None,
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
                drop(st);
            }
            ([one], Gesture::Pan { last }) => {
                let one = *one;
                st.gesture = Gesture::Pan { last: one };
                drop(st);
                self.scroll_by(Vec2::new(last.x - one.x, last.y - one.y));
            }
            ([one], _) => {
                st.gesture = Gesture::Pan { last: *one };
                drop(st);
            }
            _ => {
                st.gesture = Gesture::None;
                drop(st);
            }
        }
        match after {
            After::Nothing => {}
            After::Arm(seq) => self.arm_long_press(seq),
            After::Lifted(press, p) => self.lifted(press, p),
        }
        true
    }

    /// The text place under a point of the view, when that page's text is known.
    fn text_at(&self, p: Point) -> Option<TextPos> {
        let (page, at) = {
            let st = self.0.st.borrow();
            let c = Point::new(p.x + st.scroll.x, p.y + st.scroll.y);
            st.hit(c)?
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
        self.command_for(e).is_some_and(|c| self.run(c))
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
        let resized = st.viewport != cx.size;
        st.viewport = cx.size;
        st.rtl = cx.rtl;
        st.dpr = if cx.scale > 0.0 { cx.scale } else { 1.0 };
        // A view that learns its size in this frame fits before it chooses tiles: tiles asked
        // for at the zoom it is about to leave would be wasted work (the resize callback, which
        // publishes the fitted zoom, runs only after the frame).
        let fit = st.fit;
        if resized && fit != Fit::Free {
            let page = self.0.page.get_untracked().unwrap_or(0);
            if let Some(z) = st.layout.fit_zoom(fit, st.viewport, page).filter(|z| z.is_finite()) {
                let top = Point::new(st.viewport.width / 2.0, 0.0);
                let (min, max) = st.zoom_range;
                st.apply_zoom(z.clamp(min, max), Some(top), fit);
            }
        }
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
        let pages = if st.mode == PageMode::Paged {
            st.layout.row_pages(st.row)
        } else {
            st.layout.pages_between(around.min.y, around.max.y, z)
        };
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
            let size = self.0.sizes.borrow().get(page).copied().unwrap_or(Size::ZERO);
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
            cx.list.fill_rect(*r, Brush::Solid(st.paper()));
        }
        let level = level_of(z * dpr);
        let settled = st.frozen.is_none() && st.moved_at.is_none_or(|t| t.elapsed() >= SETTLE);
        let budget = st.cache_budget();
        let tile = st.tile_edge();
        let sizes = self.0.sizes.borrow().clone();
        let source = self.0.source.borrow().clone();
        let size_of = move |p: usize| sizes.get(p).copied().unwrap_or(Size::ZERO);
        let fcx = FrameCx {
            viewer: self.0.viewer,
            source: &source,
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
            let tiles = by_page.remove(page).unwrap_or_default();
            // The filter is on the page's pixels alone: what is drawn over them comes after.
            let filtered = st.filter.filter(|_| !tiles.is_empty());
            if let Some(matrix) = filtered {
                cx.list.push(Item::PushFilter(Filter::ColorMatrix { matrix }));
            }
            for d in tiles {
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
            if filtered.is_some() {
                cx.list.push(Item::PopFilter);
            }
            // A page the source could not draw says so with the theme's danger color.
            if st.tiles.errors.contains_key(page) {
                let size = self.0.sizes.borrow().get(*page).copied().unwrap_or(Size::ZERO);
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
        let size = self.0.sizes.borrow().get(page).copied().unwrap_or(Size::ZERO);
        if size.is_empty() {
            return;
        }
        let shown = st.rotation.apply(size);
        let fit = (cx.size.width / shown.width).min(cx.size.height / shown.height);
        let (w, h) = (shown.width * fit, shown.height * fit);
        let at = Point::new((cx.size.width - w) / 2.0, (cx.size.height - h) / 2.0);
        let r = Rect::new(at.x, at.y, at.x + w, at.y + h);
        cx.list.fill_rect(r, Brush::Solid(st.paper()));
        let filter = st.filter;
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
                if let Some(matrix) = filter {
                    cx.list.push(Item::PushFilter(Filter::ColorMatrix { matrix }));
                }
                cx.list.push(Item::Image {
                    image,
                    rect: Rect::new(0.0, 0.0, size.width, size.height),
                    radii: CornerRadii::ZERO,
                    sampling: Sampling::Linear,
                });
                if filter.is_some() {
                    cx.list.push(Item::PopFilter);
                }
                cx.list.push(Item::PopTransform);
            }
            None if !st.thumb_flight.contains_key(&page) => {
                let cancel = Cancel::new();
                st.thumb_flight.insert(page, cancel.clone());
                let sizes = self.0.sizes.borrow().clone();
        let source = self.0.source.borrow().clone();
                let size_of = move |p: usize| sizes.get(p).copied().unwrap_or(Size::ZERO);
                let fcx = FrameCx {
                    viewer: self.0.viewer,
                    source: &source,
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

    /// The colour change on the pages.
    pub fn page_filter(&self) -> Option<ColorMatrix> {
        self.0.st.borrow().filter
    }

    /// Changes the colours of the pages themselves (their paper and what the source draws on
    /// it, here and in the thumbnails) and of nothing else: the panel behind them, their
    /// shadows, annotations, the selection, search hits and the scroll bars keep theirs.
    /// [`ColorMatrix`] has the usual ones (invert, sepia, grayscale, warm, cool, brightness,
    /// contrast, and ink and paper colours of your own), and
    /// [`ColorMatrix::then`] joins them. `None`, or a matrix that changes nothing: the pages
    /// as the source draws them.
    pub fn set_page_filter(&self, filter: Option<ColorMatrix>) {
        self.0.st.borrow_mut().filter = filter.map(|m| m.limited()).filter(|m| !m.is_identity());
        self.thaw();
        self.repaint();
        self.repaint_thumbs();
    }

    /// The number of pages, as a signal: it changes when the document does
    /// ([`PageView::document_changed`], [`PageView::set_source`]).
    pub fn page_count_signal(&self) -> Signal<usize> {
        self.0.count
    }

    /// Tells the view these pages of its source now draw or read differently (an edit, a
    /// filled form field, a rotation saved into the file). They are rendered and read again;
    /// zoom, scroll and the current page stay as they are.
    pub fn pages_changed(&self, pages: &[usize]) {
        self.reload(Some(pages));
    }

    /// Tells the view its source is a different document now, or has pages added, removed or
    /// resized: everything is laid out, rendered and read again. Zoom and scroll stay (as far
    /// as the new pages allow), and so does the current page.
    pub fn document_changed(&self) {
        self.reload(None);
    }

    /// Shows another source in this view, as [`PageView::document_changed`] does.
    pub fn set_source(&self, source: Arc<dyn PageSource>) {
        *self.0.source.borrow_mut() = source;
        self.reload(None);
    }

    fn reload(&self, pages: Option<&[usize]>) {
        self.thaw();
        let source = self.0.source.borrow().clone();
        let sizes: Vec<Size> = (0..source.page_count()).map(|p| source.page_size(p)).collect();
        let resized = **self.0.sizes.borrow() != sizes;
        let query = {
            let mut st = self.0.st.borrow_mut();
            st.tiles.invalidate(pages);
            for (_, c) in st.thumb_flight.drain() {
                c.cancel();
            }
            st.epoch += 1;
            // Whatever was being read is asked for again; what was read of changed pages is
            // forgotten, with a selection on them.
            st.text.pending.clear();
            match pages {
                Some(pages) => {
                    for p in pages {
                        st.text.pages.remove(p);
                    }
                    if st.text.selection.is_some_and(|(a, b)| pages.contains(&a.page) || pages.contains(&b.page)) {
                        st.text.selection = None;
                    }
                }
                None => {
                    st.text.pages.clear();
                    st.text.selection = None;
                }
            }
            if resized {
                *self.0.sizes.borrow_mut() = Rc::new(sizes.clone());
                st.relayout(&sizes);
            }
            st.text.search.text.clone()
        };
        if resized {
            let _ = self.0.count.try_set(sizes.len());
            self.refit();
        }
        // The hits were places in the old text.
        if !query.is_empty() {
            self.find(&query);
        }
        self.update_page_text();
        self.changed();
        self.repaint();
        self.repaint_thumbs();
    }

    /// Lets go of every tile not on screen now, as memory pressure does.
    pub fn release_memory(&self) {
        self.0.st.borrow_mut().tiles.release_unused();
    }
}

impl State {
    /// The colour of a page before its source has drawn on it: white, through the filter.
    fn paper(&self) -> Color {
        let Some(m) = self.filter else { return Color::WHITE };
        let [r, g, b, a] = m.apply([1.0, 1.0, 1.0, 1.0]);
        Color { r, g, b, a }
    }

    /// The zoom drawn: the set zoom moved onto the nearest render level, so tiles land on
    /// whole device pixels (at most 0.14% away).
    fn eff_zoom(&self) -> f32 {
        self.eff(self.zoom)
    }

    fn tile_edge(&self) -> u32 {
        remedy_ui_compositor::tile_pixels(self.dpr)
    }

    /// Sets the zoom keeping what is at `anchor` (the view's own logical pixels; `None`: its
    /// middle) where it is, and how the zoom follows the view.
    fn apply_zoom(&mut self, zoom: f32, anchor: Option<Point>, fit: Fit) {
        let anchor = anchor.unwrap_or(Point::new(self.viewport.width / 2.0, self.viewport.height / 2.0));
        let vw = self.viewport.width;
        let before = self.eff_zoom();
        let at = Point::new(self.scroll.x + anchor.x, self.scroll.y + anchor.y);
        let hit = self.layout.page_at(at, before, vw);
        let old_level = level_of(before * self.dpr);
        self.zoom = zoom;
        self.fit = fit;
        let after = self.eff_zoom();
        if let Some((page, p)) = hit {
            let c = self.layout.page_transform(page, after, vw).apply(p);
            self.scroll = Vec2::new(c.x - anchor.x, c.y - anchor.y);
        }
        if level_of(after * self.dpr) != old_level {
            self.moved_at = Some(Instant::now());
        }
        self.clamp();
    }

    fn relayout(&mut self, sizes: &[Size]) {
        let page = self.layout.row_pages(self.row).start;
        self.layout = PageLayout::new(sizes.to_vec(), self.spread, self.rotation, self.gap, self.margin);
        self.row = self.layout.row_of(page);
        self.clamp();
    }

    /// The page under a point of the content, and the point in its points: in paged mode
    /// only a page of the row shown.
    fn hit(&self, c: Point) -> Option<(usize, Point)> {
        let hit = self.layout.page_at(c, self.eff_zoom(), self.viewport.width)?;
        (self.mode != PageMode::Paged || self.layout.row_of(hit.0) == self.row).then_some(hit)
    }

    /// The vertical scrolls the view may have: all of the content, or in paged mode the row
    /// shown.
    fn y_range(&self) -> (f32, f32) {
        if self.mode == PageMode::Paged && self.layout.rows() > 0 {
            return self.y_range_of(self.row.min(self.layout.rows() - 1));
        }
        (0.0, (self.layout.content_size(self.eff_zoom()).height - self.viewport.height).max(0.0))
    }

    /// The scrolls that show `row`: in paged mode from its top to its bottom (one place,
    /// centring it, when it is shorter than the view); otherwise its top alone.
    fn y_range_of(&self, row: usize) -> (f32, f32) {
        let z = self.eff_zoom();
        let top = self.layout.row_top(row, z) - self.margin;
        if self.mode != PageMode::Paged {
            return (top, top);
        }
        let bottom = self.layout.row_bottom(row, z) + self.margin;
        let vh = self.viewport.height;
        if bottom - top <= vh {
            let y = top - (vh - (bottom - top)) / 2.0;
            (y, y)
        } else {
            (top, bottom - vh)
        }
    }

    fn clamp(&mut self) {
        let c = self.layout.content_size(self.eff_zoom());
        let max_x = (c.width - self.viewport.width).max(0.0);
        if self.layout.rows() > 0 {
            self.row = self.row.min(self.layout.rows() - 1);
        }
        let (lo, hi) = self.y_range();
        let x = if self.scroll.x.is_finite() { self.scroll.x } else { 0.0 };
        let y = if self.scroll.y.is_finite() { self.scroll.y } else { lo };
        self.scroll = Vec2::new(x.clamp(0.0, max_x), y.clamp(lo, hi));
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
        // The vertical bar on the end side: the right, or the left right to left; the
        // horizontal one runs beside it.
        let (bar_x, run) =
            if self.rtl { (2.0, (BAR + 4.0, vw - 2.0)) } else { (vw - BAR - 2.0, (2.0, vw - BAR - 4.0)) };
        let (lo, hi) = self.y_range();
        if hi > lo && vh > 0.0 {
            let track = Rect::new(bar_x, 2.0, bar_x + BAR, vh - 2.0);
            let len = (track.height() * vh / (hi - lo + vh)).max(BAR_MIN).min(track.height());
            let y = track.min.y + (track.height() - len) * (self.scroll.y - lo) / (hi - lo);
            out.push((track, Rect::new(track.min.x, y, track.max.x, y + len)));
        }
        if c.width > vw && vw > 0.0 {
            let track = Rect::new(run.0, vh - BAR - 2.0, run.1, vh - 2.0);
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
                let (lo, hi) = self.y_range();
                self.scroll.y = lo + t * (hi - lo);
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
pub(crate) fn semantics(view: &PageView, label: &str, locale: &str) -> Semantics {
    let n = view.page_count();
    let page = view.0.page.get() + 1;
    let zoom = (view.0.zoom.get() * 100.0).round();
    let value = crate::say(
        locale,
        "page-view-value",
        &[("page", page.into()), ("count", n.into()), ("zoom", zoom.into())],
    );
    Semantics::new(Role::Document).label(label).value(value)
}

/// The paint closure for a page view.
pub(crate) fn painter(view: &PageView) -> impl Fn(&mut PaintCx<'_>) + 'static {
    let v = view.clone();
    move |cx| v.paint(cx)
}
