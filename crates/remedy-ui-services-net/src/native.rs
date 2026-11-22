//! Requests natively: ureq with rustls on ring and the platform's certificate verifier, the
//! same stack as the updater. Each request runs on a thread of its own, so a caller that
//! cancels returns at once while the request is let go (it ends within its timeout).

use std::sync::mpsc;
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use remedy_ui_reactive::Cancel;
use ureq::RequestExt;

use super::{HttpError, NetError, Prepared, Response};

/// Largest body read into memory.
const MAX_BODY: u64 = 256 * 1024 * 1024;
/// How often a waiting caller looks at its cancel token.
const CANCEL_POLL: Duration = Duration::from_millis(20);

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        // ring is the TLS provider; installing it twice is harmless.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            // Plain http reaches only the loopback interface; `allowed` checks that first.
            .https_only(false)
            .user_agent("RemedyUI")
            .build()
            .into()
    })
}

fn net(e: ureq::Error) -> NetError {
    match e {
        ureq::Error::Timeout(_) => NetError::Timeout,
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::TimedOut => NetError::Timeout,
        e => NetError::Network(e.to_string()),
    }
}

/// Runs the request where it is called.
fn run(p: Prepared) -> Result<Response, NetError> {
    let mut b = ureq::http::Request::builder().method(p.method).uri(&p.url);
    for (k, v) in &p.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    if let Some(c) = p.cookies.as_ref().and_then(|j| j.header(&p.url)) {
        b = b.header("Cookie", c);
    }
    let started = Instant::now();
    let res = match p.body {
        Some(body) => b
            .body(body)
            .map_err(|e| NetError::Refused(e.to_string()))?
            .with_agent(agent())
            .configure()
            .timeout_global(Some(p.timeout))
            .build()
            .run(),
        None => b
            .body(())
            .map_err(|e| NetError::Refused(e.to_string()))?
            .with_agent(agent())
            .configure()
            .timeout_global(Some(p.timeout))
            .build()
            .run(),
    };
    let mut res = res.map_err(net)?;
    if let Some(jar) = &p.cookies {
        for v in res.headers().get_all("set-cookie") {
            if let Ok(v) = v.to_str() {
                jar.store(&p.url, v);
            }
        }
    }
    let status = res.status().as_u16();
    let headers = res
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    let body = res.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| {
        // A body that stalls past the deadline reads as a broken connection.
        if started.elapsed() >= p.timeout { NetError::Timeout } else { net(e) }
    })?;
    if (200..300).contains(&status) {
        Ok(Response { status, headers, body })
    } else {
        Err(NetError::Http(HttpError::new(status, &body)))
    }
}

/// Keepalive requests still running, for [`finish_keepalive`].
fn in_flight() -> &'static (Mutex<usize>, Condvar) {
    static IN_FLIGHT: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// Counts a keepalive request until it ends.
struct Kept;

impl Kept {
    fn new() -> Self {
        *in_flight().0.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Kept
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        let (n, cv) = in_flight();
        let mut n = n.lock().unwrap_or_else(PoisonError::into_inner);
        *n = n.saturating_sub(1);
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
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("remedy-net".into())
        .spawn(move || {
            let _kept = kept;
            let _ = tx.send(run(p));
        })
        .map_err(|e| NetError::Network(format!("no thread for the request: {e}")))?;
    loop {
        match rx.recv_timeout(CANCEL_POLL) {
            Ok(out) => return out,
            Err(mpsc::RecvTimeoutError::Timeout) if !keepalive && cancel.is_cancelled() => {
                return Err(NetError::Cancelled);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(NetError::Network("the request ended without an answer".into()));
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
