// The app's service worker: the build's files are cached when it installs, served from the
// cache first (the app starts offline), and caches of older builds are removed when it
// activates. The files the app fetches only when it needs them (fallback fonts) are cached
// as they are first fetched, and in the background once the worker runs, so they are there
// offline without holding up the first start. `cargo remedy package --target web` fills in
// the build's version and file lists.
const CACHE = @CACHE@;
const FILES = @FILES@;
const LATER = @LATER@;

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith(@PREFIX@) && k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
  // Not waited for: one at a time, each kept once it is in.
  caches.open(CACHE).then(async (c) => {
    for (const f of LATER) {
      if (!(await c.match(f))) await c.add(f).catch(() => {});
    }
  });
});

const LATER_URLS = new Set(LATER.map((f) => new URL(f, self.location).href));

self.addEventListener('fetch', (event) => {
  if (event.request.method !== 'GET') return;
  event.respondWith(caches.match(event.request).then((hit) => {
    if (hit) return hit;
    return fetch(event.request).then((response) => {
      if (response.ok && LATER_URLS.has(event.request.url)) {
        const copy = response.clone();
        caches.open(CACHE).then((c) => c.put(event.request, copy));
      }
      return response;
    });
  }));
});
