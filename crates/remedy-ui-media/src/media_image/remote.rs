//! Images from addresses ([`Source::Url`](crate::media_image::Source::Url)): fetched through a [`Fetch`]
//! (the engine's own by default, or the app's), kept in a bounded memory cache and, natively,
//! on disk in the app's cache folder ([`set_cache_dir`]), following the server's
//! `Cache-Control` (`max-age`, `no-cache`, `no-store`) or `Expires`, and revalidating with
//! `ETag` and `Last-Modified`. How much is kept: [`set_memory_limit`], [`set_disk_limit`]. In a browser page the browser's own HTTP cache is the disk cache.
//!
//! Requests for the same address share one fetch; it is cancelled when nobody wants it any
//! more (every element showing it gone).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use remedy_ui_core::reactive::Cancel;

use crate::media_image::decode::DecodeError;

/// Fetches the bytes at an address. The engine has its own (`remedy-ui-services-net`:
/// natively HTTPS through ureq and rustls with the platform's certificates, in a page the
/// browser's `fetch`); an app sets another with [`set_fetcher`] (its own HTTP client, with
/// its cookies or auth headers).
pub trait Fetch: Send + Sync {
    /// Starts fetching `request` and calls `done` once with the server's reply (any status:
    /// a `304` answers a request with validators), from any thread. Once `cancel` is
    /// cancelled nobody waits for the reply any more: stop early, and `done` may be dropped
    /// uncalled.
    fn fetch(&self, request: FetchRequest, cancel: Cancel, done: FetchDone);
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

/// The default of [`set_memory_limit`]: 64 MiB.
pub const DEFAULT_MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// The default of [`set_disk_limit`]: 256 MiB.
pub const DEFAULT_DISK_LIMIT: u64 = 256 * 1024 * 1024;

static MEMORY_LIMIT: AtomicUsize = AtomicUsize::new(DEFAULT_MEMORY_LIMIT);
static DISK_LIMIT: AtomicU64 = AtomicU64::new(DEFAULT_DISK_LIMIT);

/// Keeps up to `bytes` of fetched (encoded) images in memory, the most recently used longest
/// (default [`DEFAULT_MEMORY_LIMIT`]). One image over a quarter of it is not kept in memory
/// (the disk still keeps it). Lowering it lets go of the least recently used at once.
pub fn set_memory_limit(bytes: usize) {
    MEMORY_LIMIT.store(bytes, Ordering::Relaxed);
    trim(&mut memory());
}

/// Keeps up to `bytes` of fetched images in the disk cache ([`set_cache_dir`]); the least
/// recently used files go first (default [`DEFAULT_DISK_LIMIT`]). Checked when an image is
/// written there.
pub fn set_disk_limit(bytes: u64) {
    DISK_LIMIT.store(bytes, Ordering::Relaxed);
}

fn memory_limit() -> usize {
    MEMORY_LIMIT.load(Ordering::Relaxed)
}

static FETCHER: RwLock<Option<Arc<dyn Fetch>>> = RwLock::new(None);
static DISK: RwLock<Option<PathBuf>> = RwLock::new(None);
static PROXY: RwLock<Option<String>> = RwLock::new(None);

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

/// In a browser page, fetches an image from another site whose server does not allow it (no
/// CORS headers, so the page may not read it) once more through `prefix` followed by its
/// percent-encoded address: a proxy on the app's own server, such as `/image-proxy?url=`
/// (`None`: no proxy, and such an image fails with [`DecodeError::Network`], showing the
/// element's fallback). Natively nothing needs it and it is not used.
pub fn set_web_proxy(prefix: Option<String>) {
    *PROXY.write().unwrap_or_else(PoisonError::into_inner) = prefix;
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
    let mut no_cache = false;
    for d in reply.header("cache-control").unwrap_or_default().split(',') {
        let d = d.trim().to_ascii_lowercase();
        match d.split_once('=') {
            Some(("max-age", n)) => p.max_age = n.trim_matches('"').parse().ok().or(p.max_age),
            _ if d == "no-store" => p.store = false,
            _ if d == "no-cache" => no_cache = true,
            _ => {}
        }
    }
    // RFC 9111 4.2.1: `max-age` wins; otherwise `Expires` less the server's `Date` (or now, if
    // it sent none). An `Expires` that is not a date (often "0") means already stale.
    if p.max_age.is_none()
        && let Some(e) = reply.header("expires")
    {
        let date = reply.header("date").and_then(http_date).unwrap_or_else(now);
        p.max_age = Some(http_date(e).map_or(0, |e| e.saturating_sub(date)));
    }
    if no_cache {
        p.max_age = Some(0);
    }
    p
}

/// Seconds since 1970 of an HTTP date (RFC 9110 5.6.7): IMF-fixdate
/// (`Sun, 06 Nov 1994 08:49:37 GMT`), and the obsolete RFC 850 (`Sunday, 06-Nov-94 08:49:37
/// GMT`) and asctime (`Sun Nov  6 08:49:37 1994`) forms every recipient must still read.
fn http_date(s: &str) -> Option<u64> {
    const MONTHS: [&str; 12] =
        ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let month = |m: &str| MONTHS.iter().position(|x| m.eq_ignore_ascii_case(x)).map(|i| i as u64 + 1);
    let s = s.trim();
    // The weekday is redundant: skip it.
    let rest = s.split_once([',', ' ']).map(|(_, r)| r.trim_start())?;
    let words: Vec<&str> = rest.split_whitespace().collect();
    let (day, mon, year, time): (u64, u64, u64, &str) = match words.as_slice() {
        [d, m, y, t, "GMT"] => (d.parse().ok()?, month(m)?, y.parse::<u64>().ok()?, *t),
        [dmy, t, "GMT"] => {
            let mut p = dmy.split('-');
            let (d, m, y) = (p.next()?, p.next()?, p.next()?);
            let y: u64 = y.parse().ok()?;
            // RFC 9110: a two-digit year more than 50 years ahead is in the past century.
            let y = if y >= 100 {
                y
            } else if y >= 70 {
                1900 + y
            } else {
                2000 + y
            };
            (d.parse().ok()?, month(m)?, y, *t)
        }
        [m, d, t, y] => (d.parse().ok()?, month(m)?, y.parse().ok()?, *t),
        _ => return None,
    };
    let mut t = time.split(':').map(str::parse::<u64>);
    let (h, mi, sec) = (t.next()?.ok()?, t.next()?.ok()?, t.next()?.ok()?);
    if t.next().is_some() || !(1..=31).contains(&day) || year < 1970 || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from 1970-01-01 to the date (Howard Hinnant's days_from_civil).
    let (y, m) = if mon <= 2 { (year - 1, mon + 9) } else { (year, mon - 3) };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146_097 + doe).checked_sub(719_468)?;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec)
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
    if bytes.len() > memory_limit() / 4 {
        return;
    }
    let mut m = memory();
    m.tick += 1;
    let used = m.tick;
    if let Some(old) = m.entries.insert(url.to_owned(), Kept { bytes: bytes.clone(), expires, used }) {
        m.total -= old.bytes.len();
    }
    m.total += bytes.len();
    trim(&mut m);
}

