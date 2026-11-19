// The app's service worker: the build's files are cached when it installs, served from the
// cache first (the app starts offline), and caches of older builds are removed when it
// activates. `cargo remedy package --target web` fills in the build's version and file list.
const CACHE = @CACHE@;
const FILES = @FILES@;

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith(@PREFIX@) && k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  if (event.request.method !== 'GET') return;
  event.respondWith(caches.match(event.request).then((hit) => hit || fetch(event.request)));
});
