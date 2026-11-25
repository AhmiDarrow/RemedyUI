//! Requests natively: ureq with rustls on ring and the platform's certificate verifier, the
//! same stack as the updater. Each request runs on a thread of its own, so a caller that
//! cancels returns at once; the request stops reading its body (closing the connection) at
//! the next chunk, or ends within its timeout while it waits for the server.
//!
//! Redirects are followed here rather than by ureq, so each hop is checked against the
//! address policy and gets the cookies of its own address. Plain `http` never goes through a
//! proxy, and the addresses its host name resolves to are checked before connecting ([`Guard`]).

use std::io::Read;
use std::sync::mpsc;
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use remedy_ui_core::reactive::Cancel;
use ureq::RequestExt;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use super::{HttpError, HttpPolicy, NetError, Prepared, Response, absolute, allowed, reaches};

/// How often a waiting caller looks at its cancel token.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// Resolves names as usual, then keeps only the addresses plain `http` may reach under the
/// policy: the check holds for the address actually connected to, so a name that resolves
/// somewhere else than it looked (DNS rebinding) is refused.
#[derive(Debug)]
struct Guard(HttpPolicy);

/// Why [`Guard`] refused, told apart from other connection errors.
#[derive(Debug)]
struct Unreachable(String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

impl Resolver for Guard {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = DefaultResolver::default().resolve(uri, config, timeout)?;
        // A proxy's own address (only `https` goes through one) is not the app's to limit.
        if uri.scheme_str() != Some("http") || config.proxy().is_some() {
            return Ok(all);
        }
        let mut kept = self.empty();
        for a in all.iter().filter(|a| reaches(a.ip(), self.0)) {
            kept.push(*a);
        }
        if kept.is_empty() {
            let host = uri.host().unwrap_or_default();
            let ips: Vec<String> = all.iter().map(|a| a.ip().to_string()).collect();
            return Err(ureq::Error::Other(Box::new(Unreachable(format!(
                "{host} resolves to {}, which plain http does not reach under {:?}",
                ips.join(", "),
                self.0
            )))));
        }
        Ok(kept)
    }
}

/// How the native client keeps connections open between requests to reuse them
/// ([`set_pool_limits`]). The defaults are ureq's: 10 idle connections, 3 to one host and
/// port, each kept 15 seconds. More idle connections save handshakes for an app that talks
/// to many hosts or in bursts, at an open socket (and a server's slot) each; 0 connections
/// opens a new one for every request.
///
/// ```
/// use std::time::Duration;
/// use remedy_ui_platform::services_net::{PoolLimits, set_pool_limits};
/// set_pool_limits(PoolLimits::default().idle_per_host(8).idle_age(Duration::from_secs(60)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolLimits {
    /// The most idle connections kept in all. Default 10.
    pub idle_connections: usize,
    /// The most idle connections kept to one host and port. Default 3.
    pub idle_per_host: usize,
    /// How long an idle connection is kept. Default 15 seconds.
    pub idle_age: Duration,
}

impl PoolLimits {
    /// The defaults, as a constant.
    pub const DEFAULT: PoolLimits =
        PoolLimits { idle_connections: 10, idle_per_host: 3, idle_age: Duration::from_secs(15) };

    /// Sets [`idle_connections`](Self::idle_connections).
    pub fn idle_connections(mut self, value: usize) -> Self {
        self.idle_connections = value;
        self
    }

    /// Sets [`idle_per_host`](Self::idle_per_host).
    pub fn idle_per_host(mut self, value: usize) -> Self {
        self.idle_per_host = value;
        self
    }

    /// Sets [`idle_age`](Self::idle_age).
    pub fn idle_age(mut self, value: Duration) -> Self {
        self.idle_age = value;
        self
    }
}

impl Default for PoolLimits {
    fn default() -> Self {
        PoolLimits::DEFAULT
    }
}

/// The agents requests go through, with the pool limits they were built with (one per
/// policy: loopback only, private networks).
type Built = Option<(PoolLimits, ureq::Agent)>;
static AGENTS: Mutex<(Built, Built)> = Mutex::new((None, None));
static POOL: Mutex<PoolLimits> = Mutex::new(PoolLimits::DEFAULT);

