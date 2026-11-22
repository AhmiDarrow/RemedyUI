//! Loading off the UI thread: a worker pool sized from the machine, newest requests first,
//! with work for images nobody is waiting for any more skipped; and a cache that shares an
//! image while anything shows it and lets it go when nothing does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use remedy_ui_paint::Image;
use remedy_ui_reactive::Signal;

use crate::decode::{
    DecodeError, Decoded, decode, dimensions, dimensions_from, fitted_scalable, format_of, router,
};

/// Where an image comes from.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Source {
    /// A file, read off the UI thread. Its cache entry is checked against the file's size
    /// and modification time, so an edited file shows its new pixels.
    File(PathBuf),
    /// Bytes already in memory (decrypted files, downloads, embedded assets). Requests with
    /// the same `Arc` share a cache entry.
    Bytes(Arc<[u8]>),
}

impl Source {
    /// What identifies the source in a cache: the path of a file, or the allocation of bytes.
    /// An allocation's address can be reused once it is freed, so a cache keyed by this must
    /// keep the source (or a `Weak` to its bytes) for as long as it keeps the entry.
    pub fn id(&self) -> SourceId {
        SourceId(self.key())
    }

    /// The file's size and modification time now; `None` for bytes, and for a file that
    /// cannot be read. A cache entry whose stamp differs is out of date.
    pub fn stamp(&self) -> Option<Stamp> {
        match self {
            Source::File(p) => stamp(p),
            Source::Bytes(_) => None,
        }
    }

    /// The source's bytes (reads a file whole), and the file's stamp as it was read. Do this
    /// off the UI thread.
    pub fn read(&self) -> Result<(Arc<[u8]>, Option<Stamp>), DecodeError> {
        self.read_bytes()
    }

    fn key(&self) -> SourceKey {
        match self {
            Source::File(p) => SourceKey::File(p.clone()),
            Source::Bytes(b) => SourceKey::Bytes(b.as_ptr() as usize, b.len()),
        }
    }

    /// The source's size, reading only a file's header.
    fn size(&self) -> Result<(u32, u32), DecodeError> {
        if let Some(r) = router() {
            // Headers are untrusted bytes too: the router reads them where it decodes.
            let (bytes, _) = self.read_bytes()?;
            return r.dimensions(&bytes);
        }
        match self {
            Source::Bytes(b) => dimensions(b),
            Source::File(p) => {
                use std::io::{BufRead, Seek};
                let f = std::fs::File::open(p).map_err(|e| DecodeError::Io(e.to_string()))?;
                let mut r = std::io::BufReader::with_capacity(8192, f);
                let head = r.fill_buf().map_err(|e| DecodeError::Io(e.to_string()))?;
                if format_of(head).is_some() {
                    // Added formats (SVG) need the whole file for their size.
                    let bytes = std::fs::read(p).map_err(|e| DecodeError::Io(e.to_string()))?;
                    return dimensions(&bytes);
                }
                r.rewind().map_err(|e| DecodeError::Io(e.to_string()))?;
                dimensions_from(r)
            }
        }
    }

    fn read_bytes(&self) -> Result<(Arc<[u8]>, Option<Stamp>), DecodeError> {
        match self {
            Source::Bytes(b) => Ok((b.clone(), None)),
            Source::File(p) => {
                let stamp = stamp(p);
                let bytes = std::fs::read(p).map_err(|e| DecodeError::Io(e.to_string()))?;
                Ok((Arc::from(bytes), stamp))
            }
        }
    }
}

/// A file's size and modification time, to notice edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp(u64, std::time::SystemTime);

fn stamp(p: &std::path::Path) -> Option<Stamp> {
    let m = std::fs::metadata(p).ok()?;
    Some(Stamp(m.len(), m.modified().ok()?))
}

/// What identifies a [`Source`] in a cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(SourceKey);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SourceKey {
    File(PathBuf),
    Bytes(usize, usize),
}

/// Where a load is.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ImageState {
    /// Reading or decoding.
    Loading,
    /// The source's size is known (pixels, upright); the pixels are still on their way.
    Sized((u32, u32)),
    /// Decoded: the pixels at the requested size, and the source's size.
    Ready(Image, (u32, u32)),
    /// It could not be shown, and why.
    Failed(DecodeError),
}

// ---------- the worker pool ----------

type Job = Box<dyn FnOnce() + Send>;

