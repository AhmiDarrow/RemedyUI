//! Cookies natively, as RFC 6265 describes a user agent keeping them: `Domain` and host-only
//! cookies, `Path`, `Secure`, `HttpOnly`, `Expires` and `Max-Age`, and `SameSite` kept; with
//! RFC 6265bis's size limits, `__Secure-` and `__Host-` name prefixes and its rule that a
//! plain-http origin leaves `Secure` cookies alone.

use std::sync::{Arc, Mutex, PoisonError};

use crate::services_net::Parts;

/// How a cookie's `SameSite` attribute limits it in a browser. A native app makes no
/// cross-site requests (each one is the app's own), so the jar keeps it and sends the cookie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    /// `SameSite=Strict`.
    Strict,
    /// `SameSite=Lax`.
    Lax,
    /// `SameSite=None`.
    None,
}

/// A cookie the jar keeps.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
    /// The host it goes to, in lowercase; with [`Cookie::host_only`] false, its subdomains too.
    pub domain: String,
    /// Set without a `Domain` attribute: only the exact host that set it gets it.
    pub host_only: bool,
    /// The path it goes to, and the paths below it.
    pub path: String,
    /// Sent only over `https` (or to the loopback interface).
    pub secure: bool,
    /// Hidden from scripts in a browser; a native client is an HTTP client, so it is sent.
    pub http_only: bool,
    /// Its `SameSite` attribute, when it had a known one.
    pub same_site: Option<SameSite>,
    /// When it expires, in seconds since 1970; `None` for one that lasts the session.
    pub expires: Option<i64>,
    created: u64,
}

/// Cookies kept between native requests, as a server sets them with `Set-Cookie` and as
/// RFC 6265 sends them back: to the host that set them (or its subdomains, with `Domain`),
/// under their `Path`, `Secure` ones only over `https` or to the loopback interface, until
/// they expire (`Expires`, `Max-Age`). As RFC 6265bis says, a cookie whose name and value
/// are longer than 4096 bytes is ignored (an attribute longer than 1024 bytes too), a
/// `__Secure-` name needs `Secure`, a `__Host-` name also `Path=/` and no `Domain`, and a
/// plain-http origin cannot replace a `Secure` cookie. A `Domain` of a single label (`com`) is refused; a
/// longer public suffix (`co.uk`) is not known without the public suffix list, so the jar
/// trusts the servers the app talks to. A browser page needs no jar: the browser keeps the
/// page's cookies.
#[derive(Clone, Debug, Default)]
pub struct CookieJar(Arc<Mutex<Jar>>);

/// The cookies a jar keeps unless set otherwise ([`CookieJar::max_cookies`]): 3000, what
/// browsers keep in all.
pub const DEFAULT_MAX_COOKIES: usize = 3000;

/// The cookies a jar keeps for one domain unless set otherwise
/// ([`CookieJar::max_per_host`]): 180, about what browsers keep, so one server cannot push
/// out every other server's cookies.
pub const DEFAULT_MAX_COOKIES_PER_HOST: usize = 180;

/// RFC 6265bis 5.6: the longest name and value together, in bytes.
const MAX_PAIR: usize = 4096;
/// RFC 6265bis 5.6: the longest attribute value, in bytes.
const MAX_ATTRIBUTE: usize = 1024;

#[derive(Debug)]
struct Jar {
    cookies: Vec<Cookie>,
    next: u64,
    max: Option<usize>,
    per_host: Option<usize>,
}

impl Default for Jar {
    fn default() -> Self {
        Jar {
            cookies: Vec::new(),
            next: 0,
            max: Some(DEFAULT_MAX_COOKIES),
            per_host: Some(DEFAULT_MAX_COOKIES_PER_HOST),
        }
    }
}

