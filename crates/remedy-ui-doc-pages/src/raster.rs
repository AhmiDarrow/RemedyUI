//! Page tiles: rendered by the app's [`PageSource`] on worker threads at the scale shown,
//! cached within a share of the machine's memory, and drawn scaled from another scale while
//! the right one renders.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use remedy_ui_core::Rect;
use remedy_ui_paint::Image;

use crate::source::{Cancel, PageSource, PageText, RenderError, RenderRequest};

/// Scales are kept in steps of 1/256 of a doubling: fine enough that a tile drawn at a
/// neighbouring step is off by under 0.3% (no visible softening), coarse enough that float
/// noise in the zoom does not re-render.
const LEVEL_STEPS: f32 = 256.0;

/// The long side of a page's preview, in pixels: one tile. Every page shown gets one first,
/// so a fast scroll or zoom never shows an empty page for long.
pub(crate) const PREVIEW_PIXELS: f32 = 256.0;

/// A scale (pixels per point) as a level.
pub(crate) fn level_of(scale: f32) -> i32 {
    let l = (scale.max(f32::MIN_POSITIVE).log2() * LEVEL_STEPS).round();
    l.clamp(i32::MIN as f32 / 2.0, i32::MAX as f32 / 2.0) as i32
}

/// A level's scale (pixels per point).
pub(crate) fn scale_of(level: i32) -> f32 {
    (level as f32 / LEVEL_STEPS).exp2()
}

/// One tile of one page at one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TileId {
    pub(crate) page: usize,
    pub(crate) level: i32,
    pub(crate) col: u32,
    pub(crate) row: u32,
}

/// A page's grid at a level: its size in pixels and the tile edge.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Grid {
    pub(crate) scale: f32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) tile: u32,
}

impl Grid {
    pub(crate) fn new(page: remedy_ui_core::Size, level: i32, tile: u32) -> Grid {
        let scale = scale_of(level);
        let px = |v: f32| (v * scale).ceil().clamp(1.0, u32::MAX as f32) as u32;
        Grid { scale, width: px(page.width), height: px(page.height), tile: tile.max(1) }
    }

    /// Columns and rows of tiles.
    pub(crate) fn cells(&self) -> (u32, u32) {
        (self.width.div_ceil(self.tile), self.height.div_ceil(self.tile))
    }

    /// The cells touching `region` (page points).
    pub(crate) fn cells_in(&self, region: Rect) -> Vec<(u32, u32)> {
        let (cols, rows) = self.cells();
        let t = self.tile as f32 / self.scale;
        let start = |v: f32, n: u32| (v / t).floor().clamp(0.0, n as f32) as u32;
        let end = |v: f32, n: u32| (v / t).ceil().clamp(0.0, n as f32) as u32;
        let (c0, c1) = (start(region.min.x, cols), end(region.max.x, cols));
        let (r0, r1) = (start(region.min.y, rows), end(region.max.y, rows));
        let mut out = Vec::new();
        for r in r0..r1 {
            for c in c0..c1 {
                out.push((c, r));
            }
        }
        out
    }

    /// How many cells touch `region`, without listing them.
    pub(crate) fn count_in(&self, region: Rect) -> u64 {
        let (cols, rows) = self.cells();
        let t = self.tile as f32 / self.scale;
        let span = |a: f32, b: f32, n: u32| {
            let s = (a / t).floor().clamp(0.0, n as f32);
            let e = (b / t).ceil().clamp(0.0, n as f32);
            (e - s).max(0.0) as u64
        };
        span(region.min.x, region.max.x, cols) * span(region.min.y, region.max.y, rows)
    }

    /// A cell's pixel size and the page area it covers (points).
    pub(crate) fn cell(&self, (c, r): (u32, u32)) -> (u32, u32, Rect) {
        let x0 = c.saturating_mul(self.tile).min(self.width);
        let y0 = r.saturating_mul(self.tile).min(self.height);
        let x1 = x0.saturating_add(self.tile).min(self.width);
        let y1 = y0.saturating_add(self.tile).min(self.height);
        let s = self.scale;
        let rect = Rect::new(x0 as f32 / s, y0 as f32 / s, x1 as f32 / s, y1 as f32 / s);
        (x1 - x0, y1 - y0, rect)
    }
}

