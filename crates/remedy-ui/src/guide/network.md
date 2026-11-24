# Talking to a server

The `net` feature brings [`remedy_ui::net`](crate::net): one HTTP client for native apps and
browser pages. Natively it is ureq with rustls and the platform's certificate verifier; in a
page it is the browser's `fetch`. Nothing in it blocks the UI thread, so the same code runs in
both.

## Requests and errors

An [`Http`](crate::net::Http) holds a base address, headers sent with every request, its
limits and, natively, an optional [`CookieJar`](crate::net::CookieJar). Every limit has a
default and can be changed, or lifted with `None`:

| Limit | Default | Set with |
|---|---|---|
| Time for the whole request | 30 seconds | `Http::timeout`, `Request::timeout` |
| Body read, kept or streamed | 256 MiB | `Http::max_body`, `Request::max_body` |
| Redirects followed (natively) | 10 | `Http::max_redirects` (`0` follows none) |
| Cookies a jar keeps | 3000 | `CookieJar::max_cookies` |

A [`Request`](crate::net::Request) is a method, a path
and a body: JSON, raw bytes or a [`Multipart`](crate::net::Multipart) form.
[`Http::send_json`](crate::net::Http::send_json) decodes the answer into your type.

A status outside 200-299 comes back as [`NetError::Http`](crate::net::NetError::Http): the
status, the headers, the body, and the `detail` field of a JSON body (`{"detail": "pair this
device first"}`), so an app can show the server's own words or react to a `401`. A
`304 Not Modified` answering a conditional request (`If-None-Match`, `If-Modified-Since`)
comes back the same way, with its new `ETag`, `Last-Modified` and `Cache-Control` in
`HttpError::header`.

```no_run
use remedy_ui::net::{Http, NetError, Request};
use remedy_ui::reactive::Cancel;

#[derive(serde::Deserialize)]
struct Saved {
    id: u32,
}

let http = Http::new("https://example.com/api").header("Authorization", "Bearer …");
http.send_json::<Saved>(
    Request::post("notes").json(&serde_json::json!({ "text": "hi" })),
    &Cancel::new(),
    |out| match out {
        Ok(saved) => println!("saved as {}", saved.id),
        Err(NetError::Http(e)) if e.status == 401 => println!("pair again"),
        Err(e) => println!("not saved: {e}"),
    },
);
```

The callback runs where the request ended: on a thread of its own natively, between frames
in a page. To change signals from it, post back to the UI thread
([`waker`](crate::reactive::waker)), or use a resource, which does that for you.

## Fetching for a key

