//! RemedyUI HTTP client: the same calls in a native app and in a browser page.
//!
//! - [`Http`]: a client for one base address, with default headers, a timeout and (natively)
//!   an optional [`CookieJar`]. In a browser the page's own cookies go with requests to its
//!   origin (`credentials: same-origin`), as `fetch` does by default.
//! - [`Request`]: `GET`, `POST`, `PUT`, `PATCH` or `DELETE` a path, with headers, a JSON, raw or
//!   [`Multipart`] body, its own timeout, and [`Request::keepalive`] for a request that
//!   must finish while the page or app closes (a last save): the browser's `fetch`
//!   `keepalive`, or natively a request the app waits for on its way out
//!   ([`finish_keepalive`], which `remedy_ui::run` calls).
//! - [`Http::send`] starts a request and calls back with the [`Response`] or a
//!   [`NetError`]; it never blocks, so it runs in a browser page. A status outside 200-299
//!   is an [`HttpError`] carrying the status, the body and the `detail` field of a JSON
//!   body (`{"detail": "…"}`, as FastAPI and many others answer). [`Http::send_json`]
//!   decodes the body into your type.
//! - [`Http::resource`]: a reactive [`Resource`] that fetches JSON for a key; a new key
//!   aborts the request under way (the browser's `AbortController`, or natively the request
//!   is let go and its answer dropped).
//! - [`Http::stream`] hands a long body over chunk by chunk, cancellable, within a size limit.
//! - Natively [`Http::send_blocking`] also waits for the answer, for work already on a
//!   background thread.
//! - Natively a [`CookieJar`] keeps cookies as RFC 6265 says; redirects are followed hop by
//!   hop, each hop checked and sent its own cookies.
//! - Every limit has a default and can be set or lifted: [`Http::timeout`] and
//!   [`Request::timeout`] ([`DEFAULT_TIMEOUT`]), [`Http::max_body`] and [`Request::max_body`]
//!   ([`DEFAULT_MAX_BODY`]), [`Http::max_redirects`] ([`DEFAULT_MAX_REDIRECTS`]),
//!   [`CookieJar::max_cookies`] ([`DEFAULT_MAX_COOKIES`]), [`CookieJar::max_per_host`]
//!   ([`DEFAULT_MAX_COOKIES_PER_HOST`]).
//!
//! **Policy.** This pack is opt-in and contacts only addresses the app names. Natively it
//! refuses plain `http` beyond the loopback interface unless the client allows the local
//! network ([`HttpPolicy::PrivateNetworks`]), checking the address a name resolves to (TLS is
//! rustls with the platform's certificate verifier); in a browser a relative path goes to the
//! page's own origin and the browser's own rules (mixed content, CORS) apply on top. Nothing is logged, and no header
//! is added beyond what the app sets and `User-Agent` natively.
//!
//! ```no_run
//! use remedy_ui_platform::services_net::{Http, Request};
//! use remedy_ui_core::reactive::Cancel;
//!
//! #[derive(serde::Deserialize)]
//! struct Saved {
//!     id: u32,
//! }
//!
//! let http = Http::new("https://example.com/api");
//! http.send_json::<Saved>(
//!     Request::post("notes").json(&serde_json::json!({ "text": "hi" })),
//!     &Cancel::new(),
//!     |out| match out {
//!         Ok(saved) => println!("saved as {}", saved.id),
//!         Err(e) => println!("not saved: {e}"),
//!     },
//! );
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::reactive::{Cancel, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;

mod cookies;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

pub use cookies::{Cookie, CookieJar, DEFAULT_MAX_COOKIES, DEFAULT_MAX_COOKIES_PER_HOST, SameSite};
#[cfg(not(target_arch = "wasm32"))]
pub use native::{PoolLimits, finish_keepalive, on_keepalive, pool_limits, set_pool_limits};

/// The answer to a request that succeeded (status 200-299).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The response headers, names in lowercase.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// The first header named `name` (any case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The body as text (invalid UTF-8 replaced).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body decoded from JSON. An empty body decodes as JSON `null`, so `()` and
    /// `Option<T>` accept a `204 No Content`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, NetError> {
        let body: &[u8] = if self.body.iter().all(u8::is_ascii_whitespace) { b"null" } else { &self.body };
        serde_json::from_slice(body).map_err(|e| NetError::Decode(e.to_string()))
    }
}