// ---------- the worker pool ----------

/// Work for the pool, cheapest-priority first.
struct Job {
    cancel: Cancel,
    priority: Arc<AtomicU64>,
    work: Box<dyn FnOnce() + Send>,
}

/// Jobs waiting for a worker.
static QUEUE: Mutex<Vec<Job>> = Mutex::new(Vec::new());

/// Runs the most urgent job not cancelled; false when none is left.
fn step() -> bool {
    let job = {
        let mut q = QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        q.retain(|j| !j.cancel.is_cancelled());
        let best =
            q.iter().enumerate().min_by_key(|(_, j)| j.priority.load(Ordering::Relaxed)).map(|(i, _)| i);
        match best {
            Some(i) => q.swap_remove(i),
            None => return false,
        }
    };
    let _span = remedy_ui_core::phase!("page raster").entered();
    (job.work)();
    true
}

fn pool() -> &'static remedy_ui_core::exec::Pool {
    static POOL: OnceLock<remedy_ui_core::exec::Pool> = OnceLock::new();
    // One worker per core, leaving one for the UI thread; parked while there is no work (in a
    // browser, cooperative slices between frames).
    POOL.get_or_init(|| {
        let n = remedy_ui_core::exec::parallelism().saturating_sub(1).max(1);
        remedy_ui_core::exec::Pool::new("remedy-pages", n, step)
    })
}

/// Queues `work` on the page workers. It is skipped if `cancel` is set before it starts.
pub(crate) fn spawn(cancel: Cancel, priority: Arc<AtomicU64>, work: impl FnOnce() + Send + 'static) {
    QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(Job {
        cancel,
        priority,
        work: Box::new(work),
    });
    pool().notify();
}

/// A priority: the class first (previews, then visible tiles, then prefetch, then
/// thumbnails and text), then distance from the middle of the view.
pub(crate) fn priority(class: u8, distance: f32) -> u64 {
    let d = distance.abs().min(u32::MAX as f32) as u64;
    (u64::from(class) << 40) | d.min((1 << 40) - 1)
}

/// Priority classes.
pub(crate) mod class {
    /// A page's preview.
    pub(crate) const PREVIEW: u8 = 0;
    /// A tile on screen.
    pub(crate) const VISIBLE: u8 = 1;
    /// Text of a page on screen (selection and the screen reader need it).
    pub(crate) const TEXT: u8 = 2;
    /// A tile just off screen.
    pub(crate) const PREFETCH: u8 = 3;
    /// A thumbnail.
    pub(crate) const THUMB: u8 = 4;
    /// Searching the document.
    pub(crate) const SEARCH: u8 = 5;
}

// ---------- getting results back to the UI thread ----------

/// What a worker finished.
pub(crate) enum Done {
    Tile(TileId, Result<Image, RenderError>),
    Text(usize, Option<PageText>),
    Found(u64, usize, Option<PageText>),
}

type Sink = Rc<dyn Fn(Done)>;

thread_local! {
    static SINKS: RefCell<HashMap<u64, Sink>> = RefCell::new(HashMap::new());
}

static NEXT_VIEWER: AtomicU64 = AtomicU64::new(1);

/// A new viewer id with `sink` receiving its results on this (the UI) thread.
pub(crate) fn register(sink: Sink) -> u64 {
    let id = NEXT_VIEWER.fetch_add(1, Ordering::Relaxed);
    SINKS.with(|s| s.borrow_mut().insert(id, sink));
    id
}

/// Stops delivering to a viewer that is gone.
pub(crate) fn unregister(viewer: u64) {
    let _ = SINKS.try_with(|s| s.borrow_mut().remove(&viewer));
}

fn deliver(viewer: u64, done: Done) {
    let sink = SINKS.with(|s| s.borrow().get(&viewer).cloned());
    if let Some(sink) = sink {
        sink(done);
    }
}