/// Seconds since 1970.
#[cfg(not(target_arch = "wasm32"))]
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Seconds since 1970 (a page's `SystemTime` panics).
#[cfg(target_arch = "wasm32")]
#[allow(clippy::cast_possible_truncation)]
fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

impl CookieJar {
    /// An empty jar, keeping up to [`DEFAULT_MAX_COOKIES`], [`DEFAULT_MAX_COOKIES_PER_HOST`]
    /// of them for one domain.
    pub fn new() -> Self {
        CookieJar::default()
    }

    /// Keeps at most `max` cookies, forgetting the oldest beyond that; `None` for no limit.
    pub fn max_cookies(self, max: impl Into<Option<usize>>) -> Self {
        let mut jar = self.lock();
        jar.max = max.into();
        jar.trim();
        drop(jar);
        self
    }

    /// Keeps at most `max` cookies of one domain ([`Cookie::domain`]), forgetting that
    /// domain's oldest beyond that; `None` for no limit.
    pub fn max_per_host(self, max: impl Into<Option<usize>>) -> Self {
        let mut jar = self.lock();
        jar.per_host = max.into();
        let mut domains: Vec<String> = jar.cookies.iter().map(|c| c.domain.clone()).collect();
        domains.sort_unstable();
        domains.dedup();
        for d in &domains {
            jar.trim_host(d);
        }
        drop(jar);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Jar> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The value of the cookie named `name` that a request to `url` would send.
    pub fn get(&self, url: &str, name: &str) -> Option<String> {
        self.matching(url, now()).into_iter().find(|c| c.name == name).map(|c| c.value)
    }

    /// Keeps a cookie for `url`'s host and every path, as if the server had set
    /// `name=value; Path=/` (with `Secure` for a `__Secure-` or `__Host-` name, which
    /// promises it). Like a server's, it is ignored when `url` is plain `http` beyond the
    /// loopback interface and the name has such a prefix or a `Secure` cookie has that name.
    pub fn set(&self, url: &str, name: impl Into<String>, value: impl Into<String>) {
        let Some(at) = Parts::of(url) else { return };
        let name = name.into();
        let from_secure = at.secure();
        let secure = prefixed(&name, "__Secure-") || prefixed(&name, "__Host-");
        if secure && !from_secure {
            return;
        }
        let cookie = Cookie {
            name,
            value: value.into(),
            domain: at.host,
            host_only: true,
            path: "/".into(),
            secure,
            http_only: false,
            same_site: None,
            expires: None,
            created: 0,
        };
        self.lock().keep(cookie, from_secure, now());
    }

    /// Every cookie kept and not yet expired.
    pub fn cookies(&self) -> Vec<Cookie> {
        let now = now();
        let mut jar = self.lock();
        jar.cookies.retain(|c| c.expires.is_none_or(|e| e > now));
        jar.cookies.clone()
    }

    /// Forgets every cookie.
    pub fn clear(&self) {
        self.lock().cookies.clear();
    }

    /// The cookies a request to `url` sends, longest path first, then oldest first.
    fn matching(&self, url: &str, now: i64) -> Vec<Cookie> {
        let Some(at) = Parts::of(url) else { return Vec::new() };
        let secure = at.secure();
        let mut jar = self.lock();
        jar.cookies.retain(|c| c.expires.is_none_or(|e| e > now));
        let mut out: Vec<Cookie> = jar
            .cookies
            .iter()
            .filter(|c| {
                (if c.host_only { c.domain == at.host } else { domain_match(&at.host, &c.domain) })
                    && path_match(&at.path, &c.path)
                    && (!c.secure || secure)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.created.cmp(&b.created)));
        out
    }

    /// The `Cookie` header for a request to `url`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn header(&self, url: &str) -> Option<String> {
        let c = self.matching(url, now());
        (!c.is_empty())
            .then(|| c.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; "))
    }

    /// Takes in a `Set-Cookie` header from `url`'s answer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn store(&self, url: &str, set_cookie: &str) {
        let now = now();
        if let Some((c, from_secure)) = parse(url, set_cookie, now) {
            self.lock().keep(c, from_secure, now);
        }
    }
}

impl Jar {
    /// Replaces the cookie of the same name, domain and path (keeping when it was first
    /// set), or drops it when the new one has already expired. `from_secure`: its origin was
    /// `https` or the loopback interface.
    fn keep(&mut self, mut c: Cookie, from_secure: bool, now: i64) {
        // RFC 6265bis 5.7: a plain-http origin neither replaces nor shadows a Secure cookie.
        if !from_secure
            && self.cookies.iter().any(|o| {
                o.secure
                    && o.name == c.name
                    && (domain_match(&o.domain, &c.domain) || domain_match(&c.domain, &o.domain))
                    && path_match(&c.path, &o.path)
                    && o.expires.is_none_or(|e| e > now)
            })
        {
            return;
        }
        let old =
            self.cookies.iter().position(|o| o.name == c.name && o.domain == c.domain && o.path == c.path);
        let created = old.map(|i| self.cookies.remove(i).created);
        if c.expires.is_some_and(|e| e <= now) {
            return;
        }
        c.created = created.unwrap_or_else(|| {
            self.next += 1;
            self.next
        });
        let domain = c.domain.clone();
        self.cookies.push(c);
        self.trim_host(&domain);
        self.trim();
    }

    /// Forgets the oldest cookies of `domain` beyond the limit for one domain.
    fn trim_host(&mut self, domain: &str) {
        let Some(max) = self.per_host else { return };
        while self.cookies.iter().filter(|c| c.domain == domain).count() > max {
            let of_domain = self.cookies.iter().enumerate().filter(|(_, c)| c.domain == domain);
            let Some(oldest) = of_domain.min_by_key(|(_, c)| c.created).map(|(i, _)| i) else {
                return;
            };
            self.cookies.remove(oldest);
        }
    }

    /// Forgets the oldest cookies beyond the limit.
    fn trim(&mut self) {
        let Some(max) = self.max else { return };
        while self.cookies.len() > max {
            let Some(oldest) = self.cookies.iter().enumerate().min_by_key(|(_, c)| c.created).map(|(i, _)| i)
            else {
                return;
            };
            self.cookies.remove(oldest);
        }
    }
}

/// RFC 6265 5.1.3: `host` is `domain` or a subdomain of it (never for an IP address).
fn domain_match(host: &str, domain: &str) -> bool {
    host == domain
        || (host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.'))
            && host.parse::<std::net::IpAddr>().is_err())
}

/// RFC 6265 5.1.4: `path` is `cookie` or below it.
fn path_match(path: &str, cookie: &str) -> bool {
    path == cookie
        || path.strip_prefix(cookie).is_some_and(|rest| cookie.ends_with('/') || rest.starts_with('/'))
}

/// RFC 6265 5.1.4: the directory of the request's path.
fn default_path(path: &str) -> String {
    match path.rfind('/') {
        Some(i) if path.starts_with('/') && i > 0 => path.get(..i).unwrap_or("/").to_owned(),
        _ => "/".to_owned(),
    }
}

/// RFC 6265bis 4.1.3: whether `name` starts with `prefix` (`__Secure-`, `__Host-`) in any case.
fn prefixed(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix))
}

