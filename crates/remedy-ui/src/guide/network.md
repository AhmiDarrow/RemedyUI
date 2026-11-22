# Talking to a server

The `net` feature brings [`remedy_ui::net`](crate::net): one HTTP client for native apps and
browser pages. Natively it is ureq with rustls and the platform's certificate verifier; in a
page it is the browser's `fetch`. Nothing in it blocks the UI thread, so the same code runs in
both.

## Requests and errors

An [`Http`](crate::net::Http) holds a base address, headers sent with every request, a
timeout (30 seconds unless set) and, natively, an optional
[`CookieJar`](crate::net::CookieJar). A [`Request`](crate::net::Request) is a method, a path
and a body: JSON, raw bytes or a [`Multipart`](crate::net::Multipart) form.
[`Http::send_json`](crate::net::Http::send_json) decodes the answer into your type.

A status outside 200-299 comes back as [`NetError::Http`](crate::net::NetError::Http): the
status, the body, and the `detail` field of a JSON body (`{"detail": "pair this device
first"}`), so an app can show the server's own words or react to a `401`.

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
  (bodies up to 64 KiB), or natively a request that `run` waits for (up to 10 seconds) as the
  app quits. Cancelling does not stop it.
- [`Request::timeout`](crate::net::Request::timeout) overrides the client's timeout for one
  request; running out ends it with `NetError::Timeout`.

## Where it may connect

The client contacts only the addresses the app names. Natively it refuses plain `http`
beyond the loopback interface. In a page, a relative path (`Http::new("")` or `"/api"`) goes to
the page's own origin with the page's cookies (`credentials: same-origin`), and the browser's
rules (CORS, mixed content) apply on top.