/// Runs `job` on a worker and hands its result to the viewer on the UI thread. Without a
/// UI-thread runtime to post back to (no reactive runtime on this thread, which a running
/// app always has) it runs here.
pub(crate) fn run(
    viewer: u64,
    cancel: Cancel,
    priority: Arc<AtomicU64>,
    job: impl FnOnce(&Cancel) -> Done + Send + 'static,
) {
    let Ok(waker) = remedy_ui_reactive::waker() else {
        let done = job(&cancel);
        deliver(viewer, done);
        return;
    };
    let c = cancel.clone();
    spawn(cancel, priority, move || {
        let done = job(&c);
        let _ = waker.post(move || deliver(viewer, done));
    });
}

// ---------- the tile cache ----------

struct Entry {
    image: Image,
    used: u64,
}

/// What a frame draws for a page: an image covering `region` (page points), clipped to
/// `clip` when it stands in for a missing tile.
#[derive(Clone, Debug)]
pub(crate) struct Draw {
    pub(crate) page: usize,
    pub(crate) image: Image,
    pub(crate) region: Rect,
    pub(crate) clip: Option<Rect>,
}

/// What part of a page shows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Visible {
    pub(crate) page: usize,
    /// The visible area, page points (unrotated).
    pub(crate) region: Rect,
    /// Distance of the area's middle from the view's middle, logical pixels.
    pub(crate) distance: f32,
    /// Near the view rather than on it: drawn (paper and preview) so a scroll or zoom the
    /// compositor moves finds it there, but only its preview is asked for.
    pub(crate) near: bool,
}

/// Counters for the view's work, for tests, devtools and the app's own status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageStats {
    /// Tiles the source rendered.
    pub rendered: u64,
    /// Requests withdrawn before or while rendering (scrolled away, zoom moved on).
    pub cancelled: u64,
    /// Tiles requested from the source, ever.
    pub requested: u64,
    /// Tiles held.
    pub cached_tiles: usize,
    /// Bytes of tile pixels held.
    pub cached_bytes: usize,
    /// The bytes the cache may hold now (sized from the machine and the view).
    pub cache_budget: usize,
    /// Renders queued or running.
    pub in_flight: usize,
    /// The largest number of tiles one frame wanted rendered.
    pub most_wanted: usize,
    /// Tiles the latest frame showed from another scale (or the page's preview) while their
    /// own scale renders.
    pub standing_in: usize,
    /// Tiles the latest frame had nothing to show for yet (plain paper).
    pub blank: usize,
}

struct Flight {
    cancel: Cancel,
    priority: Arc<AtomicU64>,
}

/// Everything a viewer knows about its tiles.
pub(crate) struct Tiles {
    done: HashMap<TileId, Entry>,
    /// Tiles held per page and level, so stand-ins are found without scanning the cache.
    levels: HashMap<usize, BTreeMap<i32, usize>>,
    flying: HashMap<TileId, Flight>,
    pub(crate) errors: HashMap<usize, String>,
    frame: u64,
    bytes: usize,
    pub(crate) stats: PageStats,
}

/// What one frame of tiles needs to know.
pub(crate) struct FrameCx<'a> {
    pub(crate) viewer: u64,
    pub(crate) source: &'a Arc<dyn PageSource>,
    pub(crate) sizes: &'a dyn Fn(usize) -> remedy_ui_core::Size,
    pub(crate) level: i32,
    pub(crate) tile: u32,
    /// The zoom has stopped moving: render the exact scale everywhere and prefetch.
    pub(crate) settled: bool,
    /// Bytes the cache may hold.
    pub(crate) budget: usize,
}

impl Tiles {
    pub(crate) fn new() -> Self {
        Tiles {
            done: HashMap::new(),
            levels: HashMap::new(),
            flying: HashMap::new(),
            errors: HashMap::new(),
            frame: 0,
            bytes: 0,
            stats: PageStats::default(),
        }
    }

    fn cached(&mut self, id: &TileId) -> Option<Image> {
        let frame = self.frame;
        self.done.get_mut(id).map(|e| {
            e.used = frame;
            e.image.clone()
        })
    }

    fn has(&self, id: &TileId) -> bool {
        self.done.contains_key(id)
    }

