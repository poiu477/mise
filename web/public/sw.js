// mise service worker: keeps the app shell, recipes and photos available when the network drops mid-recipe.
// Photos are content-addressed (a media id never changes), so they are served from cache first; pages and
// recipe data always try the network and fall back to the last copy. Account data (/api/me, tokens, people)
// is never cached, so nothing personal outlives signing out on a shared device.
const SHELL = "mise-shell-v2";
const PHOTOS = "mise-photos-v1";
const RECIPE_DATA = ["/api/dishes", "/api/variants/"];

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
  // Account and sign-in requests go straight to the network, never through a cache.
  if (url.pathname.startsWith("/api/") && !url.pathname.startsWith("/api/media/") && !RECIPE_DATA.some((p) => url.pathname.startsWith(p))) return;

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
        const recipe = RECIPE_DATA.some((p) => url.pathname.startsWith(p));
        if (res.ok && (req.mode === "navigate" || url.pathname.startsWith("/assets/") || recipe)) {
          const copy = res.clone();
          caches.open(SHELL).then((c) => c.put(req.mode === "navigate" ? "/" : req, copy));
        }
        return res;
      })
      .catch(async () => (await caches.match(req.mode === "navigate" ? "/" : req)) || Response.error()),
  );
});
