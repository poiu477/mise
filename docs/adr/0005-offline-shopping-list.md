# 0005 · The shopping list works offline from a device copy and queued changes

Accepted, 2026-11-03 (status.md's Decisions; commits "Keep the shopping list working with no signal" and "Keep an
open shopping list saving after another tab signs in"). The alternatives below are not recorded from the time;
they are weighed against what the code does.

## Context

The list is used in a shop, often with no signal. The server never stores the list: it works it out from the
week's plan on each request and lays the stored ticks, removals and added items on top
([architecture.md](../architecture.md#the-api-mise-api)). Several people in a household share a list, and a
device can be shared too: the session cookie is the same in every tab, so someone can sign in as another account
in one tab while a list is open in another.

## Decision

- **A device copy per account** ([offline.ts](../../web/src/offline.ts)): each week's list as last seen, in
  `localStorage`. Without the server the page shows that copy.
- **Changes queued as small intents**, not a whole list: tick a line, tick an added item, add an item. They are
  sent in order once the server answers; a later tick of the same line replaces the earlier one, and an item added
  offline keeps a `local-…` id until the server gives it one, so the device queues it once and ticks it in place.
  Removing, staples and units need the server and wait for it.
- **Work stays with the sign-in that made it.** Each sign-in gets an id on the device; a copy only saves while that
  sign-in lasts, so an answer that arrives after signing out can't put the list back. Signing out clears everything
  under `mise:`.
- **The server refuses work meant for someone else.** The app sends `X-Mise-Account` with every API request except
  asking who is signed in (`/api/me`) and signing in (`/api/auth/…`); when it isn't the signed-in account, `auth::require_auth` answers 412 ([auth/mod.rs](../../crates/mise-api/src/auth/mod.rs)).
  The list treats 412 like no signal: the change keeps waiting for its own account, and the app checks again who
  is signed in. Scripts and integrations can leave the header out.

## Alternatives

- **Let the service worker cache the list's API answers.** [sw.js](../../web/public/sw.js) already does this for
  recipes. It gives a read-only list offline, but ticks made without signal would be lost, and account data is
  deliberately never cached there.
- **Background Sync to send the queue.** Only Chromium browsers have it, so phones on Safari or Firefox would still
  need the app's own queue; the app sends the queue itself when it next reaches the server.
- **Send the whole list back and let the last copy win.** Simpler, but the list is computed from the plan, so a
  stale copy would overwrite ticks made on another phone and undo plan changes. Small intents merge on the server.
- **A sync engine or CRDTs.** They solve concurrent edits in general; the list's changes are ticks and additions
  that already merge without conflict, so the extra library and storage format buy nothing here.
- **Drop the queue whenever the account changes, without the 412 guard.** Safe, but a tick made offline in a shop
  is lost because someone signed in at home. The header lets the change wait instead of vanishing or landing in
  another household's list.

## Consequences

- The browser journeys cover it: ticking and adding offline, opening with no connection, sending once back online,
  a second tab starting a new sign-in, and sign-out leaving nothing on the device (`web/e2e/journeys.spec.ts`).
- Opening the app with no signal needs a service worker, and phones allow those only over HTTPS; the reverse
  proxy for that is still an open question in status.md.
- Sending is at least once, not exactly once. Adding an item has no idempotency key: if the server saves it and
  the answer is lost on the way back, the device sends it again and the list shows it twice (status.md's known
  issues). Ticks are safe to repeat, since a tick sets a state rather than toggling it.
- `localStorage` can be missing or full; the list then simply isn't kept, and the app still works online.
- Every signed-in route can answer 412, which the API document doesn't list everywhere yet
  ([0004](0004-generated-api-client.md)).