    /// Tiles of another level that together cover `area` of `page`, nearest the wanted level
    /// first (finer on a tie).
    fn stand_in(&mut self, cx: &FrameCx<'_>, page: usize, area: Rect) -> Option<Vec<(TileId, Rect)>> {
        let held: Vec<(i32, usize)> =
            self.levels.get(&page)?.iter().map(|(l, n)| (*l, *n)).filter(|(l, _)| *l != cx.level).collect();
        let mut order = held;
        order.sort_by_key(|(l, _)| ((*l - cx.level).abs(), -*l));
        let size = (cx.sizes)(page);
        for (level, held) in order {
            let grid = Grid::new(size, level, cx.tile);
            // A much finer level needs more tiles than it holds to cover the area: count
            // first (zooming far out from a deep zoom).
            if grid.count_in(area) > held as u64 {
                continue;
            }
            let cells = grid.cells_in(area);
            let ids: Vec<(TileId, Rect)> = cells
                .into_iter()
                .map(|(col, row)| (TileId { page, level, col, row }, grid.cell((col, row)).2))
                .filter(|(_, r)| r.intersect(area).is_some_and(|i| !i.is_empty()))
                .collect();
            if !ids.is_empty() && ids.iter().all(|(id, _)| self.has(id)) {
                return Some(ids);
            }
        }
        None
    }

    /// Plans one frame: what to draw for the visible pages, what to render, and what to
    /// stop rendering.
    pub(crate) fn frame(&mut self, cx: &FrameCx<'_>, visible: &[Visible]) -> Vec<Draw> {
        self.frame += 1;
        self.stats.standing_in = 0;
        self.stats.blank = 0;
        let mut draws = Vec::new();
        let mut wanted: HashMap<TileId, u64> = HashMap::new();
        let mut want = |id: TileId, p: u64| {
            let e = wanted.entry(id).or_insert(p);
            *e = (*e).min(p);
        };
        for v in visible {
            let size = (cx.sizes)(v.page);
            if size.is_empty() {
                continue;
            }
            let grid = Grid::new(size, cx.level, cx.tile);
            // A preview first, unless the page shows smaller than one anyway.
            let preview_level = level_of(PREVIEW_PIXELS / size.width.max(size.height));
            let preview = (cx.level > preview_level).then_some(TileId {
                page: v.page,
                level: preview_level,
                col: 0,
                row: 0,
            });
            if v.near {
                // At or below the preview's scale the page is a single tile anyway.
                let id = preview.unwrap_or(TileId { page: v.page, level: cx.level, col: 0, row: 0 });
                match self.cached(&id) {
                    Some(image) => {
                        let whole = Rect::new(0.0, 0.0, size.width, size.height);
                        let region = if id.level == cx.level { grid.cell((0, 0)).2 } else { whole };
                        draws.push(Draw { page: v.page, image, region, clip: None });
                    }
                    None => want(id, priority(class::PREFETCH, v.distance)),
                }
                continue;
            }
            if let Some(p) = preview
                && !self.has(&p)
            {
                want(p, priority(class::PREVIEW, v.distance));
            }
            for cell in grid.cells_in(v.region) {
                let (_, _, area) = grid.cell(cell);
                let id = TileId { page: v.page, level: cx.level, col: cell.0, row: cell.1 };
                if let Some(image) = self.cached(&id) {
                    draws.push(Draw { page: v.page, image, region: area, clip: None });
                    continue;
                }
                let standing = self.stand_in(cx, v.page, area);
                let found = standing.is_some();
                match standing {
                    Some(tiles) => {
                        self.stats.standing_in += 1;
                        for (sid, region) in tiles {
                            if let Some(image) = self.cached(&sid) {
                                draws.push(Draw { page: v.page, image, region, clip: Some(area) });
                            }
                        }
                    }
                    None => match preview.and_then(|p| self.cached(&p)) {
                        Some(image) => {
                            self.stats.standing_in += 1;
                            let whole = Rect::new(0.0, 0.0, size.width, size.height);
                            draws.push(Draw { page: v.page, image, region: whole, clip: Some(area) });
                        }
                        None => self.stats.blank += 1,
                    },
                }
                // While the zoom moves, tiles that something stands in for wait for it to stop.
                if cx.settled || !found {
                    want(id, priority(class::VISIBLE, v.distance));
                }
            }
            if cx.settled {
                // One tile around what shows, so a scroll finds them ready.
                let t = cx.tile as f32 / grid.scale;
                let around =
                    Rect::new(v.region.min.x - t, v.region.min.y - t, v.region.max.x + t, v.region.max.y + t);
                for cell in grid.cells_in(around) {
                    let id = TileId { page: v.page, level: cx.level, col: cell.0, row: cell.1 };
                    if !self.has(&id) {
                        want(id, priority(class::PREFETCH, v.distance));
                    }
                }
            }
        }
        self.stats.most_wanted = self.stats.most_wanted.max(wanted.len());
        // Work nobody wants now stops: queued work is skipped, running work is told.
        let stale: Vec<TileId> = self.flying.keys().filter(|id| !wanted.contains_key(id)).copied().collect();
        for id in stale {
            if let Some(f) = self.flying.remove(&id) {
                f.cancel.cancel();
                self.stats.cancelled += 1;
            }
        }
        for (id, p) in wanted {
            if self.has(&id) {
                continue;
            }
            if let Some(f) = self.flying.get(&id) {
                f.priority.store(p, Ordering::Relaxed);
                continue;
            }
            self.start(cx, id, p);
        }
        self.stats.in_flight = self.flying.len();
        self.trim(cx.budget);
        draws
    }