/// Lets go of the least recently used until memory is within its limit.
fn trim(m: &mut Memory) {
    while m.total > memory_limit() {
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
        if total <= DISK_LIMIT.load(Ordering::Relaxed) {
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
fn resolve(url: String, cancel: Cancel, done: impl FnOnce(Got) + Send + 'static) {
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
    let c2 = cancel.clone();
    fetcher().fetch(
        request,
        cancel,
        Box::new(move |reply| {
            let why = match reply {
                Err(why) => why,
                ok => return done(answer(&url, disk, ok)),
            };
            let Some(via) = proxied(&url) else { return done(answer(&url, disk, Err(hint(&url, why)))) };
            // Refused across sites (in a page): once more through the app's own proxy.
            fetcher().fetch(
                FetchRequest { url: via, ..FetchRequest::default() },
                c2,
                Box::new(move |r| {
                    done(answer(&url, None, r.map_err(|e| format!("{why}; through the proxy: {e}"))))
                }),
            );
        }),
    );
}

/// In a page, what a failed fetch of an address on another site most likely means (the
/// browser does not tell the page).
fn hint(url: &str, why: String) -> String {
    if cfg!(target_arch = "wasm32") && url.contains("://") {
        format!(
            "{why} (an image from another site is shown in a page only when its server allows it with CORS headers, or through the app's own proxy: set_web_proxy)"
        )
    } else {
        why
    }
}

/// `url` through the proxy [`set_web_proxy`] names, in a page, for an absolute address.
fn proxied(url: &str) -> Option<String> {
    if !cfg!(target_arch = "wasm32") || !url.contains("://") {
        return None;
    }
    let prefix = PROXY.read().unwrap_or_else(PoisonError::into_inner).clone()?;
    Some(format!("{prefix}{}", encode(url)))
}

/// Percent-encodes everything but the unreserved characters, for a query value.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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
    resolve(url.to_owned(), Cancel::new(), move |r| {
        let _ = tx.send(r);
    });
    rx.recv().unwrap_or_else(|_| Err(DecodeError::Network(String::from("the fetch ended without an answer"))))
}

type Waiter = Box<dyn FnOnce(Got)>;

struct Flight {
    generation: u64,
    waiters: Vec<(u64, Waiter)>,
    cancel: Cancel,
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
            fl.cancel.cancel();
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
        let cancel = Cancel::new();
        let flight = Flight { generation: id, waiters: vec![(id, Box::new(done))], cancel: cancel.clone() };
        f.by_url.insert(url.to_owned(), flight);
        (id, Some(cancel))
    });
    let me = Fetching { url: url.to_owned(), id, detached: false };
    let Some(cancel) = start else { return me };
    let url = url.to_owned();
    let Ok(waker) = remedy_ui_core::reactive::waker() else {
        // No runtime to answer through: wait here.
        let got = blocking(&url);
        land(&url, id, got);
        return me;
    };
    // On a worker: the disk is read there; the fetch itself does not wait.
    crate::media_image::load::spawn_job(Arc::default(), move || {
        resolve(url.clone(), cancel, move |got| {
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

/// The engine's HTTP client (`remedy-ui-services-net`): natively ureq with rustls, refusing
/// plain `http` beyond the loopback interface; in a page the browser's `fetch`.
struct Own;

impl Fetch for Own {
    fn fetch(&self, request: FetchRequest, cancel: Cancel, done: FetchDone) {
        #[cfg(feature = "media-image-http")]
        {
            use remedy_ui_platform::services_net::{Http, NetError, Request};
            let mut r = Request::get(request.url);
            if let Some(e) = request.if_none_match {
                r = r.header("If-None-Match", e);
            }
            if let Some(m) = request.if_modified_since {
                r = r.header("If-Modified-Since", m);
            }
            Http::new("").send(r, &cancel, move |out| {
                done(match out {
                    Ok(res) => Ok(FetchReply { status: res.status, headers: res.headers, body: res.body }),
                    // A `304` brings the copy's new validators and freshness in its headers.
                    Err(NetError::Http(e)) => Ok(FetchReply::new(e.status, e.headers, e.body.into_bytes())),
                    Err(e) => Err(e.to_string()),
                });
            });
        }
        #[cfg(not(feature = "media-image-http"))]
        {
            let _ = (request, cancel);
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

    fn headers(h: &[(&str, &str)]) -> FetchReply {
        FetchReply::new(200, h.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(), vec![])
    }

    #[test]
    fn http_dates_are_read_in_all_three_forms() {
        let t = Some(784_111_777);
        assert_eq!(http_date("Sun, 06 Nov 1994 08:49:37 GMT"), t);
        assert_eq!(http_date("Sunday, 06-Nov-94 08:49:37 GMT"), t);
        assert_eq!(http_date("Sun Nov  6 08:49:37 1994"), t);
        assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(http_date("Tue, 29 Feb 2028 12:00:00 GMT"), Some(1_835_438_400));
        assert_eq!(http_date("0"), None);
        assert_eq!(http_date("Sun, 06 Nov 1994 08:49:37 PST"), None);
    }

    #[test]
    fn expires_is_read_when_there_is_no_max_age() {
        let date = ("Date", "Sun, 06 Nov 1994 08:49:37 GMT");
        let p = policy(&headers(&[date, ("Expires", "Sun, 06 Nov 1994 09:49:37 GMT")]));
        assert_eq!(p.max_age, Some(3600));
        // max-age wins over Expires.
        let p = policy(&headers(&[
            date,
            ("Expires", "Sun, 06 Nov 1994 09:49:37 GMT"),
            ("Cache-Control", "max-age=5"),
        ]));
        assert_eq!(p.max_age, Some(5));
        // Already past, or not a date: stale at once.
        assert_eq!(policy(&headers(&[date, ("Expires", "Sat, 05 Nov 1994 00:00:00 GMT")])).max_age, Some(0));
        assert_eq!(policy(&headers(&[("Expires", "0")])).max_age, Some(0));
        assert_eq!(
            policy(&headers(&[("Expires", "Sun, 06 Nov 1994 09:49:37 GMT"), ("Cache-Control", "no-cache")]))
                .max_age,
            Some(0)
        );
    }

    #[test]
    fn memory_stays_within_its_limit() {
        let big: Arc<[u8]> = Arc::from(vec![0u8; DEFAULT_MEMORY_LIMIT / 5]);
        for i in 0..8 {
            remember(&format!("test://mem/{i}"), big.clone(), None);
        }
        assert!(remembered_bytes() <= DEFAULT_MEMORY_LIMIT);
        assert!(remembered("test://mem/7").is_some(), "the newest is kept");
        assert!(remembered("test://mem/0").is_none(), "the oldest went");
        remember("test://mem/stale", big.clone(), Some(1));
        assert!(remembered("test://mem/stale").is_none(), "expired");
        // A lower limit lets go at once, and a higher one keeps more.
        set_memory_limit(DEFAULT_MEMORY_LIMIT / 4);
        assert!(remembered_bytes() <= DEFAULT_MEMORY_LIMIT / 4);
        set_memory_limit(DEFAULT_MEMORY_LIMIT * 4);
        for i in 0..8 {
            remember(&format!("test://mem/{i}"), big.clone(), None);
        }
        assert!(remembered("test://mem/0").is_some(), "kept under the higher limit");
        set_memory_limit(DEFAULT_MEMORY_LIMIT);
        assert!(remembered_bytes() <= DEFAULT_MEMORY_LIMIT);
    }

    #[test]
    fn names_are_stable() {
        assert_eq!(file_name(""), "cbf29ce484222325");
        assert_ne!(file_name("https://a/1"), file_name("https://a/2"));
    }
}