/// A status outside 200-299, with what the server said.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError {
    /// The HTTP status.
    pub status: u16,
    /// The response headers, names in lowercase: a `304 Not Modified` answering a
    /// conditional request carries the new `ETag`, `Last-Modified` and `Cache-Control` here.
    pub headers: Vec<(String, String)>,
    /// The body, as text.
    pub body: String,
    /// The `detail` field of a JSON body: its text, or the JSON itself when it is not a
    /// string (a list of validation errors, say).
    pub detail: Option<String>,
}

impl HttpError {
    fn new(status: u16, headers: Vec<(String, String)>, body: &[u8]) -> Self {
        let body = String::from_utf8_lossy(body).into_owned();
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("detail").map(|d| d.as_str().map_or_else(|| d.to_string(), str::to_owned)));
        HttpError { status, headers, body, detail }
    }

    /// The first header named `name` (any case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Why a request gave no [`Response`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetError {
    /// The server answered with a status outside 200-299 (a `304 Not Modified` too, with
    /// its headers).
    Http(HttpError),
    /// The server could not be reached, or the connection broke.
    Network(String),
    /// No full answer came within the timeout.
    Timeout,
    /// The request was cancelled.
    Cancelled,
    /// The answer was not the JSON the caller asked for.
    Decode(String),
    /// The body was longer than the limit (in bytes) the request allowed.
    TooLarge(u64),
    /// The request was not sent: an address this pack does not contact (plain `http`
    /// beyond what the [`HttpPolicy`] allows) or a body that could not be encoded.
    Refused(String),
}

impl NetError {
    /// The HTTP status, when the server answered with an error.
    pub fn status(&self) -> Option<u16> {
        match self {
            NetError::Http(e) => Some(e.status),
            _ => None,
        }
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Http(e) => match &e.detail {
                Some(d) => write!(f, "the server answered {}: {d}", e.status),
                None => write!(f, "the server answered {}", e.status),
            },
            NetError::Network(e) => write!(f, "the server could not be reached: {e}"),
            NetError::Timeout => write!(f, "the server did not answer in time"),
            NetError::Cancelled => write!(f, "the request was cancelled"),
            NetError::Decode(e) => write!(f, "the answer was not readable: {e}"),
            NetError::TooLarge(n) => write!(f, "the answer was longer than {n} bytes"),
            NetError::Refused(e) => write!(f, "the request was not sent: {e}"),
        }
    }
}

impl std::error::Error for NetError {}

/// A `multipart/form-data` body: text fields and files, as an HTML form uploads them.
#[derive(Clone, Debug, Default)]
pub struct Multipart {
    parts: Vec<Part>,
}

#[derive(Clone, Debug)]
struct Part {
    name: String,
    file: Option<(String, String)>,
    bytes: Vec<u8>,
}

impl Multipart {
    /// An empty form.
    pub fn new() -> Self {
        Multipart::default()
    }

    /// Adds a text field.
    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parts.push(Part { name: name.into(), file: None, bytes: value.into().into_bytes() });
        self
    }

    /// Adds a file: its field name, file name, media type and bytes.
    pub fn file(
        mut self,
        name: impl Into<String>,
        file_name: impl Into<String>,
        content_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Self {
        self.parts.push(Part {
            name: name.into(),
            file: Some((file_name.into(), content_type.into())),
            bytes,
        });
        self
    }

    /// The body and its `Content-Type` (with the boundary).
    fn encode(&self) -> (Vec<u8>, String) {
        // A boundary that no part contains.
        let mut n = 0u64;
        let boundary = loop {
            let b = format!("remedy-ui-form-{n:016x}");
            let needle = b.as_bytes();
            if !self.parts.iter().any(|p| p.bytes.windows(needle.len()).any(|w| w == needle)) {
                break b;
            }
            n = n.wrapping_add(0x9e37_79b9_7f4a_7c15);
        };
        // Quotes and line breaks in names would end the header early.
        let q = |s: &str| s.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A");
        let mut body = Vec::new();
        for p in &self.parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match &p.file {
                Some((file, ty)) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                        q(&p.name),
                        q(file),
                        q(ty)
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", q(&p.name)).as_bytes(),
                ),
            }
            body.extend_from_slice(&p.bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (body, format!("multipart/form-data; boundary={boundary}"))
    }
}