/// Sets how the native client keeps connections for reuse, from the next request on;
/// connections idle in the pool before are closed. Not used in browsers, which pool for the
/// page.
pub fn set_pool_limits(limits: PoolLimits) {
    *POOL.lock().unwrap_or_else(PoisonError::into_inner) = limits;
}

/// How the native client keeps connections now ([`set_pool_limits`]).
pub fn pool_limits() -> PoolLimits {
    *POOL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn agent(policy: HttpPolicy) -> ureq::Agent {
    let pool = pool_limits();
    let mut agents = AGENTS.lock().unwrap_or_else(PoisonError::into_inner);
    let (loopback, private) = &mut *agents;
    let slot = if policy == HttpPolicy::PrivateNetworks { private } else { loopback };
    if let Some((built, agent)) = slot.as_ref()
        && *built == pool
    {
        return agent.clone();
    }
    let agent = {
        // ring is the TLS provider; installing it twice is harmless.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            // `allowed` and `Guard` decide which plain-http addresses are reached.
            .https_only(false)
            // Followed in `run`, hop by hop.
            .max_redirects(0)
            .user_agent("RemedyUI")
            .max_idle_connections(pool.idle_connections)
            .max_idle_connections_per_host(pool.idle_per_host)
            .max_idle_age(pool.idle_age)
            .build();
        ureq::Agent::with_parts(config, DefaultConnector::default(), Guard(policy))
    };
    *slot = Some((pool, agent.clone()));
    agent
}

fn net(e: ureq::Error) -> NetError {
    match e {
        ureq::Error::Timeout(_) => NetError::Timeout,
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::TimedOut => NetError::Timeout,
        ureq::Error::Other(e) if e.is::<Unreachable>() => NetError::Refused(e.to_string()),
        e => NetError::Network(e.to_string()),
    }
}

/// `location` (absolute, scheme-relative, absolute-path or relative) against `base`.
fn join(base: &str, location: &str) -> String {
    if absolute(location) {
        return location.to_owned();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    if let Some(l) = location.strip_prefix("//") {
        return format!("{scheme}://{l}");
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let tail = tail.split('#').next().unwrap_or_default();
    let (path, query) = tail.split_once('?').map_or((tail, None), |(p, q)| (p, Some(q)));
    let (to, extra) = location.find(['?', '#']).map_or((location, ""), |i| location.split_at(i));
    // RFC 3986 5.2.2: only a query or a fragment keeps the base's path (and, without a
    // query of its own, the base's query).
    if to.is_empty() {
        let path = if path.is_empty() { "/" } else { path };
        return match query.filter(|_| !extra.starts_with('?')) {
            Some(q) => format!("{scheme}://{authority}{path}?{q}{extra}"),
            None => format!("{scheme}://{authority}{path}{extra}"),
        };
    }
    let merged = if to.starts_with('/') {
        to.to_owned()
    } else {
        format!("{}{to}", path.rfind('/').and_then(|i| path.get(..=i)).unwrap_or("/"))
    };
    format!("{scheme}://{authority}{}{extra}", remove_dots(&merged))
}

/// RFC 3986 5.2.4: an absolute `path` without its `.` and `..` segments (written out or
/// percent-encoded), so cookies and the server see the path the redirect means.
fn remove_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut segments = path.split('/').skip(1).peekable();
    while let Some(s) = segments.next() {
        let last = segments.peek().is_none();
        match s.replace("%2e", ".").replace("%2E", ".").as_str() {
            "." => {}
            ".." => {
                out.pop();
            }
            _ => {
                out.push(s);
                continue;
            }
        }
        // A dot segment at the end leaves a directory.
        if last {
            out.push("");
        }
    }
    format!("/{}", out.join("/"))
}

/// The scheme and the host with its port of an address, to tell when a redirect leaves the
/// origin (a default port written out counts as another, which only withholds more).
fn origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

/// Runs the request where it is called; once `stop` is cancelled it lets go of the body.
fn run(p: Prepared, stop: Option<&Cancel>) -> Result<Response, NetError> {
    // `None`: no limit (or one too far off to count).
    let deadline = p.timeout.and_then(|t| Instant::now().checked_add(t));
    let (mut method, mut url, mut headers, mut body) =
        (p.method, p.url.clone(), p.headers.clone(), p.body.clone());
    let mut hops = 0;
    let mut res = loop {
        allowed(&url, p.policy)?;
        let mut b = ureq::http::Request::builder().method(method).uri(&url);
        // One `Cookie` header (RFC 6265 5.4): the app's own cookies, then the jar's.
        let mut cookie: Vec<String> = Vec::new();
        for (k, v) in &headers {
            if k.eq_ignore_ascii_case("cookie") {
                cookie.push(v.clone());
            } else {
                b = b.header(k.as_str(), v.as_str());
            }
        }
        cookie.extend(p.cookies.as_ref().and_then(|j| j.header(&url)));
        if !cookie.is_empty() {
            b = b.header("Cookie", cookie.join("; "));
        }
        let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        let plain = url.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://"));
        // An empty POST or PUT says `Content-Length: 0` rather than going chunked.
        let send = body.clone().or_else(|| matches!(method, "POST" | "PUT").then(Vec::new));
        macro_rules! go {
            ($req:expr) => {{
                let c = $req
                    .map_err(|e| NetError::Refused(e.to_string()))?
                    .with_agent(agent(p.policy))
                    .configure();
                // Plain http goes straight to the address `Guard` checked, never via a proxy.
                let c = if plain { c.proxy(None) } else { c };
                c.timeout_global(left).build().run()
            }};
        }
        let res = match send {
            Some(bytes) => go!(b.body(bytes)),
            None => go!(b.body(())),
        };
        let res = res.map_err(net)?;
        if let Some(jar) = &p.cookies {
            for v in res.headers().get_all("set-cookie") {
                if let Ok(v) = v.to_str() {
                    jar.store(&url, v);
                }
            }
        }
        let status = res.status().as_u16();
        let location = res.headers().get("location").and_then(|l| l.to_str().ok()).map(str::to_owned);
        let Some(location) = location.filter(|_| matches!(status, 301 | 302 | 303 | 307 | 308)) else {
            break res;
        };
        hops += 1;
        if p.max_redirects == 0 {
            break res;
        }
        if hops > p.max_redirects {
            return Err(NetError::Network(format!("more than {} redirects", p.max_redirects)));
        }
        let next = join(&url, &location);
        // As browsers do: a 303, or a 301 or 302 of a POST, becomes a GET without the body.
        if status == 303 || (matches!(status, 301 | 302) && method == "POST") {
            method = if method == "HEAD" { "HEAD" } else { "GET" };
            body = None;
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
        }
        // Credentials the app set for one origin are not handed to another.
        if origin(&next) != origin(&url) {
            let secret = ["authorization", "proxy-authorization", "cookie"];
            headers.retain(|(k, _)| !secret.iter().any(|h| k.eq_ignore_ascii_case(h)));
        }
        url = next;
    };
    let status = res.status().as_u16();
    let headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    let ok = (200..300).contains(&status);
    // The answer as a streamed chunk sees it: everything but the body.
    let head = Response { status, headers, body: Vec::new() };
    let chunks = p.chunks.as_ref().filter(|_| ok);
    let mut reader =
        res.body_mut().with_config().limit(p.limit.map_or(u64::MAX, |l| l.saturating_add(1))).reader();
    let (mut body, mut chunk, mut total) = (Vec::new(), vec![0; 64 * 1024], 0u64);
    loop {
        // Nobody wants it any more: dropping the reader closes the connection.
        if stop.is_some_and(Cancel::is_cancelled) {
            return Err(NetError::Cancelled);
        }
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            // A body that stalls past the deadline reads as a broken connection.
            Err(_) if deadline.is_some_and(|d| Instant::now() >= d) => return Err(NetError::Timeout),
            Err(e) => return Err(net(ureq::Error::Io(e))),
        };
        total += n as u64;
        if let Some(limit) = p.limit.filter(|l| total > *l) {
            return Err(NetError::TooLarge(limit));
        }
        let bytes = chunk.get(..n).unwrap_or_default();
        match chunks {
            Some(c) => match c.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                Some(f) => f(&head, bytes),
                // The caller has had its answer (cancelled, timed out).
                None => return Err(NetError::Cancelled),
            },
            None => body.extend_from_slice(bytes),
        }
    }
    if ok {
        Ok(Response { body, ..head })
    } else {
        Err(NetError::Http(HttpError::new(status, head.headers, &body)))
    }
}