/// RFC 6265 5.2 and 5.3 with RFC 6265bis's limits: the cookie a `Set-Cookie` header from `url`
/// sets and whether `url` is a secure origin, or `None` when a user agent ignores it.
fn parse(url: &str, header: &str, now: i64) -> Option<(Cookie, bool)> {
    let at = Parts::of(url)?;
    // A control character anywhere makes a user agent ignore the whole header.
    if header.bytes().any(|b| b.is_ascii_control() && b != b'\t') {
        return None;
    }
    let mut attrs = header.split(';');
    let (name, value) = attrs.next()?.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() || name.len() + value.len() > MAX_PAIR {
        return None;
    }
    let (mut expires, mut max_age, mut domain, mut path) = (None, None, None, None);
    let (mut secure, mut http_only, mut same_site) = (false, false, None);
    for a in attrs {
        let (k, v) = a.split_once('=').map_or((a.trim(), ""), |(k, v)| (k.trim(), v.trim()));
        if v.len() > MAX_ATTRIBUTE {
            continue;
        }
        match k.to_ascii_lowercase().as_str() {
            "expires" => expires = cookie_date(v).or(expires),
            "max-age" => {
                let digits = v.strip_prefix('-').unwrap_or(v);
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                    // Too many seconds to count is as good as forever.
                    let n = v.parse::<i64>().unwrap_or(if v.starts_with('-') { i64::MIN } else { i64::MAX });
                    max_age = Some(if n <= 0 { i64::MIN } else { now.saturating_add(n) });
                }
            }
            "domain" if !v.is_empty() => {
                domain = Some(v.strip_prefix('.').unwrap_or(v).to_ascii_lowercase());
            }
            "path" => path = v.starts_with('/').then(|| v.to_owned()),
            "secure" => secure = true,
            "httponly" => http_only = true,
            "samesite" => {
                same_site = match v.to_ascii_lowercase().as_str() {
                    "strict" => Some(SameSite::Strict),
                    "lax" => Some(SameSite::Lax),
                    "none" => Some(SameSite::None),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    // A secure cookie only from a secure origin (RFC 6265bis), as browsers keep them.
    let from_secure = at.secure();
    if secure && !from_secure {
        return None;
    }
    // RFC 6265bis 4.1.3: a prefixed name is only kept with what the prefix promises.
    let host_prefix = prefixed(name, "__Host-");
    if (host_prefix || prefixed(name, "__Secure-")) && !secure {
        return None;
    }
    if host_prefix && (domain.is_some() || path.as_deref() != Some("/")) {
        return None;
    }
    let (domain, host_only) = match domain {
        Some(d) if d == at.host => (d, false),
        // A single label (`com`) is a public suffix; a domain the host is not under is refused.
        Some(d) if !d.contains('.') || !domain_match(&at.host, &d) => return None,
        Some(d) => (d, false),
        None => (at.host.clone(), true),
    };
    let cookie = Cookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain,
        host_only,
        path: path.unwrap_or_else(|| default_path(&at.path)),
        secure,
        http_only,
        same_site,
        expires: max_age.or(expires),
        created: 0,
    };
    Some((cookie, from_secure))
}

/// RFC 6265 5.1.1: a cookie date (`Wed, 21 Oct 2026 07:28:00 GMT` and the older forms), in
/// seconds since 1970.
fn cookie_date(s: &str) -> Option<i64> {
    let delimiter = |c: char| matches!(c, '\t' | ' '..='/' | ';'..='@' | '['..='`' | '{'..='~');
    // Up to `max` leading digits (at least `min`), then nothing or a non-digit.
    let digits = |t: &str, min: usize, max: usize| -> Option<i64> {
        let n = t.bytes().take_while(u8::is_ascii_digit).count();
        (min..=max).contains(&n).then(|| t.get(..n)?.parse().ok()).flatten()
    };
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for t in s.split(delimiter).filter(|t| !t.is_empty()) {
        if time.is_none() {
            let mut hms = t.splitn(3, ':');
            if let (Some(h), Some(m), Some(sec)) = (hms.next(), hms.next(), hms.next())
                && h.len() <= 2
                && m.len() <= 2
                && let (Some(h), Some(m), Some(sec)) = (digits(h, 1, 2), digits(m, 1, 2), digits(sec, 1, 2))
            {
                time = Some((h, m, sec));
                continue;
            }
        }
        if day.is_none()
            && let Some(d) = digits(t, 1, 2)
        {
            day = Some(d);
            continue;
        }
        if month.is_none() {
            let lower = t.get(..3).map(str::to_ascii_lowercase);
            let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
            if let Some(m) = lower.and_then(|l| names.iter().position(|n| *n == l)) {
                month = Some(i64::try_from(m).ok()? + 1);
                continue;
            }
        }
        if year.is_none()
            && let Some(y) = digits(t, 2, 4)
        {
            year = Some(y);
        }
    }
    let ((h, mi, sec), d, mo, y) = (time?, day?, month?, year?);
    let y = match y {
        70..=99 => y + 1900,
        0..=69 => y + 2000,
        y => y,
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let in_month = month_days.get(usize::try_from(mo - 1).ok()?).copied()?;
    if y < 1601 || d < 1 || d > in_month || h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    // Days since 1970 of a civil date (Howard Hinnant's algorithm).
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(cookie_date("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1_445_412_480));
        assert_eq!(cookie_date("Sunday, 06-Nov-94 08:49:37 GMT"), Some(784_111_777));
        assert_eq!(cookie_date("Sun Nov  6 08:49:37 1994"), Some(784_111_777));
        assert_eq!(cookie_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(cookie_date("Fri, 30 Feb 2024 00:00:00 GMT"), None);
        assert_eq!(cookie_date("Thu, 29 Feb 2024 00:00:00 GMT"), Some(1_709_164_800));
        assert_eq!(cookie_date("not a date"), None);
    }

    #[test]
    fn paths() {
        assert_eq!(default_path("/a/b/c"), "/a/b");
        assert_eq!(default_path("/a"), "/");
        assert_eq!(default_path(""), "/");
        assert!(path_match("/a/b", "/a"));
        assert!(path_match("/a/", "/a/"));
        assert!(path_match("/a/b", "/a/"));
        assert!(!path_match("/ab", "/a"));
        assert!(domain_match("x.a.test", "a.test"));
        assert!(!domain_match("xa.test", "a.test"));
        assert!(!domain_match("10.0.0.1", "0.0.1"));
    }

    #[test]
    fn domains_paths_and_security() {
        let jar = CookieJar::new();
        jar.store("https://www.a.test/login/form", "sid=1; Domain=.A.test; Path=/; HttpOnly; SameSite=Lax");
        jar.store("https://www.a.test/login/form", "here=1");
        jar.store("https://www.a.test/", "tok=s; Secure; Path=/api");
        jar.store("https://www.a.test/", "far=1; Domain=b.test");
        jar.store("https://www.a.test/", "tld=1; Domain=test");
        assert_eq!(jar.header("https://api.a.test/").as_deref(), Some("sid=1"));
        assert_eq!(jar.header("https://www.a.test/login/x").as_deref(), Some("here=1; sid=1"));
        assert_eq!(jar.header("https://www.a.test/api/v1").as_deref(), Some("tok=s; sid=1"));
        assert_eq!(
            jar.header("http://www.a.test/api/v1").as_deref(),
            Some("sid=1"),
            "secure only over https"
        );
        assert_eq!(jar.header("https://b.test/"), None);
        let sid = jar.cookies().into_iter().find(|c| c.name == "sid").unwrap_or_else(|| unreachable!());
        assert!(sid.http_only && !sid.host_only && sid.domain == "a.test");
        assert_eq!(sid.same_site, Some(SameSite::Lax));
        // A secure cookie from a plain-http origin is refused, except on the loopback interface.
        jar.store("http://10.0.0.2/", "s=1; Secure");
        assert_eq!(jar.get("https://10.0.0.2/", "s"), None);
        jar.store("http://127.0.0.1:8080/", "s=1; Secure");
        assert_eq!(jar.get("http://127.0.0.1:8080/", "s").as_deref(), Some("1"));
    }

    #[test]
    fn the_limit_can_be_set() {
        let jar = CookieJar::new().max_cookies(2);
        for n in ["a", "b", "c"] {
            jar.store("https://a.test/", &format!("{n}=1"));
        }
        assert_eq!(jar.header("https://a.test/").as_deref(), Some("b=1; c=1"), "the oldest went");
        let jar = jar.max_cookies(None).max_per_host(None);
        for n in 0..(DEFAULT_MAX_COOKIES + 5) {
            jar.store("https://a.test/", &format!("n{n}=1"));
        }
        assert_eq!(jar.cookies().len(), DEFAULT_MAX_COOKIES + 7);
        assert_eq!(CookieJar::new().lock().max, Some(DEFAULT_MAX_COOKIES));
    }

    #[test]
    fn one_domain_cannot_fill_the_jar() {
        let jar = CookieJar::new().max_per_host(2);
        jar.store("https://b.test/", "keep=1");
        for n in ["a", "b", "c"] {
            jar.store("https://a.test/", &format!("{n}=1"));
        }
        assert_eq!(jar.header("https://a.test/").as_deref(), Some("b=1; c=1"), "its own oldest went");
        assert_eq!(jar.get("https://b.test/", "keep").as_deref(), Some("1"));
        let jar = jar.max_per_host(1);
        assert_eq!(jar.header("https://a.test/").as_deref(), Some("c=1"));
        assert_eq!(CookieJar::new().lock().per_host, Some(DEFAULT_MAX_COOKIES_PER_HOST));
    }

    #[test]
    fn sizes_prefixes_and_secure_cookies_left_alone() {
        let names = |jar: &CookieJar| jar.cookies().into_iter().map(|c| c.name).collect::<Vec<_>>();
        let jar = CookieJar::new();
        let at = "https://www.a.test/x/y";
        jar.store(at, &format!("big={}", "v".repeat(MAX_PAIR)));
        jar.store(at, &format!("fits={}", "v".repeat(MAX_PAIR - 4)));
        // An attribute too long is ignored, not the cookie.
        jar.store(at, &format!("attr=1; Path=/{}", "p".repeat(MAX_ATTRIBUTE)));
        jar.store(at, "ctl=1\u{1}");
        assert_eq!(names(&jar), ["fits", "attr"]);
        assert_eq!(jar.cookies().get(1).map(|c| c.path.clone()).as_deref(), Some("/x"));

        let jar = CookieJar::new();
        jar.store(at, "__Secure-no=1");
        jar.store(at, "__secure-yes=1; Secure");
        jar.store(at, "__Host-nopath=1; Secure");
        jar.store(at, "__Host-domain=1; Secure; Path=/; Domain=a.test");
        jar.store(at, "__Host-plain=1; Path=/");
        jar.store(at, "__Host-yes=1; Secure; Path=/");
        jar.set("http://www.a.test/", "__Host-set", "1");
        jar.set(at, "__Host-set2", "1");
        assert_eq!(names(&jar), ["__secure-yes", "__Host-yes", "__Host-set2"]);
        assert!(jar.cookies().iter().all(|c| c.secure));

        // Over plain http neither the same cookie nor one that would shadow it is taken.
        let jar = CookieJar::new();
        jar.store("https://www.a.test/", "sid=good; Secure; Path=/; Domain=a.test");
        jar.store("http://www.a.test/", "sid=evil; Path=/; Domain=a.test");
        jar.store("http://www.a.test/", "sid=evil; Path=/deep");
        jar.store("http://www.a.test/", "sid=; Max-Age=0; Path=/; Domain=a.test");
        jar.set("http://www.a.test/", "sid", "evil");
        jar.store("http://www.a.test/", "other=1");
        assert_eq!(jar.header("https://www.a.test/deep/x").as_deref(), Some("sid=good; other=1"));
        // A secure origin replaces it as before.
        jar.store("https://www.a.test/", "sid=new; Path=/; Domain=a.test");
        assert_eq!(jar.get("http://www.a.test/", "sid").as_deref(), Some("new"));
    }

    #[test]
    fn expiry() {
        let jar = CookieJar::new();
        jar.store("https://a.test/", "old=1; Expires=Thu, 01 Jan 1970 00:00:01 GMT");
        jar.store("https://a.test/", "new=1; Expires=Fri, 01 Jan 2100 00:00:00 GMT");
        jar.store("https://a.test/", "short=1; Max-Age=3600; Expires=Thu, 01 Jan 1970 00:00:01 GMT");
        assert_eq!(
            jar.header("https://a.test/").as_deref(),
            Some("new=1; short=1"),
            "Max-Age wins over Expires"
        );
        let short = jar.cookies().into_iter().find(|c| c.name == "short").and_then(|c| c.expires);
        assert!(short.is_some_and(|e| (e - now() - 3600).abs() < 5));
        jar.store("https://a.test/", "new=; Max-Age=0");
        assert_eq!(jar.header("https://a.test/").as_deref(), Some("short=1"));
        // Replacing keeps its place.
        jar.store("https://a.test/", "a=1");
        jar.store("https://a.test/", "b=1");
        jar.store("https://a.test/", "a=2");
        assert_eq!(jar.header("https://a.test/").as_deref(), Some("short=1; a=2; b=1"));
    }
}