/// Jobs waiting for a worker, oldest first.
static QUEUE: Mutex<Vec<(Arc<AtomicBool>, Job)>> = Mutex::new(Vec::new());

/// Runs the newest job not cancelled; false when none is left.
fn step() -> bool {
    let job = {
        let mut q = QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            // Newest first: what was asked for last is what is on screen now.
            match q.pop() {
                Some((cancelled, _)) if cancelled.load(Ordering::Relaxed) => {}
                Some((_, job)) => break job,
                None => return false,
            }
        }
    };
    let _span = remedy_ui_trace::phase!("image work").entered();
    job();
    true
}

fn pool() -> &'static remedy_ui_core::exec::Pool {
    static POOL: OnceLock<remedy_ui_core::exec::Pool> = OnceLock::new();
    // One worker per core, leaving one for the UI thread (cooperative slices in a browser).
    POOL.get_or_init(|| {
        let n = remedy_ui_core::exec::parallelism().saturating_sub(1).max(1);
        remedy_ui_core::exec::Pool::new("remedy-image", n, step)
    })
}

/// Runs `job` on the image workers (one per core but one, newest request first). A job whose
/// `cancelled` flag is set before it starts is skipped. Long work should be done in short
/// jobs that queue the next, so other images get their turn.
pub fn spawn_job(cancelled: Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
    spawn(cancelled, job);
}

fn spawn(cancelled: Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
    QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((cancelled, Box::new(job)));
    pool().notify();
}

// ---------- the cache ----------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    source: SourceKey,
    /// The box asked for, in device pixels, rounded up to a power of two (0 for the
    /// source's own size), and whether the image covers it.
    bucket: (u32, u32),
    cover: bool,
}

struct Done {
    pixels: Weak<[u8]>,
    /// Holds the bytes' allocation so its address is not reused while this entry exists.
    bytes: Option<Weak<[u8]>>,
    id: u64,
    size: (u32, u32),
    source_size: (u32, u32),
    stamp: Option<Stamp>,
}

struct Pending {
    waiters: Vec<(u64, Signal<ImageState>)>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct Cache {
    done: HashMap<Key, Done>,
    pending: HashMap<Key, Pending>,
    sizes: HashMap<SourceKey, Known>,
    next_waiter: u64,
    tick: u64,
}

/// How many source sizes are remembered; the least recently used go first.
const SIZES_CAP: usize = 1024;

/// A source's size, as last read.
struct Known {
    size: (u32, u32),
    /// The file's stamp when its size was read.
    stamp: Option<Stamp>,
    /// Holds the bytes' allocation so its address is not reused while this entry exists.
    bytes: Option<Weak<[u8]>>,
    used: u64,
}

/// A weak hold on a bytes source, which keeps its address from being reused.
fn hold(source: &Source) -> Option<Weak<[u8]>> {
    match source {
        Source::Bytes(b) => Some(Arc::downgrade(b)),
        Source::File(_) => None,
    }
}

/// True when the bytes held are freed: nobody can ask for them again.
fn gone(bytes: &Option<Weak<[u8]>>) -> bool {
    bytes.as_ref().is_some_and(|w| w.strong_count() == 0)
}

impl Cache {
    /// The size known for `source`, and the file stamp it was read at.
    fn known(&mut self, source: &SourceKey) -> Option<((u32, u32), Option<Stamp>)> {
        self.tick += 1;
        let tick = self.tick;
        let k = self.sizes.get_mut(source).filter(|k| !gone(&k.bytes))?;
        k.used = tick;
        Some((k.size, k.stamp))
    }

