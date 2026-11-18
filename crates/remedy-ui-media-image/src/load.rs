//! Loading off the UI thread: a worker pool sized from the machine, newest requests first,
//! with work for images nobody is waiting for any more skipped; and a cache that shares an
//! image while anything shows it and lets it go when nothing does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

use remedy_ui_paint::Image;
use remedy_ui_reactive::Signal;

use crate::decode::{DecodeError, decode, dimensions, dimensions_from, fitted_scalable, format_of};

/// Where an image comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// A file, read off the UI thread. Its cache entry is checked against the file's size
    /// and modification time, so an edited file shows its new pixels.
    File(PathBuf),
    /// Bytes already in memory (decrypted files, downloads, embedded assets). Requests with
    /// the same `Arc` share a cache entry.
    Bytes(Arc<[u8]>),
}

impl Source {
    fn key(&self) -> SourceKey {
        match self {
            Source::File(p) => SourceKey::File(p.clone()),
            Source::Bytes(b) => SourceKey::Bytes(b.as_ptr() as usize, b.len()),
        }
    }

    /// The source's size, reading only a file's header.
    fn size(&self) -> Result<(u32, u32), DecodeError> {
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

    fn read(&self) -> Result<(Arc<[u8]>, Option<Stamp>), DecodeError> {
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
type Stamp = (u64, std::time::SystemTime);

fn stamp(p: &std::path::Path) -> Option<Stamp> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SourceKey {
    File(PathBuf),
    Bytes(usize, usize),
}

/// Where a load is.
#[derive(Clone, Debug)]
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

struct Pool {
    queue: Mutex<Vec<(Arc<AtomicBool>, Job)>>,
    ready: Condvar,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<&'static Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let pool: &'static Pool =
            Box::leak(Box::new(Pool { queue: Mutex::new(Vec::new()), ready: Condvar::new() }));
        // One worker per core, leaving one for the UI thread.
        let n = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1);
        for i in 0..n {
            let _ = std::thread::Builder::new().name(format!("remedy-image-{i}")).spawn(move || worker(pool));
        }
        pool
    })
}

fn worker(pool: &'static Pool) {
    loop {
        let job = {
            let Ok(mut q) = pool.queue.lock() else { return };
            loop {
                // Newest first: what was asked for last is what is on screen now.
                match q.pop() {
                    Some((cancelled, _)) if cancelled.load(Ordering::Relaxed) => {}
                    Some((_, job)) => break job,
                    None => match pool.ready.wait(q) {
                        Ok(g) => q = g,
                        Err(_) => return,
                    },
                }
            }
        };
        job();
    }
}

fn spawn(cancelled: Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
    let p = pool();
    if let Ok(mut q) = p.queue.lock() {
        q.push((cancelled, Box::new(job)));
        p.ready.notify_one();
    }
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
    sizes: HashMap<SourceKey, (u32, u32)>,
    next_waiter: u64,
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
        let cached = c.done.get(&key).and_then(|d| d.pixels.upgrade().map(|px| (d, px)));
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
        if let Some(size) = c.sizes.get(&key.source).copied() {
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
    let Ok(waker) = remedy_ui_reactive::waker() else {
        let result = decode_job(&source, &key_box(&key), key.cover);
        finish(key, result);
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
        let _ = waker.post(move || finish(key, result));
    });
}

fn key_box(key: &Key) -> Option<(u32, u32)> {
    (key.bucket != (0, 0)).then_some(key.bucket)
}

type JobResult = Result<(Image, (u32, u32), Option<Stamp>), DecodeError>;

fn decode_job(source: &Source, box_px: &Option<(u32, u32)>, cover: bool) -> JobResult {
    let (bytes, stamp) = source.read()?;
    let show_at = match box_px {
        Some(b) => {
            let scalable = format_of(&bytes).is_some_and(|f| f.scalable());
            Some(fitted_scalable(dimensions(&bytes)?, *b, cover, scalable))
        }
        None => None,
    };
    let d = decode(&bytes, show_at)?;
    Ok((d.image, d.source_size, stamp))
}

fn finish(key: Key, result: JobResult) {
    let waiters = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let p = c.pending.remove(&key);
        if let Ok((image, source_size, stamp)) = &result {
            c.sizes.insert(key.source.clone(), *source_size);
            c.done.insert(
                key.clone(),
                Done {
                    pixels: Arc::downgrade(&image.pixels),
                    id: image.id,
                    size: (image.width, image.height),
                    source_size: *source_size,
                    stamp: *stamp,
                },
            );
        }
        // Entries for images nobody holds any more are dropped as they are found.
        c.done.retain(|_, d| d.pixels.strong_count() > 0);
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
                    c.borrow_mut().done.insert(
                        key.clone(),
                        Done {
                            pixels: Arc::downgrade(&image.pixels),
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
    if let Some(size) = CACHE.with(|c| c.borrow().sizes.get(&source.key()).copied()) {
        state.set(ImageState::Sized(size));
        return;
    }
    let src = source.clone();
    let Ok(waker) = remedy_ui_reactive::waker() else {
        let r = src.size();
        let _ = state.try_set(r.map_or_else(ImageState::Failed, ImageState::Sized));
        return;
    };
    spawn(Arc::new(AtomicBool::new(false)), move || {
        let r = src.size();
        let _ = waker.post(move || {
            if let Ok(size) = &r {
                CACHE.with(|c| {
                    c.borrow_mut().sizes.insert(src.key(), *size);
                });
            }
            // Only move forward: a load may already have finished.
            let still_loading = state.with_untracked(|s| matches!(s, ImageState::Loading)).unwrap_or(false);
            if still_loading || r.is_err() {
                let _ = state.try_set(r.map_or_else(ImageState::Failed, ImageState::Sized));
            }
        });
    });
}

/// How many decoded images are cached and alive (for tests and the inspector).
pub fn cached() -> usize {
    CACHE.with(|c| c.borrow().done.values().filter(|d| d.pixels.strong_count() > 0).count())
}
