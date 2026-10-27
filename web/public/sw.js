// mise service worker: keeps the app shell and photos available when the network drops mid-recipe.
// Photos are content-addressed (a media id never changes), so they are served from cache first; pages and
// API data always try the network and fall back to the last copy.
const SHELL = "mise-shell-v1";
const PHOTOS = "mise-photos-v1";

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(SHELL).then((c) => c.addAll(["/", "/manifest.webmanifest", "/icon.svg"])));
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== SHELL && k !== PHOTOS).map((k) => caches.delete(k)))),
  );
  self.clients.claim();
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;

  if (url.pathname.startsWith("/api/media/")) {
    event.respondWith(
      caches.open(PHOTOS).then(async (c) => {
        const hit = await c.match(req);
        if (hit) return hit;
        const res = await fetch(req);
        if (res.ok) c.put(req, res.clone());
        return res;
      }),
    );
    return;
  }

  event.respondWith(
    fetch(req)
      .then((res) => {
        if (res.ok && (req.mode === "navigate" || url.pathname.startsWith("/assets/") || url.pathname.startsWith("/api/"))) {
          const copy = res.clone();
          caches.open(SHELL).then((c) => c.put(req.mode === "navigate" ? "/" : req, copy));
        }
        return res;
      })
      .catch(async () => (await caches.match(req.mode === "navigate" ? "/" : req)) || Response.error()),
  );
});
