//! Images from addresses ([`Source::Url`](crate::Source::Url)): fetched through a [`Fetch`]
//! (the engine's own by default, or the app's), kept in a bounded memory cache and, natively,
//! on disk in the app's cache folder ([`set_cache_dir`]), following the server's
//! `Cache-Control` (`max-age`, `no-cache`, `no-store`) and revalidating with `ETag` and
//! `Last-Modified`. In a browser page the browser's own HTTP cache is the disk cache.
//!
//! Requests for the same address share one fetch; it is cancelled when nobody wants it any
//! more (every element showing it gone).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use crate::decode::DecodeError;

/// Fetches the bytes at an address. The engine has its own (natively HTTPS through ureq and
/// rustls with the platform's certificates, in a page the browser's `fetch`); an app sets
/// another with [`set_fetcher`] (its own HTTP client, with its cookies or auth headers).
pub trait Fetch: Send + Sync {
    /// Starts fetching `request` and calls `done` once with the server's reply (any status:
    /// a `304` answers a request with validators), from any thread. Once `cancelled` is set
    /// nobody waits for the reply any more: stop early, and `done` may be dropped uncalled.
    fn fetch(&self, request: FetchRequest, cancelled: Arc<AtomicBool>, done: FetchDone);
}

/// What a [`Fetch`] calls with its answer: the reply, or why there is none.
pub type FetchDone = Box<dyn FnOnce(Result<FetchReply, String>) + Send>;

/// What to fetch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FetchRequest {
    /// The address.
    pub url: String,
    /// Send as `If-None-Match`: the `ETag` of the copy on disk.
    pub if_none_match: Option<String>,
    /// Send as `If-Modified-Since`: the `Last-Modified` of the copy on disk.
    pub if_modified_since: Option<String>,
}