    fn remember(
        &mut self,
        source: SourceKey,
        size: (u32, u32),
        stamp: Option<Stamp>,
        bytes: Option<Weak<[u8]>>,
    ) {
        self.tick += 1;
        self.sizes.insert(source, Known { size, stamp, bytes, used: self.tick });
        // Sizes of freed bytes go (letting their allocations go too), then the least recently used.
        self.sizes.retain(|_, k| !gone(&k.bytes));
        if self.sizes.len() > SIZES_CAP
            && let Some(oldest) = self.sizes.iter().min_by_key(|(_, k)| k.used).map(|(s, _)| s.clone())
        {
            self.sizes.remove(&oldest);
        }
    }
}

/// How many source sizes are remembered (for tests and the inspector).
pub fn known_sizes() -> usize {
    CACHE.with(|c| c.borrow().sizes.len())
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

fn bucket(px: u32) -> u32 {
    if px == 0 { 0 } else { px.next_power_of_two() }
}

/// A request for an image: dropping it (when the element showing it goes) withdraws it, and
/// the work is skipped if nobody else wants the same image.
#[derive(Debug)]
pub struct Request {
    key: Key,
    waiter: u64,
}

impl Drop for Request {
    fn drop(&mut self) {
        let _ = CACHE.try_with(|c| {
            let mut c = c.borrow_mut();
            let empty = c.pending.get_mut(&self.key).is_some_and(|p| {
                p.waiters.retain(|(w, _)| *w != self.waiter);
                p.waiters.is_empty()
            });
            if empty && let Some(p) = c.pending.remove(&self.key) {
                p.cancelled.store(true, Ordering::Relaxed);
            }
        });
    }
}

/// Loads `source` to show in a box of `box_px` device pixels (`None`: at its own size),
/// fitting inside it or (`cover`) covering it. `state` hears about progress: the cached image
/// at once when another element shows it, otherwise `Loading`, then `Ready` or `Failed`.
///
/// Keep the returned request while the image is wanted.
pub fn load(source: &Source, box_px: Option<(u32, u32)>, cover: bool, state: Signal<ImageState>) -> Request {
    let key =
        Key { source: source.key(), bucket: box_px.map_or((0, 0), |(w, h)| (bucket(w), bucket(h))), cover };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.next_waiter += 1;
        let waiter = c.next_waiter;
        let request = Request { key: key.clone(), waiter };
        let cached =
            c.done.get(&key).filter(|d| !gone(&d.bytes)).and_then(|d| d.pixels.upgrade().map(|px| (d, px)));
        if let Some((d, pixels)) = cached {
            let image = Image { id: d.id, width: d.size.0, height: d.size.1, pixels };
            state.set(ImageState::Ready(image, d.source_size));
            // A file may have changed since: look, off the UI thread, and reload if it did.
            if let (Source::File(p), Some(old)) = (source, d.stamp) {
                revalidate(p.clone(), old, key, source.clone(), state);
            }
            return request;
        }
        c.done.remove(&key);
        // A file's size may be stale here; the decode below brings the current one.
        if let Some((size, _)) = c.known(&key.source) {
            state.set(ImageState::Sized(size));
        } else {
            state.set(ImageState::Loading);
        }
        if let Some(p) = c.pending.get_mut(&key) {
            p.waiters.push((waiter, state));
            return request;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        c.pending
            .insert(key.clone(), Pending { waiters: vec![(waiter, state)], cancelled: cancelled.clone() });
        drop(c);
        start(source.clone(), key, cancelled);
        request
    })
}

fn start(source: Source, key: Key, cancelled: Arc<AtomicBool>) {
    let held = hold(&source);
    let Ok(waker) = remedy_ui_reactive::waker() else {
        let result = decode_job(&source, &key_box(&key), key.cover);
        finish(key, held, result);
        return;
    };
    let box_px = key_box(&key);
    let cover = key.cover;
    let c2 = cancelled.clone();
    spawn(cancelled, move || {
        if c2.load(Ordering::Relaxed) {
            return;
        }
        let result = decode_job(&source, &box_px, cover);
        let _ = waker.post(move || finish(key, held, result));
    });
}

fn key_box(key: &Key) -> Option<(u32, u32)> {
    (key.bucket != (0, 0)).then_some(key.bucket)
}

type JobResult = Result<(Image, (u32, u32), Option<Stamp>), DecodeError>;

/// Decodes `bytes` to show in a box of `box_px` device pixels (`None`: at their own size),
/// fitting inside it or (`cover`) covering it: the decoding [`load`] does, for callers that
/// already hold the bytes and run on a worker of their own. With a [`Router`](crate::Router)
/// set, the decoding happens there.
pub fn decode_for_box(bytes: &[u8], box_px: Option<(u32, u32)>, cover: bool) -> Result<Decoded, DecodeError> {
    if let Some(r) = router() {
        return r.decode_for_box(bytes, box_px, cover);
    }
    let show_at = match box_px {
        Some(b) => {
            let scalable = format_of(bytes).is_some_and(|f| f.scalable());
            Some(fitted_scalable(dimensions(bytes)?, b, cover, scalable))
        }
        None => None,
    };
    decode(bytes, show_at)
}

fn decode_job(source: &Source, box_px: &Option<(u32, u32)>, cover: bool) -> JobResult {
    let (bytes, stamp) = source.read_bytes()?;
    let d = decode_for_box(&bytes, *box_px, cover)?;
    Ok((d.image, d.source_size, stamp))
}

fn finish(key: Key, held: Option<Weak<[u8]>>, result: JobResult) {
    let waiters = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let p = c.pending.remove(&key);
        if let Ok((image, source_size, stamp)) = &result {
            c.remember(key.source.clone(), *source_size, *stamp, held.clone());
            c.done.insert(
                key.clone(),
                Done {
                    pixels: Arc::downgrade(&image.pixels),
                    bytes: held,
                    id: image.id,
                    size: (image.width, image.height),
                    source_size: *source_size,
                    stamp: *stamp,
                },
            );
        }
        // Entries for images nobody holds any more, or whose bytes are freed, are dropped as
        // they are found.
        c.done.retain(|_, d| d.pixels.strong_count() > 0 && !gone(&d.bytes));
        p.map(|p| p.waiters).unwrap_or_default()
    });
    let state = match result {
        Ok((image, size, _)) => ImageState::Ready(image, size),
        Err(e) => ImageState::Failed(e),
    };
    for (_, s) in waiters {
        // An element that went away has disposed its signal; that is fine.
        let _ = s.try_set(state.clone());
    }
}