    fn start(&mut self, cx: &FrameCx<'_>, id: TileId, p: u64) {
        let size = (cx.sizes)(id.page);
        let grid = Grid::new(size, id.level, cx.tile);
        let (width, height, region) = grid.cell((id.col, id.row));
        let request = RenderRequest { page: id.page, region, scale: grid.scale, width, height };
        let cancel = Cancel::new();
        let priority = Arc::new(AtomicU64::new(p));
        self.flying.insert(id, Flight { cancel: cancel.clone(), priority: priority.clone() });
        self.stats.requested += 1;
        let source = cx.source.clone();
        run(cx.viewer, cancel, priority, move |c| {
            let result =
                if c.is_cancelled() { Err(RenderError::Cancelled) } else { source.render(&request, c) };
            Done::Tile(id, result)
        });
    }

    /// Takes a finished render.
    pub(crate) fn adopt(
        &mut self,
        id: TileId,
        result: Result<Image, RenderError>,
        tile: u32,
        size: remedy_ui_core::Size,
    ) {
        self.flying.remove(&id);
        match result {
            Ok(image) => {
                let (w, h, _) = Grid::new(size, id.level, tile).cell((id.col, id.row));
                let fits = image.width == w
                    && image.height == h
                    && image.pixels.len() as u64 == u64::from(w) * u64::from(h) * 4;
                if !fits {
                    self.errors.insert(
                        id.page,
                        format!(
                            "the page source drew {}×{} pixels for a {w}×{h} tile",
                            image.width, image.height
                        ),
                    );
                    return;
                }
                self.bytes += image.pixels.len();
                match self.done.insert(id, Entry { image, used: self.frame }) {
                    Some(old) => self.bytes = self.bytes.saturating_sub(old.image.pixels.len()),
                    None => *self.levels.entry(id.page).or_default().entry(id.level).or_insert(0) += 1,
                }
                self.stats.rendered += 1;
                self.errors.remove(&id.page);
            }
            Err(RenderError::Cancelled) => {}
            Err(RenderError::Failed(why)) => {
                self.errors.insert(id.page, why);
            }
        }
        self.sync_stats();
    }

    fn remove(&mut self, id: &TileId) {
        if let Some(e) = self.done.remove(id) {
            self.bytes = self.bytes.saturating_sub(e.image.pixels.len());
            let empty = self.levels.get_mut(&id.page).is_some_and(|l| {
                let gone = l.get_mut(&id.level).is_some_and(|n| {
                    *n = n.saturating_sub(1);
                    *n == 0
                });
                if gone {
                    l.remove(&id.level);
                }
                l.is_empty()
            });
            if empty {
                self.levels.remove(&id.page);
            }
        }
    }

