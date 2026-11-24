# Security

mise is a self-hosted recipe app that a household runs for itself. It holds accounts, a household's own recipes and
plans, and the server can be asked to fetch web pages. A flaw that lets someone read or change what isn't theirs, or
reach what the server can reach, matters even on a home network.

## Reporting a problem

Please report a vulnerability privately, not in a public issue: use **Report a vulnerability** on the
[GitHub mirror's Security tab](https://github.com/poiu477/mise/security). Say what you found, how to reproduce it,
and what it lets someone do. Paul, who maintains mise, will answer there, and fix it on `main` before anything is
said in public; you'll be credited unless you'd rather not be.

Only the latest commit on `main` is supported: there are no maintained release branches.

## In scope

- The server (`mise-server`) and its API: signing in, sessions, API and kitchen tokens, single sign-on; one
  household reading or changing another's recipes, plans or lists, or a read-only token or kitchen device changing
  anything; requests another site can make a signed-in browser send.
- Importing: making the server fetch from somewhere it refuses (see "Importing from a link" in
  [docs/deploy.md](docs/deploy.md)), and files that make an import use far more memory, disk or time than its limits
  allow.
- Photos: a file that gets past the decoder, or is served to a household that shouldn't see it.
- The web app: script injection, or anything the Content-Security-Policy should stop and doesn't.
- The Docker image and `compose.yaml` as shipped.

## Out of scope

- Anything that needs an admin's account, the server's shell or its database: they hold the install already.
- `IMPORT_ALLOW_PRIVATE=true` letting signed-in people make the server fetch from the home network: that is what it
  does, and [docs/deploy.md](docs/deploy.md) says so.
- An install exposed to the internet without HTTPS, or with the reverse proxy set up differently from
  [docs/deploy.md](docs/deploy.md).
- Someone who knows a username keeping that account locked for a while by failing to sign in on purpose (see
  [docs/architecture.md](docs/architecture.md), "Security headers and requests from other sites").
- Denial of service by sheer volume of requests.

How the server protects itself, as the code enforces it, is in [docs/architecture.md](docs/architecture.md).