fn revalidate(path: PathBuf, old: Stamp, key: Key, source: Source, state: Signal<ImageState>) {
    let Ok(waker) = remedy_ui_reactive::waker() else { return };
    let cancelled = Arc::new(AtomicBool::new(false));
    spawn(cancelled, move || {
        if stamp(&path) == Some(old) {
            return;
        }
        let result = decode_job(&source, &key_box(&key), key.cover);
        let _ = waker.post(move || {
            if let Ok((image, size, stamp)) = &result {
                CACHE.with(|c| {
                    let mut c = c.borrow_mut();
                    c.remember(key.source.clone(), *size, *stamp, None);
                    c.done.insert(
                        key.clone(),
                        Done {
                            pixels: Arc::downgrade(&image.pixels),
                            bytes: None,
                            id: image.id,
                            size: (image.width, image.height),
                            source_size: *size,
                            stamp: *stamp,
                        },
                    );
                });
            }
            let _ = state.try_set(match result {
                Ok((image, size, _)) => ImageState::Ready(image, size),
                Err(e) => ImageState::Failed(e),
            });
        });
    });
}

/// Reads the source's size off the UI thread, for laying out before the pixels arrive.
pub fn probe(source: &Source, state: Signal<ImageState>) {
    let known = CACHE.with(|c| c.borrow_mut().known(&source.key()));
    if let Some((size, _)) = known {
        state.set(ImageState::Sized(size));
        // Bytes never change; a file may have, so its stamp is checked off the UI thread.
        if matches!(source, Source::Bytes(_)) {
            return;
        }
    }
    let src = source.clone();
    // The stamp is taken before the size, so an edit in between is noticed next time.
    let read = move || {
        let now = src.stamp();
        if known.is_some_and(|(_, old)| old == now) {
            return None;
        }
        Some((src.size(), now, src))
    };
    let apply = move |(r, stamp, src): (Result<(u32, u32), DecodeError>, Option<Stamp>, Source)| {
        if let Ok(size) = &r {
            CACHE.with(|c| c.borrow_mut().remember(src.key(), *size, stamp, hold(&src)));
        }
        // Only move forward: a load may already have finished.
        let unloaded = state
            .with_untracked(|s| matches!(s, ImageState::Loading | ImageState::Sized(_)))
            .unwrap_or(false);
        if unloaded || r.is_err() {
            let _ = state.try_set(r.map_or_else(ImageState::Failed, ImageState::Sized));
        }
    };
    let Ok(waker) = remedy_ui_reactive::waker() else {
        if let Some(r) = read() {
            apply(r);
        }
        return;
    };
    spawn(Arc::new(AtomicBool::new(false)), move || {
        if let Some(r) = read() {
            let _ = waker.post(move || apply(r));
        }
    });
}

/// How many decoded images are cached and alive (for tests and the inspector).
pub fn cached() -> usize {
    CACHE.with(|c| c.borrow().done.values().filter(|d| d.pixels.strong_count() > 0).count())
}