[`Http::resource`](crate::net::Http::resource) is a [`Resource`](crate::reactive::Resource)
whose fetch is a request: when its key changes, the request under way is aborted (the
browser's `AbortController`; natively the request is let go and its answer dropped) and the
new one starts. Its `loading`, `get` and `error` are tracked like any resource's, so
`suspense` boundaries and bindings follow them.

```no_run
use remedy_ui::net::{Http, NetError, Request};
use remedy_ui::reactive::{Resource, Signal};

let page = Signal::new(1u32);
let http = Http::new("https://example.com/api");
let items: Resource<Vec<String>, NetError> =
    http.resource(move || page.get(), |p| Request::get(format!("items?page={p}")));
```

## Uploads, last saves and timeouts

- `Request::post("upload").multipart(&Multipart::new().text("title", "a").file("file", "a.png",
  "image/png", bytes))` sends a form as a browser would.
- [`Request::keepalive`](crate::net::Request::keepalive) marks a request that must finish while
  the page or app closes, such as the last save of a note: the browser's `fetch` keepalive
  (bodies up to 64 KiB), or natively a request that `run` waits for (up to 10 seconds, or
  [`AppConfig::keepalive_grace`](crate::AppConfig::keepalive_grace)) as the app quits. Cancelling does not stop it.
- [`Request::timeout`](crate::net::Request::timeout) overrides the client's timeout for one
  request; running out ends it with `NetError::Timeout`.
- On Android a keepalive request also finishes when the app goes to the background: while
  one is under way, the bridge's `Keepalive` service keeps the process from being frozen
  (`cargo remedy package` declares it), and quitting waits for it as on the desktop.

## Streaming a body

[`Http::stream`](crate::net::Http::stream) hands the body to a callback chunk by chunk as it
arrives, instead of keeping it: for downloads and long answers. Each chunk comes with the
answer's status and headers; the last call gets the `Response` (with an empty body) or the
error, and no chunk comes after it. Cancelling stops it between chunks and closes the
connection; a body past the request's `max_body` ends it with `NetError::TooLarge`. In a page
it reads the `fetch` response's `ReadableStream`.

```no_run
use remedy_ui::net::{Http, Request};
use remedy_ui::reactive::Cancel;

let mut got = 0usize;
Http::new("https://example.com").stream(
    Request::get("big.bin").max_body(2 << 30),
    &Cancel::new(),
    move |_head, bytes| got += bytes.len(),
    |out| println!("done: {:?}", out.map(|r| r.status)),
);
```

## Cookies

Natively a [`CookieJar`](crate::net::CookieJar) keeps what servers set as RFC 6265 says:
host-only cookies and `Domain` ones (for subdomains), `Path`, `Secure` (sent only over
`https` or to the loopback interface), `HttpOnly` (sent, since the app is the HTTP client),
`Expires` and `Max-Age`, and `SameSite` kept (a native app makes no cross-site requests).
Redirects are followed hop by hop, so each hop sends its own address's cookies and keeps
what it sets. `CookieJar::cookies` lists what the jar holds. In a page the browser keeps the
page's cookies.

## Where it may connect

The client contacts only the addresses the app names. Natively it refuses plain `http`
beyond the loopback interface, unless the client is set to
[`HttpPolicy::PrivateNetworks`](crate::net::HttpPolicy::PrivateNetworks): then plain `http`
may also go to the home or office network (RFC 1918 and link-local IPv4, unique-local and
link-local IPv6, and `.local` names), for a server such as a Portal box on the same network:

```no_run
use remedy_ui::net::{Http, HttpPolicy};

let portal = Http::new("http://192.168.1.20:8000/api").policy(HttpPolicy::PrivateNetworks);
```

The address a name resolves to is what is checked, just before connecting, so a name that
resolves somewhere else (DNS rebinding) is refused; plain `http` never goes through a proxy;
every redirect is checked the same way. In a page, a relative path (`Http::new("")` or `"/api"`) goes to
the page's own origin with the page's cookies (`credentials: same-origin`), and the browser's
rules (CORS, mixed content) apply on top.

## Limits

Every bound the client works within has a default and a public way to change it.

| Limit | Default | How to change it |
| --- | --- | --- |
| Longest body read | 256 MiB | `Http::max_body`, `Request::max_body` |
| How long a request may take | 30 s | `Http::timeout`, `Request::timeout` |
| Redirects followed (natively) | 10 | `Http::max_redirects` |
| Cookies a jar keeps | 3000 | `CookieJar::max_cookies` |
| Idle connections kept for reuse, in all / to one host (natively) | 10 / 3 | `net::PoolLimits::idle_connections`, `idle_per_host` |
| How long an idle connection is kept (natively) | 15 s | `net::PoolLimits::idle_age` |
| How long quitting waits for keepalive requests (natively, and on Android when the app goes away) | 10 s | `AppConfig::keepalive_grace` |

`PoolLimits` holds for the whole process; pass it to `net::set_pool_limits`, and requests
from then on use it (connections idle in the old pool are closed). In a page the browser
pools connections itself.

```no_run
use std::time::Duration;
use remedy_ui::net::{PoolLimits, set_pool_limits};

// An app that talks to one API server in bursts.
set_pool_limits(PoolLimits::default().idle_per_host(8).idle_age(Duration::from_secs(60)));
```

The keepalive body limit in a page (64 KiB) is the browser's own, not RemedyUI's.