    /// Drops the least recently drawn tiles until the cache is within `budget` bytes. Tiles
    /// drawn in the latest frame always stay.
    pub(crate) fn trim(&mut self, budget: usize) {
        self.stats.cache_budget = budget;
        if self.bytes > budget {
            let mut old: Vec<(u64, TileId)> =
                self.done.iter().filter(|(_, e)| e.used < self.frame).map(|(id, e)| (e.used, *id)).collect();
            old.sort();
            for (_, id) in old {
                if self.bytes <= budget {
                    break;
                }
                self.remove(&id);
            }
        }
        self.sync_stats();
    }

    /// Lets go of every tile not drawn in the latest frame (memory is short).
    pub(crate) fn release_unused(&mut self) {
        let old: Vec<TileId> =
            self.done.iter().filter(|(_, e)| e.used < self.frame).map(|(id, _)| *id).collect();
        for id in old {
            self.remove(&id);
        }
        self.sync_stats();
    }

    /// Forgets everything (the document changed), stopping what is rendering.
    pub(crate) fn clear(&mut self) {
        for (_, f) in self.flying.drain() {
            f.cancel.cancel();
        }
        self.done.clear();
        self.levels.clear();
        self.errors.clear();
        self.bytes = 0;
        self.sync_stats();
    }

    fn sync_stats(&mut self) {
        self.stats.cached_tiles = self.done.len();
        self.stats.cached_bytes = self.bytes;
        self.stats.in_flight = self.flying.len();
    }

    /// True while renders are queued or running.
    pub(crate) fn busy(&self) -> bool {
        !self.flying.is_empty()
    }

    /// A cached image by id, marking it drawn (thumbnails).
    pub(crate) fn get(&mut self, id: &TileId) -> Option<Image> {
        self.cached(id)
    }

    /// Asks for one tile outside the frame plan (a thumbnail); its render is withdrawn with
    /// `cancel`.
    pub(crate) fn request_one(&mut self, cx: &FrameCx<'_>, id: TileId, class: u8, cancel: Cancel) {
        if self.has(&id) {
            return;
        }
        let size = (cx.sizes)(id.page);
        let grid = Grid::new(size, id.level, cx.tile);
        let (width, height, region) = grid.cell((id.col, id.row));
        let request = RenderRequest { page: id.page, region, scale: grid.scale, width, height };
        let source = cx.source.clone();
        self.stats.requested += 1;
        let p = Arc::new(AtomicU64::new(priority(class, id.page as f32)));
        run(cx.viewer, cancel, p, move |c| {
            let result =
                if c.is_cancelled() { Err(RenderError::Cancelled) } else { source.render(&request, c) };
            Done::Tile(id, result)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_core::Size;

    #[test]
    fn levels_round_trip_closely() {
        for s in [0.01f32, 0.5, 1.0, 1.37, 64.0] {
            let back = scale_of(level_of(s));
            assert!((back / s - 1.0).abs() < 0.003, "{s} -> {back}");
        }
        assert!(level_of(2.0) > level_of(1.0));
    }

    #[test]
    fn grids_cover_pages_with_edge_tiles_cut_to_size() {
        let g = Grid::new(Size::new(612.0, 792.0), level_of(1.0), 256);
        assert_eq!((g.width, g.height), (612, 792));
        assert_eq!(g.cells(), (3, 4));
        let (w, h, r) = g.cell((2, 3));
        assert_eq!((w, h), (100, 24));
        assert!((r.max.x - 612.0).abs() < 1e-3 && (r.max.y - 792.0).abs() < 1e-3);
        assert_eq!(g.cells_in(Rect::new(0.0, 0.0, 10.0, 10.0)), vec![(0, 0)]);
        assert_eq!(g.cells_in(Rect::new(250.0, 250.0, 300.0, 300.0)).len(), 4);
        assert_eq!(g.cells_in(Rect::new(-50.0, -50.0, 2000.0, 2000.0)).len(), 12);
    }

    #[test]
    fn priorities_order_by_class_then_distance() {
        assert!(priority(class::PREVIEW, 1e9) < priority(class::VISIBLE, 0.0));
        assert!(priority(class::VISIBLE, 10.0) < priority(class::VISIBLE, 20.0));
        assert!(priority(class::PREFETCH, 0.0) < priority(class::THUMB, 0.0));
    }
}