/// Keepalive requests still running, for [`finish_keepalive`].
fn in_flight() -> &'static (Mutex<usize>, Condvar) {
    static IN_FLIGHT: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// Told when keepalive requests start being under way (`true`) and when the last ends.
static HOOK: Mutex<Option<fn(bool)>> = Mutex::new(None);

/// Calls `hook(true)` when a keepalive request starts while none is under way, and
/// `hook(false)` when the last one ends: for a platform that must be told to keep the app
/// running out of sight. `remedy_ui::run` sets it on Android, where it starts a service that
/// keeps the process from being frozen in the background until the requests end, as a
/// browser lets a closing page's keepalive `fetch` finish.
pub fn on_keepalive(hook: fn(bool)) {
    *HOOK.lock().unwrap_or_else(PoisonError::into_inner) = Some(hook);
}

fn tell(running: bool) {
    let hook = *HOOK.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(h) = hook {
        h(running);
    }
}

/// Counts a keepalive request until it ends. The hook is told under the count's lock, so
/// its calls keep the order of the changes they report.
struct Kept;

impl Kept {
    fn new() -> Self {
        let mut n = in_flight().0.lock().unwrap_or_else(PoisonError::into_inner);
        *n += 1;
        if *n == 1 {
            tell(true);
        }
        Kept
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        let (n, cv) = in_flight();
        let mut n = n.lock().unwrap_or_else(PoisonError::into_inner);
        *n = n.saturating_sub(1);
        if *n == 0 {
            tell(false);
        }
        cv.notify_all();
    }
}

