# 0004 · React, with a client generated from the server's OpenAPI document

Accepted, 2026-10, as part of "Build our own app (Rust, Postgres, React)" in status.md's Decisions. The design
document names the parts (React, Vite, a client generated from the OpenAPI spec, with openapi-fetch and
openapi-typescript reused rather than rebuilt) but not the alternatives, so this record weighs them against the code.

## Context

The web app is a PWA used mostly on phones: search, dish pages with toggles and servings, the plan, and a shopping
list that has to work with no signal ([0005](0005-offline-shopping-list.md)). It talks only to mise's REST API,
which scripts and integrations use too, with API tokens. The server is Rust and the app TypeScript, so the shapes of
requests and answers have to be kept the same on both sides somehow.

## Decision

- **React 19 with React Router, Vite and TypeScript**; function components and hooks, one stylesheet, no UI or
  state library; server data goes through TanStack Query ([style.md](../style.md#typescript-and-react)).
- **The server is the source of the API's types.** utoipa builds the OpenAPI 3.1 document from annotations on the
  handlers and their Rust types; `mise-server openapi` prints it and the server serves it at `/api/openapi.json`.
- **openapi-typescript** turns that document into `web/src/api/schema.d.ts`, and **openapi-fetch** is the client
  ([client.ts](../../web/src/api/client.ts)): typed paths, bodies and answers, with only a small `fetch` wrapper at
  run time. Its middleware sends `X-Mise-Account` and reacts to 401 and 412.
- **CI keeps the two in step.** It regenerates the document and the types and fails if `schema.d.ts` differs from
  what is committed, after [check-openapi.mjs](../../web/scripts/check-openapi.mjs) checks every response has a
  description.

## Alternatives

- **Types written by hand on both sides.** No tooling, but nothing notices when they drift; a renamed field becomes
  a blank on a page at run time instead of a failed build.
- **A full client generator (openapi-generator, orval).** These generate classes or hooks per endpoint.
  openapi-fetch does the same job with less: types only, one thin client, no generated code to review.
- **GraphQL, or tRPC.** GraphQL adds a schema language and server library for an API whose clients are one web app
  and a few scripts; tRPC needs a TypeScript server. Plain REST with OpenAPI also serves scripts as they are.
- **Server-rendered pages (htmx or templates in Rust).** Less JavaScript, but the offline list needs logic on the
  device: its own copy, a queue of changes and replaying them in order. That needs a client app either way.

## Consequences

- Changing the API means regenerating the types (`npm run api`, which needs a release build, or the debug-build
  route in AGENTS.md), and committing them with the change; CI fails otherwise.
- The document lists only the responses the annotations name. Signed-in routes can all return 401 and 412, and not
  every one says so yet; status.md has the task to list them and test that they match.
- Pages load data through TanStack Query calling this client ([query.ts](../../web/src/api/query.ts)), never by
  setting state in an effect; ESLint's `set-state-in-effect` rule is an error.
