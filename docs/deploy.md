# Running mise on a home server

mise runs as one Docker Compose stack: Postgres, the app (API, web app and the `mise` admin command in one
image), a nightly database backup and, for HTTPS, an optional Caddy. These steps use a Proxmox LXC; any Linux host
with Docker works the same.

## 1. The container

Create a Debian or Ubuntu LXC (2 cores, 2 GB RAM and 20 GB disk are plenty; photos need more, see below). Docker
inside an LXC needs nesting: in Proxmox, **Options → Features**, tick **nesting** (and **keyctl** for an
unprivileged container). Then install Docker inside it:

```bash
curl -fsSL https://get.docker.com | sh
```

## 2. Start mise

```bash
git clone <your mise repository> /opt/mise && cd /opt/mise
cp deploy.env.example .env
sed -i "s|^POSTGRES_PASSWORD=.*|POSTGRES_PASSWORD=$(openssl rand -base64 30 | tr -d '/+=')|" .env
docker compose up -d --build
```

Open `http://<container address>:8080`. The first visit creates the admin account (see [sign-in.md](sign-in.md)).
The app applies database migrations whenever it starts. Phones need HTTPS to keep the shopping list offline; section
7 sets it up.

## 3. Photos

Photos live in `MEDIA_DIR` (default `./data/media` next to `compose.yaml`, on the container's own disk). A library of
about 10,000 meal-kit dishes takes about 40 GB of originals plus 9 GB of web-size copies.

- **Another disk or a NAS:** mount it on the Proxmox host and pass it into the LXC as a mount point
  (`pct set <id> -mp0 /mnt/nas/mise,mp=/srv/mise-media`), then set `MEDIA_DIR=/srv/mise-media`. NFS and SMB
  mounts usually can't be made from inside an unprivileged LXC, which is why the host mounts them.
- **Permissions:** the app runs as uid 10001. Give it the folder (`chown -R 10001:10001 <MEDIA_DIR>`), or set
  `MISE_UID`/`MISE_GID` to the folder's owner. In an unprivileged LXC, uid 10001 inside is 110001 on the host.

## 4. Bring an existing library

Moving a database you curated elsewhere (dishes, food tree, decisions) keeps all of that work:

```bash
# Where the library is now (the development database):
docker exec mise-db-1 pg_dump -U mise -d mise --format=custom > mise.dump
# On the server, with the app stopped:
docker compose stop app
docker compose exec -T db pg_restore -U mise -d mise --clean --if-exists --no-owner < mise.dump
docker compose start app
```

Then copy the photo folder (`MEDIA_ROOT` there) into `MEDIA_DIR` here; paths in the database are relative to it,
so nothing else changes. Alternatively, copy the recipe archive instead, set `ARCHIVE_DIR` to it, and rebuild the
photos on the server. Inside the container the archive and the media folder are separate mounts, so photos are
copied rather than hard-linked: allow about 40 GB for the full archive (the launch seed needs far less).

```bash
docker compose exec app mise ingest-media --from /archive
```

Starting from a seed bundle instead of a database:

```bash
docker compose exec app mise import-bundle /archive/seed-bundle
docker compose exec app mise match-foods
docker compose exec app mise food apply /archive/curation.json
docker compose exec app mise dish apply /archive/dish-decisions.json
docker compose exec app mise dish prefer /archive/preferred-versions.json
docker compose exec app mise group-dishes
docker compose exec app mise derive-options
docker compose exec app mise parse-steps
docker compose exec app mise ingest-media --from /archive
```

The curation and dish decision files sit next to the bundle (see [curation.md](curation.md)). A curation file is all or
nothing, so it may only name foods the bundle contains: for the launch seed, copy `seed/mise-seed`,
`curation-seed.json`, `dish-decisions-seed.json` and `preferred-versions.json` into the archive folder and use those
names in the commands above.

## 5. Updating

```bash
git pull && docker compose up -d --build
```

## 6. Backups

The `backup` service writes `BACKUP_DIR/mise-<date>.dump` every night at `BACKUP_HOUR` (default 03:00 in the
container's time zone) and keeps `BACKUP_DAYS` (default 14). Copy that folder off the machine with the rest of your
backups; photos are best backed up from `MEDIA_DIR` directly. To restore:

```bash
docker compose stop app
docker compose exec -T db pg_restore -U mise -d mise --clean --if-exists --no-owner < backups/mise-2026-10-27.dump
docker compose start app
```

## 7. HTTPS

A phone opens the shopping list offline only from an HTTPS address: the service worker that keeps it needs a secure
origin. Kitchen mode's screen wake lock and an Alexa endpoint need HTTPS too. There are two ways to get it:

| Way | When |
| --- | --- |
| [Behind a reverse proxy you already run](#behind-an-existing-reverse-proxy) | Caddy, nginx, Traefik or Nginx Proxy Manager already answers on ports 80 and 443 |
| [The stack's own Caddy](#the-stacks-own-caddy) | Nothing else does: the `https` profile adds a Caddy in front of mise |

Either way, set `PUBLIC_BASE_URL` to the `https` address, which marks session cookies Secure, and check the result
([below](#check-it)). Every page needs a signed-in account; only `/api/health` is open. Make the first account before
the address can be reached from the internet: until then, whoever opens it first becomes the admin.

### Behind an existing reverse proxy

In `.env` on the mise host, then `docker compose up -d` to apply it:

```bash
PUBLIC_BASE_URL=https://mise.example.com
# Only when the proxy runs on this host: then nothing else can reach port 8080.
MISE_LISTEN=127.0.0.1
```

A Caddy site block. Caddy gets the certificate and redirects plain HTTP itself:

```caddyfile
mise.example.com {
	reverse_proxy 127.0.0.1:8080
}
```

With Caddy on another machine, proxy to `<mise host>:8080` instead and leave `MISE_LISTEN` at its default; a firewall
rule that lets only the proxy reach port 8080 does what `127.0.0.1` would.

- **nginx:** a `server` block for `mise.example.com` with your certificate and `location / { proxy_pass
  http://127.0.0.1:8080; proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme; }`, plus a port
  80 server that answers `return 301 https://$host$request_uri;`.
- **Traefik:** a router with ``rule: Host(`mise.example.com`)``, your certificate resolver and the `websecure`
  entry point, whose service points at `http://<mise host>:8080`, with the `web` entry point redirecting to
  `websecure`.
- **Nginx Proxy Manager:** a proxy host for `mise.example.com` forwarding to `http://<mise host>:8080`, with an SSL
  certificate and **Force SSL** on.

mise sends no HSTS header. Once HTTPS works for good, the proxy can add one, for example Caddy's
`header Strict-Transport-Security "max-age=31536000"`. Browsers then refuse plain HTTP to that name for a year, so
add it last.

### The stack's own Caddy

The `caddy` service in `compose.yaml` runs only with the `https` profile. It takes ports 80 and 443 on the host,
gets a certificate, redirects plain HTTP to HTTPS and proxies to the app. In `.env`:

```bash
COMPOSE_PROFILES=https        # so every docker compose command includes Caddy
MISE_HOST=mise.example.com    # the name phones open
MISE_TLS=internal             # or a contact email, see below
PUBLIC_BASE_URL=https://mise.example.com
MISE_LISTEN=127.0.0.1         # the app is reached through Caddy only
```

Then `docker compose up -d`. `MISE_HOST` must resolve to the server for every device that opens it: on a home
network, add it to the router's DNS (or Pi-hole, AdGuard Home), for example as `mise.home.arpa`.

`MISE_TLS` chooses the certificate:

| `MISE_TLS` | Certificate | Needs |
| --- | --- | --- |
| `internal` | From Caddy's own certificate authority, renewed by Caddy | Nothing from the internet; each phone and computer trusts Caddy's root once (below) |
| A contact email, `you@example.com` | A public one from Let's Encrypt (ZeroSSL as a fallback), which every device trusts | `MISE_HOST` in public DNS pointing at this host, and ports 80 and 443 forwarded to it from the internet |

#### Trusting Caddy's root certificate

With `MISE_TLS=internal`, a phone won't trust the certificate until it trusts Caddy's root, and until then it won't
keep the list offline. Copy the root out of the container:

```bash
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt mise-root.crt
```

Send `mise-root.crt` to each device (AirDrop, email or a cable), then:

- **iPhone and iPad:** open the file and allow the profile. In Settings, **General → VPN & Device Management**,
  install it, then in **General → About → Certificate Trust Settings** turn on full trust for "Caddy Local
  Authority".
- **Android:** **Settings → Security and privacy → More security settings → Encryption and credentials → Install a
  certificate → CA certificate** (the path varies by maker). Chrome and Samsung Internet use it; Firefox only with
  its "Use third party CA certificates" setting on.
- **Computers:** add it to the system's trusted roots (macOS: Keychain Access, System, "Always Trust"; Windows:
  `certlm.msc`, Trusted Root Certification Authorities). Firefox keeps its own list: Settings, Certificates, Import.

The root lives in the `caddy_data` volume and lasts ten years. Keep that volume: a new one makes a new root, and
every device has to trust it again.

### Check it

With `mise.example.com` standing for your address (add `--cacert mise-root.crt` to `curl` with `MISE_TLS=internal`):

1. `curl -sI http://mise.example.com/` answers with a redirect (301 or 308) to `https://mise.example.com/`.
2. `curl -s https://mise.example.com/api/health` answers `{"status":"ok",...}` without a certificate error.
3. Sign in from a computer's browser and open the developer tools (Application, Cookies): `mise_session` is
   **Secure** and **HttpOnly**. If it isn't Secure, `PUBLIC_BASE_URL` doesn't start with `https://`, or the app wasn't
   restarted since it changed (`docker compose up -d`).
4. In the same tools, **Application → Service workers** shows `/sw.js` activated. On the phone, open the shopping
   list once, then turn on airplane mode and open it again: it's still there.

CI runs the first three against the stack's own Caddy with `MISE_TLS=internal` whenever `compose.yaml` or the
`Caddyfile` changes (the `docker image` job in `.gitlab-ci.yml`).

## Commands

The `mise` admin command is in the image: `docker compose exec app mise --help`. Account recovery is
`docker compose exec -T app mise user set-password <name>` with the new password on standard input.