/// Waits up to `max` for the keepalive requests still under way (an app's last saves) to
/// end; true when none is left. `remedy_ui::run` calls this as the app quits, so a
/// keepalive request finishes as it would in a closing browser page.
pub fn finish_keepalive(max: Duration) -> bool {
    let (n, cv) = in_flight();
    let n = n.lock().unwrap_or_else(PoisonError::into_inner);
    let (n, _) = cv.wait_timeout_while(n, max, |n| *n > 0).unwrap_or_else(PoisonError::into_inner);
    *n == 0
}

pub(super) fn send_blocking(p: Prepared, cancel: &Cancel) -> Result<Response, NetError> {
    let keepalive = p.keepalive;
    if !keepalive && cancel.is_cancelled() {
        return Err(NetError::Cancelled);
    }
    let kept = keepalive.then(Kept::new);
    // ureq's own deadline normally ends it first; this one holds even when a socket call
    // overruns it, so the caller's timeout is kept.
    let deadline = p.timeout.and_then(|t| Instant::now().checked_add(t.saturating_add(CANCEL_POLL)));
    let chunks = p.chunks.clone();
    // Answering early: no chunk may follow the answer.
    let early = |e: NetError| {
        if let Some(c) = &chunks {
            c.lock().unwrap_or_else(PoisonError::into_inner).take();
        }
        Err(e)
    };
    let (tx, rx) = mpsc::channel();
    let stop = (!keepalive).then(|| cancel.clone());
    std::thread::Builder::new()
        .name("remedy-net".into())
        .spawn(move || {
            let _kept = kept;
            let _ = tx.send(run(p, stop.as_ref()));
        })
        .map_err(|e| NetError::Network(format!("no thread for the request: {e}")))?;
    loop {
        match rx.recv_timeout(CANCEL_POLL) {
            Ok(out) => return out,
            Err(mpsc::RecvTimeoutError::Timeout) if !keepalive && cancel.is_cancelled() => {
                return early(NetError::Cancelled);
            }
            Err(mpsc::RecvTimeoutError::Timeout) if deadline.is_some_and(|d| Instant::now() > d) => {
                return early(NetError::Timeout);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return early(NetError::Network("the request ended without an answer".into()));
            }
        }
    }
}

