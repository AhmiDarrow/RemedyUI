//! RemedyUI HTTP client: the same calls in a native app and in a browser page.
//!
//! - [`Http`]: a client for one base address, with default headers, a timeout and (natively)
//!   an optional [`CookieJar`]. In a browser the page's own cookies go with requests to its
//!   origin (`credentials: same-origin`), as `fetch` does by default.
//! - [`Request`]: `GET`, `POST`, `PUT` or `DELETE` a path, with headers, a JSON, raw or
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
//! - Natively [`Http::send_blocking`] also waits for the answer, for work already on a
//!   background thread.
//!
//! **Policy.** This pack is opt-in and contacts only addresses the app names. Natively it
//! refuses plain `http` beyond the loopback interface (TLS is rustls with the platform's
//! certificate verifier); in a browser a relative path goes to the page's own origin and the
//! browser's own rules (mixed content, CORS) apply on top. Nothing is logged, and no header
//! is added beyond what the app sets and `User-Agent` natively.
//!
//! ```no_run
//! use remedy_ui_services_net::{Http, Request};
//! use remedy_ui_reactive::Cancel;
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

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use remedy_ui_reactive::{Cancel, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::finish_keepalive;

/// The answer to a request that succeeded (status 200-299).
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError {
    /// The HTTP status.
    pub status: u16,
    /// The body, as text.
    pub body: String,
    /// The `detail` field of a JSON body: its text, or the JSON itself when it is not a
    /// string (a list of validation errors, say).
    pub detail: Option<String>,
}

impl HttpError {
    fn new(status: u16, body: &[u8]) -> Self {
        let body = String::from_utf8_lossy(body).into_owned();
        let detail = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| {
            v.get("detail").map(|d| d.as_str().map_or_else(|| d.to_string(), str::to_owned))
        });
        HttpError { status, body, detail }
    }
}

/// Why a request gave no [`Response`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetError {
    /// The server answered with a status outside 200-299.
    Http(HttpError),
    /// The server could not be reached, or the connection broke.
    Network(String),
    /// No full answer came within the timeout.
    Timeout,
    /// The request was cancelled.
    Cancelled,
    /// The answer was not the JSON the caller asked for.
    Decode(String),
    /// The request was not sent: an address this pack does not contact (plain `http`
    /// beyond the loopback interface) or a body that could not be encoded.
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
        self.parts.push(Part { name: name.into(), file: Some((file_name.into(), content_type.into())), bytes });
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
    timeout: Option<Duration>,
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

    /// How long the whole request may take, instead of the client's timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
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

/// Cookies kept between native requests, per origin (scheme, host and port), as a server
/// sets them with `Set-Cookie`. A browser page needs none: the browser keeps the page's.
///
/// shortcut: cookies are scoped to the exact origin that set them; `Domain`, `Path` and
/// `Expires` are not applied (only `Max-Age=0` removes one); upgrade when an app talks to
/// several hosts that share cookies.
#[derive(Clone, Debug, Default)]
pub struct CookieJar(Arc<Mutex<BTreeMap<String, BTreeMap<String, String>>>>);

impl CookieJar {
    /// An empty jar.
    pub fn new() -> Self {
        CookieJar::default()
    }

    /// The cookie named `name` kept for `origin` (`https://host:port`, the port only when
    /// not the scheme's usual one).
    pub fn get(&self, origin: &str, name: &str) -> Option<String> {
        let jar = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        jar.get(origin).and_then(|c| c.get(name)).cloned()
    }

    /// Keeps a cookie for `origin`, as if the server had set it.
    pub fn set(&self, origin: &str, name: impl Into<String>, value: impl Into<String>) {
        let mut jar = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        jar.entry(origin.to_owned()).or_default().insert(name.into(), value.into());
    }

    /// Forgets every cookie.
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    /// The `Cookie` header for a request to `url`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn header(&self, url: &str) -> Option<String> {
        let jar = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let c = jar.get(&origin(url))?;
        (!c.is_empty()).then(|| c.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "))
    }