/// One request: a method, a path (joined to the client's base address, or a full address),
/// headers and a body.
#[derive(Clone, Debug)]
pub struct Request {
    method: &'static str,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout: Option<Option<Duration>>,
    max_body: Option<Option<u64>>,
    keepalive: bool,
    error: Option<String>,
}

impl Request {
    fn new(method: &'static str, path: impl Into<String>) -> Self {
        Request {
            method,
            path: path.into(),
            headers: Vec::new(),
            body: None,
            timeout: None,
            max_body: None,
            keepalive: false,
            error: None,
        }
    }

    /// A `GET` of `path`.
    pub fn get(path: impl Into<String>) -> Self {
        Request::new("GET", path)
    }

    /// A `POST` to `path`.
    pub fn post(path: impl Into<String>) -> Self {
        Request::new("POST", path)
    }

    /// A `PUT` to `path`.
    pub fn put(path: impl Into<String>) -> Self {
        Request::new("PUT", path)
    }

    /// A `PATCH` to `path`.
    pub fn patch(path: impl Into<String>) -> Self {
        Request::new("PATCH", path)
    }

    /// A `DELETE` of `path`.
    pub fn delete(path: impl Into<String>) -> Self {
        Request::new("DELETE", path)
    }

    /// Sets a header (replacing one of the same name).
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
        self
    }

    /// A JSON body (`Content-Type: application/json`).
    pub fn json(mut self, value: &impl Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(b) => self.body = Some(b),
            Err(e) => self.error = Some(format!("the body is not JSON: {e}")),
        }
        self.header("Content-Type", "application/json")
    }

    /// A body of `content_type`.
    pub fn body(mut self, bytes: Vec<u8>, content_type: impl Into<String>) -> Self {
        self.body = Some(bytes);
        self.header("Content-Type", content_type)
    }

    /// A `multipart/form-data` body.
    pub fn multipart(self, form: &Multipart) -> Self {
        let (body, ty) = form.encode();
        self.body(body, ty)
    }

    /// How long the whole request may take, instead of the client's timeout; `None` for no
    /// limit.
    pub fn timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.timeout = Some(timeout.into());
        self
    }

    /// The longest body (in bytes) this request reads, instead of the client's
    /// [`Http::max_body`]; `None` for no limit.
    pub fn max_body(mut self, bytes: impl Into<Option<u64>>) -> Self {
        self.max_body = Some(bytes.into());
        self
    }

    /// Finishes even if the page or app closes meanwhile: the browser's `fetch` keepalive
    /// (whose bodies browsers limit to 64 KiB), or natively a request [`finish_keepalive`]
    /// waits for. Cancelling does not stop it.
    pub fn keepalive(mut self) -> Self {
        self.keepalive = true;
        self
    }
}

/// Which plain-`http` addresses a client contacts. `https` is always allowed.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HttpPolicy {
    /// Plain `http` only to the loopback interface (`localhost`, `127.0.0.1`, `[::1]`). The
    /// default.
    #[default]
    LoopbackOnly,
    /// Also plain `http` to the home or office network: private IPv4 addresses (RFC 1918:
    /// `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), link-local ones (`169.254.0.0/16`,
    /// `fe80::/10`), unique-local IPv6 (`fc00::/7`), and `.local` names (mDNS) that resolve
    /// to them. For an app that reaches a server on the same network, such as a Portal box.
    /// Natively the address a name resolves to is checked before connecting, so a name that
    /// resolves elsewhere (DNS rebinding) is refused, and plain `http` never goes through a
    /// proxy. In a page the browser decides on top (mixed content, local network access).
    PrivateNetworks,
}