/// A server's reply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FetchReply {
    /// The HTTP status.
    pub status: u16,
    /// The headers (names in any case).
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl FetchReply {
    /// A reply with `status`, `headers` and `body`.
    pub fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        FetchReply { status, headers, body }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Encoded bytes kept in memory, most recently used kept longest.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// Bytes kept on disk; the least recently used files go first.
const DISK_LIMIT: u64 = 256 * 1024 * 1024;

static FETCHER: RwLock<Option<Arc<dyn Fetch>>> = RwLock::new(None);
static DISK: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Fetches images from addresses with `fetcher` instead of the engine's own (`None`: the
/// engine's own again).
pub fn set_fetcher(fetcher: Option<Arc<dyn Fetch>>) {
    *FETCHER.write().unwrap_or_else(PoisonError::into_inner) = fetcher;
}

/// Keeps fetched images in `dir` across runs (`None`: in memory only). `remedy_ui::run` sets
/// it to the app's cache folder (`images` there). Natively only: a page has the browser's
/// HTTP cache.
pub fn set_cache_dir(dir: Option<PathBuf>) {
    *DISK.write().unwrap_or_else(PoisonError::into_inner) = dir;
}

fn fetcher() -> Arc<dyn Fetch> {
    FETCHER.read().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(|| {
        static OWN: OnceLock<Arc<dyn Fetch>> = OnceLock::new();
        OWN.get_or_init(|| Arc::new(Own)).clone()
    })
}

/// Seconds since 1970; 0 in a page, where the browser keeps time for its own cache and
/// memory entries last the session.
fn now() -> u64 {
    if cfg!(target_arch = "wasm32") {
        return 0;
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------- what the server says about keeping it ----------

#[derive(Debug, Default, PartialEq, Eq)]
struct Policy {
    /// May go on disk (not `no-store`).
    store: bool,
    /// Fresh for this long (`no-cache` is 0: ask the server each time).
    max_age: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
}

fn policy(reply: &FetchReply) -> Policy {
    let mut p = Policy {
        store: true,
        etag: reply.header("etag").map(str::to_owned),
        last_modified: reply.header("last-modified").map(str::to_owned),
        ..Policy::default()
    };
    // shortcut: `Expires` (an HTTP date) is not read, only `Cache-Control`; upgrade if a
    // server that matters sends only `Expires`.
    for d in reply.header("cache-control").unwrap_or_default().split(',') {
        let d = d.trim().to_ascii_lowercase();
        match d.split_once('=') {
            Some(("max-age", n)) => p.max_age = n.trim_matches('"').parse().ok().or(p.max_age),
            _ if d == "no-store" => p.store = false,
            _ if d == "no-cache" => p.max_age = Some(0),
            _ => {}
        }
    }
    p
}

// ---------- memory ----------

struct Kept {
    bytes: Arc<[u8]>,
    /// Stale after this second (`None`: for the session).
    expires: Option<u64>,
    used: u64,
}

#[derive(Default)]
struct Memory {
    entries: HashMap<String, Kept>,
    total: usize,
    tick: u64,
}

fn memory() -> std::sync::MutexGuard<'static, Memory> {
    static MEMORY: OnceLock<Mutex<Memory>> = OnceLock::new();
    MEMORY.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bytes at `url` if they are in memory and fresh.
pub(crate) fn remembered(url: &str) -> Option<Arc<[u8]>> {
    let mut m = memory();
    m.tick += 1;
    let tick = m.tick;
    let k = m.entries.get_mut(url)?;
    if k.expires.is_some_and(|e| e <= now()) {
        return None;
    }
    k.used = tick;
    Some(k.bytes.clone())
}

fn remember(url: &str, bytes: Arc<[u8]>, expires: Option<u64>) {
    if bytes.len() > MEMORY_LIMIT / 4 {
        return;
    }
    let mut m = memory();
    m.tick += 1;
    let used = m.tick;
    if let Some(old) = m.entries.insert(url.to_owned(), Kept { bytes: bytes.clone(), expires, used }) {
        m.total -= old.bytes.len();
    }
    m.total += bytes.len();
    while m.total > MEMORY_LIMIT {
        let Some(oldest) = m.entries.iter().min_by_key(|(_, k)| k.used).map(|(u, _)| u.clone()) else {
            break;
        };
        if let Some(k) = m.entries.remove(&oldest) {
            m.total -= k.bytes.len();
        }
    }
}

/// How many bytes of fetched images are in memory (for tests and the inspector).
pub fn remembered_bytes() -> usize {
    memory().total
}

/// Lets go of the fetched images kept in memory (when memory runs low); the disk keeps its.
pub fn forget_remembered() {
    let mut m = memory();
    m.entries.clear();
    m.total = 0;
}

/// How many addresses this thread is fetching (for tests and the inspector).
pub fn fetching() -> usize {
    FLIGHTS.with(|f| f.borrow().by_url.len())
}

// ---------- disk ----------

struct OnDisk {
    bytes: Arc<[u8]>,
    expires: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// FNV-1a: a name for the address's files that stays the same across runs and builds.
fn file_name(url: &str) -> String {
    let h =
        url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    format!("{h:016x}")
}

fn disk_dir() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    DISK.read().unwrap_or_else(PoisonError::into_inner).clone()
}

fn disk_read(url: &str) -> Option<OnDisk> {
    let dir = disk_dir()?;
    let name = file_name(url);
    let meta = std::fs::read_to_string(dir.join(format!("{name}.meta"))).ok()?;
    let mut lines = meta.lines();
    // Another address with the same name: not ours.
    if lines.next()? != url {
        return None;
    }
    let field = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_owned);
    let expires = lines.next()?.parse().ok();
    let (etag, last_modified) = (field(lines.next()), field(lines.next()));
    let path = dir.join(format!("{name}.bin"));
    let bytes = std::fs::read(&path).ok()?;
    // Used now: kept longest when the folder is pruned.
    if let Ok(f) = std::fs::File::options().append(true).open(&path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    Some(OnDisk { bytes: Arc::from(bytes), expires, etag, last_modified })
}

fn disk_write(url: &str, bytes: Option<&[u8]>, expires: Option<u64>, p: &Policy) {
    let Some(dir) = disk_dir() else { return };
    // Nothing to keep: neither fresh for a while nor checkable with the server.
    if expires.is_none_or(|e| e <= now()) && p.etag.is_none() && p.last_modified.is_none() {
        return;
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let name = file_name(url);
    let write = |ext: &str, data: &[u8]| {
        // Written whole, then renamed: a reader never sees half a file.
        let tmp = dir.join(format!("{name}.{ext}.tmp"));
        std::fs::write(&tmp, data).and_then(|()| std::fs::rename(&tmp, dir.join(format!("{name}.{ext}"))))
    };
    if let Some(b) = bytes
        && write("bin", b).is_err()
    {
        return;
    }
    let exp = expires.map(|e| e.to_string()).unwrap_or_default();
    let meta = format!(
        "{url}\n{exp}\n{}\n{}\n",
        p.etag.as_deref().unwrap_or_default(),
        p.last_modified.as_deref().unwrap_or_default()
    );
    let _ = write("meta", meta.as_bytes());
    if bytes.is_some() {
        prune(&dir);
    }
}

/// Deletes the least recently used files until the folder is within its limit.
fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut bins: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "bin"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = bins.iter().map(|b| b.1).sum();
    bins.sort();
    for (_, len, path) in bins {
        if total <= DISK_LIMIT {
            break;
        }
        let _ = std::fs::remove_file(path.with_extension("meta"));
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

// ---------- fetching ----------

type Got = Result<Arc<[u8]>, DecodeError>;

/// Finds the bytes at `url`, from disk or the server, and calls `done` with them (on any
/// thread). Do this off the UI thread.
fn resolve(url: String, cancelled: Arc<AtomicBool>, done: impl FnOnce(Got) + Send + 'static) {
    if !url.contains("://") && !cfg!(target_arch = "wasm32") {
        return done(Err(DecodeError::Network(format!("not an absolute address: {url}"))));
    }
    let disk = disk_read(&url);
    if let Some(d) = &disk
        && d.expires.is_some_and(|e| e > now())
    {
        remember(&url, d.bytes.clone(), d.expires);
        return done(Ok(d.bytes.clone()));
    }
    let request = FetchRequest {
        url: url.clone(),
        if_none_match: disk.as_ref().and_then(|d| d.etag.clone()),
        if_modified_since: disk.as_ref().and_then(|d| d.last_modified.clone()),
    };
    fetcher().fetch(request, cancelled, Box::new(move |reply| done(answer(&url, disk, reply))));
}

fn answer(url: &str, disk: Option<OnDisk>, reply: Result<FetchReply, String>) -> Got {
    let reply = reply.map_err(DecodeError::Network)?;
    let p = policy(&reply);
    let expires = p.max_age.map(|a| now() + a);
    let bytes: Arc<[u8]> = match (reply.status, disk) {
        // Unchanged: the copy on disk, fresh again.
        (304, Some(d)) => {
            disk_write(url, None, expires, &p);
            d.bytes
        }
        (200..=299, _) => {
            let bytes: Arc<[u8]> = Arc::from(reply.body);
            if p.store {
                disk_write(url, Some(&bytes), expires, &p);
            }
            bytes
        }
        (status, _) => return Err(DecodeError::Network(format!("the server answered {status}"))),
    };
    remember(url, bytes.clone(), expires);
    Ok(bytes)
}

/// The bytes at `url`, waiting for them (natively; in a page only what is in memory, since a
/// page cannot wait). Off the UI thread.
pub(crate) fn blocking(url: &str) -> Got {
    if let Some(b) = remembered(url) {
        return Ok(b);
    }
    if cfg!(target_arch = "wasm32") {
        return Err(DecodeError::Network(String::from(
            "not fetched yet (a page fetches only asynchronously)",
        )));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    resolve(url.to_owned(), Arc::default(), move |r| {
        let _ = tx.send(r);
    });
    rx.recv().unwrap_or_else(|_| Err(DecodeError::Network(String::from("the fetch ended without an answer"))))
}

type Waiter = Box<dyn FnOnce(Got)>;

struct Flight {
    generation: u64,
    waiters: Vec<(u64, Waiter)>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct Flights {
    next: u64,
    by_url: HashMap<String, Flight>,
}

thread_local! {
    static FLIGHTS: RefCell<Flights> = RefCell::default();
}

/// A wait for an address's bytes: dropping it withdraws it, and the fetch is cancelled when
/// nobody else waits for the same address.
pub(crate) struct Fetching {
    url: String,
    id: u64,
    detached: bool,
}

impl Fetching {
    /// Keeps waiting without anyone holding this.
    pub(crate) fn detach(mut self) {
        self.detached = true;
    }
}

impl Drop for Fetching {
    fn drop(&mut self) {
        if self.detached {
            return;
        }
        let gone = FLIGHTS.try_with(|f| {
            let mut f = f.borrow_mut();
            let empty = f.by_url.get_mut(&self.url).is_some_and(|fl| {
                fl.waiters.retain(|(w, _)| *w != self.id);
                fl.waiters.is_empty()
            });
            if empty { f.by_url.remove(&self.url) } else { None }
        });
        // Dropped outside the borrow: a waiter may own a `Fetching` of its own.
        if let Ok(Some(fl)) = gone {
            fl.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

/// Fetches the bytes at `url` (on the UI thread) and calls `done` with them there, unless the
/// returned wait is dropped first.
pub(crate) fn fetch(url: &str, done: impl FnOnce(Got) + 'static) -> Fetching {
    let (id, start) = FLIGHTS.with(|f| {
        let mut f = f.borrow_mut();
        f.next += 1;
        let id = f.next;
        if let Some(fl) = f.by_url.get_mut(url) {
            fl.waiters.push((id, Box::new(done)));
            return (id, None);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let flight =
            Flight { generation: id, waiters: vec![(id, Box::new(done))], cancelled: cancelled.clone() };
        f.by_url.insert(url.to_owned(), flight);
        (id, Some(cancelled))
    });
    let me = Fetching { url: url.to_owned(), id, detached: false };
    let Some(cancelled) = start else { return me };
    let url = url.to_owned();
    let Ok(waker) = remedy_ui_reactive::waker() else {
        // No runtime to answer through: wait here.
        let got = blocking(&url);
        land(&url, id, got);
        return me;
    };
    let c2 = cancelled.clone();
    crate::load::spawn_job(cancelled, move || {
        resolve(url.clone(), c2, move |got| {
            let _ = waker.post(move || land(&url, id, got));
        });
    });
    me
}

fn land(url: &str, generation: u64, got: Got) {
    let flight = FLIGHTS.with(|f| {
        let mut f = f.borrow_mut();
        if f.by_url.get(url).is_some_and(|fl| fl.generation == generation) {
            f.by_url.remove(url)
        } else {
            None
        }
    });
    for (_, w) in flight.map(|f| f.waiters).unwrap_or_default() {
        w(got.clone());
    }
}

// ---------- the engine's own fetcher ----------

struct Own;

/// Longest a fetch may take.
#[cfg(any(feature = "http", target_arch = "wasm32"))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[cfg(all(not(target_arch = "wasm32"), feature = "http"))]
mod native {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};

    use ureq::RequestExt;

    use super::{FetchDone, FetchReply, FetchRequest, TIMEOUT};

    /// Largest image body read.
    const MAX_BODY: usize = 128 * 1024 * 1024;

    fn agent() -> &'static ureq::Agent {
        static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
        AGENT.get_or_init(|| {
            // ring is the TLS provider; installing it twice is harmless.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let tls =
                ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
            ureq::Agent::config_builder()
                .tls_config(tls)
                .http_status_as_error(false)
                .timeout_global(Some(TIMEOUT))
                .user_agent("RemedyUI")
                .build()
                .into()
        })
    }

    fn run(req: &FetchRequest, cancelled: &AtomicBool) -> Result<FetchReply, String> {
        let lower = req.url.to_ascii_lowercase();
        if !lower.starts_with("https://") && !lower.starts_with("http://") {
            return Err(format!("not a web address: {}", req.url));
        }
        let mut b = ureq::http::Request::builder().uri(&req.url);
        if let Some(e) = &req.if_none_match {
            b = b.header("If-None-Match", e);
        }
        if let Some(m) = &req.if_modified_since {
            b = b.header("If-Modified-Since", m);
        }
        let mut res =
            b.body(()).map_err(|e| e.to_string())?.with_agent(agent()).run().map_err(|e| e.to_string())?;
        let status = res.status().as_u16();
        let headers = res
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect();
        let mut reader = res.body_mut().with_config().limit(MAX_BODY as u64).reader();
        let mut body = Vec::new();
        let mut chunk = vec![0; 64 * 1024];
        loop {
            // Let go mid-body (closing the connection) once nobody wants it.
            if cancelled.load(Ordering::Relaxed) {
                return Err(String::from("cancelled"));
            }
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(chunk.get(..n).unwrap_or_default()),
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(FetchReply { status, headers, body })
    }

    pub(super) fn fetch(request: FetchRequest, cancelled: Arc<AtomicBool>, done: FetchDone) {
        // Shared so a failed spawn can still answer.
        let slot = Arc::new(Mutex::new(Some(done)));
        let s2 = slot.clone();
        // A thread per fetch: a slow server holds no image worker.
        let spawned = std::thread::Builder::new().name("remedy-image-fetch".into()).spawn(move || {
            let out = run(&request, &cancelled);
            if let Some(done) = s2.lock().unwrap_or_else(PoisonError::into_inner).take() {
                done(out);
            }
        });
        if let Err(e) = spawned
            && let Some(done) = slot.lock().unwrap_or_else(PoisonError::into_inner).take()
        {
            done(Err(format!("no thread for the fetch: {e}")));
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use js_sys::{Array, Promise, Uint8Array};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    use super::{FetchDone, FetchReply, FetchRequest, TIMEOUT};

    async fn sleep(ms: f64) {
        let p = Promise::new(&mut |resolve, _| {
            let set = js_sys::Reflect::get(&js_sys::global(), &"setTimeout".into())
                .ok()
                .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
            if let Some(set) = set {
                let _ = set.call2(&JsValue::NULL, &resolve, &JsValue::from_f64(ms));
            }
        });
        let _ = JsFuture::from(p).await;
    }

    fn message(e: &JsValue) -> String {
        js_sys::Reflect::get(e, &"message".into())
            .ok()
            .and_then(|m| m.as_string())
            .or_else(|| e.as_string())
            .unwrap_or_else(|| format!("{e:?}"))
    }

    pub(super) fn fetch(request: FetchRequest, cancelled: Arc<AtomicBool>, done: FetchDone) {
        wasm_bindgen_futures::spawn_local(async move { done(run(request, cancelled).await) });
    }

    async fn run(req: FetchRequest, cancelled: Arc<AtomicBool>) -> Result<FetchReply, String> {
        let win = web_sys::window().ok_or("no browser window")?;
        let abort = web_sys::AbortController::new().map_err(|e| message(&e))?;
        let init = web_sys::RequestInit::new();
        init.set_signal(Some(&abort.signal()));
        // The browser's own cache revalidates; the page's cookies go only to its own origin.
        init.set_credentials(web_sys::RequestCredentials::SameOrigin);
        let finished = Rc::new(Cell::new(false));
        {
            let (finished, abort) = (finished.clone(), abort.clone());
            let (started, limit) = (js_sys::Date::now(), TIMEOUT.as_secs_f64() * 1000.0);
            wasm_bindgen_futures::spawn_local(async move {
                while !finished.get() {
                    if cancelled.load(Ordering::Relaxed) || js_sys::Date::now() - started > limit {
                        abort.abort();
                        return;
                    }
                    sleep(25.0).await;
                }
            });
        }
        let got = async {
            let resp: web_sys::Response = JsFuture::from(win.fetch_with_str_and_init(&req.url, &init))
                .await
                .map_err(|e| {
                    let origin = win.location().origin().unwrap_or_default();
                    let foreign = req.url.contains("://") && !req.url.starts_with(&format!("{origin}/"));
                    if foreign {
                        // The usual reason, which the browser does not tell the page.
                        format!(
                            "{} (an image from another site is shown only when its server allows it with CORS headers, or through the app's own proxy)",
                            message(&e)
                        )
                    } else {
                        message(&e)
                    }
                })?
                .unchecked_into();
            let mut headers = Vec::new();
            if let Ok(Some(entries)) = js_sys::try_iter(&resp.headers()) {
                for pair in entries.flatten() {
                    let pair: Array = pair.unchecked_into();
                    if let (Some(k), Some(v)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                        headers.push((k, v));
                    }
                }
            }
            let buf = JsFuture::from(resp.array_buffer().map_err(|e| message(&e))?).await.map_err(|e| message(&e))?;
            Ok(FetchReply { status: resp.status(), headers, body: Uint8Array::new(&buf).to_vec() })
        }
        .await;
        finished.set(true);
        got
    }
}

impl Fetch for Own {
    fn fetch(&self, request: FetchRequest, cancelled: Arc<AtomicBool>, done: FetchDone) {
        #[cfg(target_arch = "wasm32")]
        web::fetch(request, cancelled, done);
        #[cfg(all(not(target_arch = "wasm32"), feature = "http"))]
        native::fetch(request, cancelled, done);
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "http")))]
        {
            let _ = (request, cancelled);
            done(Err(String::from("built without the `http` feature: set a fetcher with set_fetcher")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(cc: &str) -> FetchReply {
        FetchReply::new(
            200,
            vec![("Cache-Control".into(), cc.into()), ("ETag".into(), "\"v1\"".into())],
            vec![],
        )
    }

    #[test]
    fn cache_control_is_read() {
        let p = policy(&reply("public, max-age=600"));
        assert_eq!((p.store, p.max_age, p.etag.as_deref()), (true, Some(600), Some("\"v1\"")));
        assert_eq!(policy(&reply("no-cache")).max_age, Some(0));
        assert!(!policy(&reply("no-store, max-age=5")).store);
        assert_eq!(policy(&reply("")).max_age, None);
    }

    #[test]
    fn memory_stays_within_its_limit() {
        let big: Arc<[u8]> = Arc::from(vec![0u8; MEMORY_LIMIT / 5]);
        for i in 0..8 {
            remember(&format!("test://mem/{i}"), big.clone(), None);
        }
        assert!(remembered_bytes() <= MEMORY_LIMIT);
        assert!(remembered("test://mem/7").is_some(), "the newest is kept");
        assert!(remembered("test://mem/0").is_none(), "the oldest went");
        remember("test://mem/stale", big, Some(1));
        assert!(remembered("test://mem/stale").is_none(), "expired");
    }

    #[test]
    fn names_are_stable() {
        assert_eq!(file_name(""), "cbf29ce484222325");
        assert_ne!(file_name("https://a/1"), file_name("https://a/2"));
    }
}
