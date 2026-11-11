// mise service worker: keeps the app shell, recipes and photos available when the network drops mid-recipe.
// Photos are content-addressed (a media id never changes), so they are served from cache first; pages and
// recipe data always try the network and fall back to the last copy. Account data (/api/v1/me, tokens, people,
// the plan and the list) is never cached here; the shopping list keeps its own copy in the app, cleared on
// signing out.
//
// The API moved from /api to /api/v1, and the old addresses answer for one more release. This worker takes over
// at once (skipWaiting, clients.claim), so a page still running the app from before calls the old addresses
// through it: both read the same way, and a copy kept under one address serves the other, so updating loses no
// recipe kept offline. When the old addresses go (docs/deploy.md), drop them here and move to mise-shell-v4,
// which clears the copies kept under them.
const SHELL = "mise-shell-v3";
const PHOTOS = "mise-photos-v1";
const API = "/api/v1";
const OLD_API = "/api";
/** Recipe data, after the API's address: kept for offline use. Nothing else under the API is. */
const RECIPE_DATA = ["/dishes", "/variants/"];
const MEDIA = "/media/";

/** The part of an API address after /api/v1 or the old /api: "/dishes" for both "/api/v1/dishes" and
 * "/api/dishes"; null outside the API. */
function apiPath(pathname) {
  for (const prefix of [API, OLD_API]) {
    if (pathname === prefix) return "";
    if (pathname.startsWith(prefix + "/")) return pathname.slice(prefix.length);
  }
  return null;
}

/** The same API request at its other address, old or new, query included: a copy cached under either serves. */
function otherAddress(url, api) {
  const prefix = url.pathname.startsWith(API + "/") ? OLD_API : API;
  return prefix + api + url.search;
}

// The page and the scripts and styles it loads, so the app opens offline from the first visit on.
self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(SHELL).then(async (c) => {
      await c.addAll(["/", "/manifest.webmanifest", "/icon.svg"]);
      const page = await (await c.match("/")).text();
      await c.addAll([...new Set(page.match(/\/assets\/[^"']+/g) ?? [])]);
    }),
  );
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((k) => k !== SHELL && k !== PHOTOS).map((k) => caches.delete(k))),
      ),
  );
  self.clients.claim();
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  const api = apiPath(url.pathname);
  const recipe = api !== null && RECIPE_DATA.some((p) => api.startsWith(p));
  const photo = api !== null && api.startsWith(MEDIA);
  // Account and sign-in requests go straight to the network, never through a cache.
  if (api !== null && !recipe && !photo) return;

  if (photo) {
    event.respondWith(
      caches.open(PHOTOS).then(async (c) => {
        const hit = (await c.match(req)) || (await c.match(otherAddress(url, api)));
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
        if (res.ok && (req.mode === "navigate" || url.pathname.startsWith("/assets/") || recipe)) {
          const copy = res.clone();
          caches.open(SHELL).then((c) => c.put(req.mode === "navigate" ? "/" : req, copy));
        }
        return res;
      })
      .catch(async () => {
        if (req.mode === "navigate") return (await caches.match("/")) || Response.error();
        const kept =
          (await caches.match(req)) || (recipe && (await caches.match(otherAddress(url, api))));
        return kept || Response.error();
      }),
  );
});