/// An absolute address taken apart: what cookies and the address policy look at.
#[derive(Debug)]
pub(crate) struct Parts {
    pub(crate) scheme: String,
    /// In lowercase, without an IPv6 address's brackets.
    pub(crate) host: String,
    /// The path, without the query; `/` when empty.
    pub(crate) path: String,
}

/// Whether `url` starts with a scheme and `://` (a `://` further in, as in a query's value,
/// does not make an address absolute).
pub(crate) fn absolute(url: &str) -> bool {
    url.split_once("://").is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    })
}

impl Parts {
    pub(crate) fn of(url: &str) -> Option<Parts> {
        if !absolute(url) {
            return None;
        }
        let (scheme, rest) = url.split_once("://")?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = match authority.strip_prefix('[') {
            Some(v6) => v6.split_once(']')?.0,
            None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
        };
        let path = tail.split(['?', '#']).next().unwrap_or_default();
        Some(Parts {
            scheme: scheme.to_ascii_lowercase(),
            host: host.to_ascii_lowercase(),
            path: if path.is_empty() { "/".into() } else { path.to_owned() },
        })
    }

    /// `https`, or the loopback interface, where browsers treat plain `http` as secure too.
    pub(crate) fn secure(&self) -> bool {
        self.scheme == "https"
            || self.host == "localhost"
            || self.host.parse().is_ok_and(|ip| reaches(ip, HttpPolicy::LoopbackOnly))
    }
}

/// Whether plain `http` may connect to `ip` under `policy`.
pub(crate) fn reaches(ip: std::net::IpAddr, policy: HttpPolicy) -> bool {
    use std::net::IpAddr;
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_canonical(),
        v4 => v4,
    };
    let private = match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        // Unique-local fc00::/7 and link-local fe80::/10.
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
    };
    ip.is_loopback() || (policy == HttpPolicy::PrivateNetworks && private)
}