pub(super) fn send(
    p: Prepared,
    cancel: Cancel,
    done: impl FnOnce(Result<Response, NetError>) + Send + 'static,
) {
    // Shared so a failed spawn can still answer.
    let done = std::sync::Arc::new(Mutex::new(Some(done)));
    let answer = done.clone();
    // Counted from the call, so a quit right after it still waits.
    let kept = p.keepalive.then(Kept::new);
    let spawned = std::thread::Builder::new().name("remedy-net".into()).spawn(move || {
        let _kept = kept;
        let out = send_blocking(p, &cancel);
        if let Some(done) = answer.lock().unwrap_or_else(PoisonError::into_inner).take() {
            done(out);
        }
    });
    if let Err(e) = spawned
        && let Some(done) = done.lock().unwrap_or_else(PoisonError::into_inner).take()
    {
        done(Err(NetError::Network(format!("no thread for the request: {e}"))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check is on the resolved address, whatever the name looked like.
    #[test]
    fn the_resolved_address_is_checked() {
        let resolve = |policy, url: &str| {
            let uri: ureq::http::Uri = url.parse().unwrap_or_else(|_| unreachable!());
            let config = ureq::Agent::config_builder().proxy(None).build();
            Guard(policy).resolve(&uri, &config, NextTimeout::default())
        };
        let lo = HttpPolicy::LoopbackOnly;
        assert!(resolve(lo, "http://127.0.0.1:80/").is_ok());
        let refused = resolve(lo, "http://192.168.1.1/").map(|_| ()).map_err(net);
        assert!(matches!(refused, Err(NetError::Refused(_))), "{refused:?}");
        assert!(resolve(HttpPolicy::PrivateNetworks, "http://192.168.1.1/").is_ok());
        assert!(resolve(HttpPolicy::PrivateNetworks, "http://8.8.8.8/").is_err());
        assert!(resolve(HttpPolicy::PrivateNetworks, "http://[fd00::5]/").is_ok());
        // https goes anywhere.
        assert!(resolve(lo, "https://8.8.8.8/").is_ok());
    }

    #[test]
    fn joins_redirects() {
        assert_eq!(join("https://a.test/x/y?q", "https://b.test/z"), "https://b.test/z");
        assert_eq!(join("https://a.test/x/y?q", "//b.test/z"), "https://b.test/z");
        assert_eq!(join("http://a.test:8/x/y?q", "/z?1"), "http://a.test:8/z?1");
        assert_eq!(join("https://a.test/x/y?q", "z"), "https://a.test/x/z");
        assert_eq!(join("https://a.test", "z"), "https://a.test/z");
        // Dot segments are resolved, and never climb above the root.
        assert_eq!(join("https://a.test/api/v1/x?q", "../other?y#f"), "https://a.test/api/other?y#f");
        assert_eq!(join("https://a.test/api/x", "/a/./b/%2E%2e/c/d/.."), "https://a.test/a/c/");
        assert_eq!(join("https://a.test/api/x", "../../../z/."), "https://a.test/z/");
        assert_eq!(join("https://a.test/a/b", "/"), "https://a.test/");
        assert_eq!(join("https://a.test/a/b/", "c/"), "https://a.test/a/b/c/");
        // A query or a fragment alone stays on the page.
        assert_eq!(join("https://a.test/a/b?q#f", "?p=1"), "https://a.test/a/b?p=1");
        assert_eq!(join("https://a.test/a/b?q#f", "#top"), "https://a.test/a/b?q#top");
        assert_eq!(join("https://a.test?q", ""), "https://a.test/?q");
        // An address in the query of a relative redirect is not where it goes.
        assert_eq!(
            join("https://a.test/x", "/in?to=https://b.test/"),
            "https://a.test/in?to=https://b.test/"
        );
    }

    #[test]
    fn another_port_is_another_origin() {
        assert_eq!(origin("http://u:p@LocalHost:8000/x"), origin("HTTP://localhost:8000?y"));
        assert_ne!(origin("http://localhost:8000/"), origin("http://localhost:9000/"));
        assert_ne!(origin("http://a.test/"), origin("https://a.test/"));
    }
}