    /// Takes in a `Set-Cookie` header from `url`'s answer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn store(&self, url: &str, set_cookie: &str) {
        let mut attrs = set_cookie.split(';');
        let Some((name, value)) = attrs.next().and_then(|nv| nv.split_once('=')) else { return };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            return;
        }
        let gone = attrs.any(|a| {
            a.split_once('=').is_some_and(|(k, v)| {
                k.trim().eq_ignore_ascii_case("max-age") && v.trim().parse::<i64>().is_ok_and(|s| s <= 0)
            })
        });
        let mut jar = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let cookies = jar.entry(origin(url)).or_default();
        if gone {
            cookies.remove(name);
        } else {
            cookies.insert(name.to_owned(), value.to_owned());
        }
    }
}

/// `scheme://host[:port]` of an absolute address.
fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let usual = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    let host = host.strip_suffix(usual).unwrap_or(host);
    format!("{}://{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
}

/// Only HTTPS, except the loopback interface. Relative addresses (a browser page's own
/// origin) pass.
fn allowed(url: &str) -> Result<(), NetError> {
    let lower = url.to_ascii_lowercase();
    if !lower.contains("://") {
        return if cfg!(target_arch = "wasm32") {
            Ok(())
        } else {
            Err(NetError::Refused(format!("not an absolute address: {url}")))
        };
    }
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| lower.strip_prefix(p).is_some_and(|r| r.is_empty() || r.starts_with([':', '/', '?', '#'])));
    if lower.starts_with("https://") || local {
        Ok(())
    } else {
        Err(NetError::Refused(format!("not an https address: {url}")))
    }
}

/// A request ready to go: the full address and everything the backend sends.
struct Prepared {
    method: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout: Duration,
    keepalive: bool,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    cookies: Option<CookieJar>,
}

/// An HTTP client for one base address. Cloning shares its settings and cookie jar.
#[derive(Clone, Debug)]
pub struct Http {
    base: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
    cookies: Option<CookieJar>,
}

impl Http {
    /// A client for `base` (`https://host/api`; in a browser `""` or `/api` for the page's
    /// own origin). Requests time out after 30 seconds unless set otherwise.
    pub fn new(base: impl Into<String>) -> Self {
        Http { base: base.into(), headers: Vec::new(), timeout: Duration::from_secs(30), cookies: None }
    }

    /// Sends `name: value` with every request (a request's own header of that name wins).
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
        self
    }

    /// How long a request may take in all, unless it sets its own.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Keeps cookies in `jar` natively: sent with each request to the origin that set them.
    /// A browser page keeps its own cookies and ignores this.
    pub fn cookies(mut self, jar: CookieJar) -> Self {
        self.cookies = Some(jar);
        self
    }

    /// The full address of `path`: itself when absolute, else joined to the base.
    pub fn url(&self, path: &str) -> String {
        if path.contains("://") || self.base.is_empty() {
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
        allowed(&url)?;
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
            keepalive: req.keepalive,
            cookies: self.cookies.clone(),
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
    /// # use remedy_ui_services_net::{Http, NetError, Request};
    /// # use remedy_ui_reactive::{Resource, Signal};
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
    }

    #[test]
    fn refuses_plain_http_off_loopback() {
        assert!(allowed("https://a.test").is_ok());
        assert!(allowed("http://127.0.0.1:8080/x").is_ok());
        assert!(allowed("http://localhost/x").is_ok());
        assert!(allowed("http://localhost.evil.test/x").is_err());
        assert!(allowed("http://a.test").is_err());
    }

    #[test]
    fn reads_the_detail() {
        let e = HttpError::new(422, br#"{"detail":[{"msg":"bad"}]}"#);
        assert_eq!(e.detail.as_deref(), Some(r#"[{"msg":"bad"}]"#));
        assert_eq!(HttpError::new(401, br#"{"detail":"pair first"}"#).detail.as_deref(), Some("pair first"));
        assert_eq!(HttpError::new(500, b"oops").detail, None);
    }

    #[test]
    fn origins_and_cookies() {
        assert_eq!(origin("https://A.test:443/x?y"), "https://a.test");
        assert_eq!(origin("http://127.0.0.1:8080/x"), "http://127.0.0.1:8080");
        let jar = CookieJar::new();
        jar.store("https://a.test/login", "sid=1; Path=/; HttpOnly");
        jar.store("https://a.test/login", "theme=dark");
        assert_eq!(jar.header("https://a.test/other").as_deref(), Some("sid=1; theme=dark"));
        assert_eq!(jar.header("https://b.test/"), None);
        jar.store("https://a.test/logout", "sid=; Max-Age=0");
        assert_eq!(jar.header("https://a.test/").as_deref(), Some("theme=dark"));
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