/// `https`, or plain `http` to what `policy` reaches (natively the resolved address is
/// checked again before connecting). Relative addresses (a browser page's own origin) pass.
fn allowed(url: &str, policy: HttpPolicy) -> Result<(), NetError> {
    let Some(at) = Parts::of(url) else {
        return if cfg!(target_arch = "wasm32") {
            Ok(())
        } else {
            Err(NetError::Refused(format!("not an absolute address: {url}")))
        };
    };
    let ok = match at.scheme.as_str() {
        "https" => true,
        "http" => {
            at.host == "localhost"
                || at.host.parse().is_ok_and(|ip| reaches(ip, policy))
                || (policy == HttpPolicy::PrivateNetworks && at.host.ends_with(".local"))
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else if policy == HttpPolicy::LoopbackOnly {
        Err(NetError::Refused(format!("not an https address: {url}")))
    } else {
        Err(NetError::Refused(format!("not an https address or one on the local network: {url}")))
    }
}

/// What reads a streamed body: the answer's status and headers with each chunk. Taken away
/// (set to `None`) once the request has answered otherwise, so no chunk comes after.
pub(crate) type Chunks = Arc<Mutex<Option<Box<dyn FnMut(&Response, &[u8]) + Send>>>>;

/// A request ready to go: the full address and everything the backend sends.
struct Prepared {
    method: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    /// `None`: no limit.
    timeout: Option<Duration>,
    keepalive: bool,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    cookies: Option<CookieJar>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    policy: HttpPolicy,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    max_redirects: usize,
    /// The longest body read; `None`: no limit.
    limit: Option<u64>,
    /// Where a streamed body goes, chunk by chunk, instead of into the [`Response`].
    chunks: Option<Chunks>,
}

/// The longest body a client reads unless set otherwise ([`Http::max_body`]): 256 MiB.
pub const DEFAULT_MAX_BODY: u64 = 256 * 1024 * 1024;
/// How long a request may take unless set otherwise ([`Http::timeout`]): 30 seconds.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Redirects followed natively unless set otherwise ([`Http::max_redirects`]): 10.
pub const DEFAULT_MAX_REDIRECTS: usize = 10;

/// An HTTP client for one base address. Cloning shares its settings and cookie jar.
#[derive(Clone, Debug)]
pub struct Http {
    base: String,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
    max_body: Option<u64>,
    max_redirects: usize,
    cookies: Option<CookieJar>,
    policy: HttpPolicy,
}

impl Http {
    /// A client for `base` (`https://host/api`; in a browser `""` or `/api` for the page's
    /// own origin). Requests time out after 30 seconds unless set otherwise.
    pub fn new(base: impl Into<String>) -> Self {
        Http {
            base: base.into(),
            headers: Vec::new(),
            timeout: Some(DEFAULT_TIMEOUT),
            max_body: Some(DEFAULT_MAX_BODY),
            max_redirects: DEFAULT_MAX_REDIRECTS,
            cookies: None,
            policy: HttpPolicy::default(),
        }
    }

    /// Sends `name: value` with every request (a request's own header of that name wins).
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
        self
    }

    /// How long a request may take in all, unless it sets its own ([`DEFAULT_TIMEOUT`]
    /// unless set); `None` for no limit.
    pub fn timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.timeout = timeout.into();
        self
    }

    /// The longest body (in bytes) a request reads, kept or streamed, unless it sets its own
    /// ([`DEFAULT_MAX_BODY`] unless set); `None` for no limit. A longer one ends with
    /// [`NetError::TooLarge`].
    pub fn max_body(mut self, bytes: impl Into<Option<u64>>) -> Self {
        self.max_body = bytes.into();
        self
    }

    /// How many redirects a native request follows ([`DEFAULT_MAX_REDIRECTS`] unless set);
    /// `0` follows none, so a `3xx` answers as an [`HttpError`] with its `Location`. In a page
    /// the browser follows them (up to its own 20).
    pub fn max_redirects(mut self, hops: usize) -> Self {
        self.max_redirects = hops;
        self
    }

    /// Which plain-`http` addresses this client contacts (only the loopback interface unless
    /// set otherwise).
    pub fn policy(mut self, policy: HttpPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Keeps cookies in `jar` natively, sent as RFC 6265 says (see [`CookieJar`]). A browser
    /// page keeps its own cookies and ignores this.
    pub fn cookies(mut self, jar: CookieJar) -> Self {
        self.cookies = Some(jar);
        self
    }

    /// The full address of `path`: itself when absolute, else joined to the base.
    pub fn url(&self, path: &str) -> String {
        if absolute(path) || self.base.is_empty() {
            return path.to_owned();
        }
        if path.is_empty() {
            return self.base.clone();
        }
        format!("{}/{}", self.base.trim_end_matches('/'), path.trim_start_matches('/'))
    }

    fn prepare(&self, req: Request) -> Result<Prepared, NetError> {
        if let Some(e) = req.error {
            return Err(NetError::Refused(e));
        }
        let url = self.url(&req.path);
        allowed(&url, self.policy)?;
        let mut headers: Vec<(String, String)> = self
            .headers
            .iter()
            .filter(|(k, _)| !req.headers.iter().any(|(r, _)| r.eq_ignore_ascii_case(k)))
            .cloned()
            .collect();
        headers.extend(req.headers);
        Ok(Prepared {
            method: req.method,
            url,
            headers,
            body: req.body,
            timeout: req.timeout.unwrap_or(self.timeout),
            max_redirects: self.max_redirects,
            keepalive: req.keepalive,
            cookies: self.cookies.clone(),
            policy: self.policy,
            limit: req.max_body.unwrap_or(self.max_body),
            chunks: None,
        })
    }

    /// Starts `req` and returns at once; `done` gets the answer when it ends: natively on a
    /// thread of the request's own, in a browser on the page's thread between frames.
    /// Cancelling `cancel` ends it with [`NetError::Cancelled`] (a keepalive request goes
    /// on, and still answers).
    pub fn send(
        &self,
        req: Request,
        cancel: &Cancel,
        done: impl FnOnce(Result<Response, NetError>) + Send + 'static,
    ) {
        match self.prepare(req) {
            Ok(p) => {
                #[cfg(not(target_arch = "wasm32"))]
                native::send(p, cancel.clone(), done);
                #[cfg(target_arch = "wasm32")]
                web::send(p, cancel.clone(), done);
            }
            Err(e) => done(Err(e)),
        }
    }

    /// Like [`Http::send`], with the body handed to `chunk` as it arrives instead of kept:
    /// for a download, a long answer or one the server sends bit by bit. `chunk` gets the
    /// answer's status and headers (an empty body) with each piece; `done` then gets the
    /// same [`Response`], or the error, after which no chunk comes. A body longer than the
    /// request's [`Request::max_body`] (or the client's [`Http::max_body`]) ends it with
    /// [`NetError::TooLarge`]; cancelling ends it between two chunks (the connection closes).
    /// A status outside 200-299 sends no chunk: its body is in the [`HttpError`]. In a page
    /// this reads the `fetch` body's `ReadableStream`.
    pub fn stream(
        &self,
        req: Request,
        cancel: &Cancel,
        chunk: impl FnMut(&Response, &[u8]) + Send + 'static,
        done: impl FnOnce(Result<Response, NetError>) + Send + 'static,
    ) {
        match self.prepare(req) {
            Ok(mut p) => {
                p.chunks = Some(Arc::new(Mutex::new(Some(Box::new(chunk)))));
                #[cfg(not(target_arch = "wasm32"))]
                native::send(p, cancel.clone(), done);
                #[cfg(target_arch = "wasm32")]
                web::send(p, cancel.clone(), done);
            }
            Err(e) => done(Err(e)),
        }
    }

    /// Like [`Http::send`], with the body decoded from JSON into `T`.
    pub fn send_json<T: DeserializeOwned + 'static>(
        &self,
        req: Request,
        cancel: &Cancel,
        done: impl FnOnce(Result<T, NetError>) + Send + 'static,
    ) {
        let req = if req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
            req
        } else {
            req.header("Accept", "application/json")
        };
        self.send(req, cancel, move |out| done(out.and_then(|r| r.json())));
    }

    /// Sends `req` and waits for the answer, on a thread that may wait (a worker, a
    /// [`Resource::new`] fetch). Natively only: a browser page cannot wait.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_blocking(&self, req: Request, cancel: &Cancel) -> Result<Response, NetError> {
        native::send_blocking(self.prepare(req)?, cancel)
    }

    /// A resource holding the JSON that `request(key)` answers, refetched when the key
    /// `source` returns changes; the request for the old key is cancelled.
    ///
    /// ```no_run
    /// # use remedy_ui_platform::services_net::{Http, NetError, Request};
    /// # use remedy_ui_core::reactive::{Resource, Signal};
    /// let page = Signal::new(1u32);
    /// let http = Http::new("https://example.com/api");
    /// let items: Resource<Vec<String>, NetError> =
    ///     http.resource(move || page.get(), |p| Request::get(format!("items?page={p}")));
    /// ```
    #[track_caller]
    pub fn resource<K, T>(
        &self,
        source: impl Fn() -> K + 'static,
        request: impl Fn(&K) -> Request + 'static,
    ) -> Resource<T, NetError>
    where
        K: PartialEq + Clone + 'static,
        T: DeserializeOwned + Send + 'static,
    {
        let http = self.clone();
        Resource::with_reply(source, move |key, cancel, reply| {
            http.send_json(request(&key), &cancel, move |out| reply.send(out));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_addresses() {
        let h = Http::new("https://a.test/api/");
        assert_eq!(h.url("/x"), "https://a.test/api/x");
        assert_eq!(h.url("x?y=1"), "https://a.test/api/x?y=1");
        assert_eq!(h.url("https://b.test/z"), "https://b.test/z");
        assert_eq!(Http::new("").url("/x"), "/x");
        // An address inside a query does not make the path absolute.
        assert_eq!(h.url("go?to=https://b.test/z"), "https://a.test/api/go?to=https://b.test/z");
        assert!(Parts::of("/go?to=https://b.test/z").is_none());
        assert!(allowed("https://a.test/go?to=http://b.test/z", HttpPolicy::LoopbackOnly).is_ok());
    }

    #[test]
    fn refuses_plain_http_off_loopback() {
        let lo = HttpPolicy::LoopbackOnly;
        assert!(allowed("https://a.test", lo).is_ok());
        assert!(allowed("http://127.0.0.1:8080/x", lo).is_ok());
        assert!(allowed("http://localhost/x", lo).is_ok());
        assert!(allowed("http://[::1]:80/x", lo).is_ok());
        assert!(allowed("http://localhost.evil.test/x", lo).is_err());
        assert!(allowed("http://a.test", lo).is_err());
        assert!(allowed("http://192.168.1.4/x", lo).is_err());
        assert!(allowed("ftp://a.test", lo).is_err());
    }

    #[test]
    fn private_networks_when_asked() {
        let lan = HttpPolicy::PrivateNetworks;
        for ok in [
            "http://10.1.2.3/x",
            "http://172.16.0.9:8000",
            "http://172.31.255.1",
            "http://192.168.1.4:8080/api",
            "http://169.254.10.10",
            "http://[fd12:3456::1]:8080/",
            "http://[fe80::1]/",
            "http://[::ffff:192.168.0.2]/",
            "http://portal.local:8000/",
            "http://127.0.0.1",
        ] {
            assert!(allowed(ok, lan).is_ok(), "{ok}");
        }
        for no in [
            "http://172.32.0.1",
            "http://8.8.8.8",
            "http://[2001:db8::1]/",
            "http://portal.lan/",
            "http://local/",
            "http://user@8.8.8.8/",
            "http://192.168.1.4.evil.test/",
        ] {
            assert!(allowed(no, lan).is_err(), "{no}");
        }
    }

    #[test]
    fn reads_the_detail() {
        let e = HttpError::new(422, Vec::new(), br#"{"detail":[{"msg":"bad"}]}"#);
        assert_eq!(e.detail.as_deref(), Some(r#"[{"msg":"bad"}]"#));
        let e = HttpError::new(401, vec![("etag".into(), "\"v\"".into())], br#"{"detail":"pair first"}"#);
        assert_eq!((e.detail.as_deref(), e.header("ETag")), (Some("pair first"), Some("\"v\"")));
        assert_eq!(HttpError::new(500, Vec::new(), b"oops").detail, None);
    }

    #[test]
    fn takes_addresses_apart() {
        let p = Parts::of("https://u:p@A.test:443/x/y?q#f").unwrap_or_else(|| unreachable!());
        assert_eq!((p.scheme.as_str(), p.host.as_str(), p.path.as_str()), ("https", "a.test", "/x/y"));
        let p = Parts::of("http://[::1]:8080?x").unwrap_or_else(|| unreachable!());
        assert_eq!((p.host.as_str(), p.path.as_str()), ("::1", "/"));
        assert!(p.secure());
        assert!(!Parts::of("http://10.0.0.1/").is_some_and(|p| p.secure()));
        assert!(Parts::of("/relative").is_none());
    }

    #[test]
    fn multipart_boundary_avoids_the_content() {
        let form = Multipart::new().text("a", "remedy-ui-form-0000000000000000").file(
            "f",
            "x\".txt",
            "text/plain",
            b"hi".to_vec(),
        );
        let (body, ty) = form.encode();
        let boundary = ty.trim_start_matches("multipart/form-data; boundary=");
        assert_ne!(boundary, "remedy-ui-form-0000000000000000");
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("filename=\"x%22.txt\""));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }
}
