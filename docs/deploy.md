# Running mise on a home server

mise runs as one Docker Compose stack: Postgres, the app (API, web app and the `mise` admin command in one
image), and a nightly database backup. These steps use a Proxmox LXC; any Linux host with Docker works the same.

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
The app applies database migrations whenever it starts.

## 3. Photos

Photos live in `MEDIA_DIR` (default `./data/media` next to `compose.yaml`, on the container's own disk). A full
HelloFresh-sized library takes about 40 GB of originals plus 9 GB of web-size copies.

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
copied rather than hard-linked: allow about 40 GB.

```bash
docker compose exec app mise ingest-media --from /archive
```

The curation and dish decision files sit next to the bundle (see [curation.md](curation.md)). A curation file is all or
nothing, so it may only name foods the bundle contains.

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
nothing, so it may only name foods the bundle contains.

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

## 7. Reaching it from outside

By default mise answers on the home network only. To publish it through an existing reverse proxy (Caddy,
nginx, Traefik, Nginx Proxy Manager), proxy to port 8080, set `PUBLIC_BASE_URL=https://mise.example.com`, and,
when the proxy runs on the same host, `MISE_LISTEN=127.0.0.1`. With an https `PUBLIC_BASE_URL`, session cookies
are marked Secure. Every page needs a signed-in account; only `/api/health` is open.

## Commands

The `mise` admin command is in the image: `docker compose exec app mise --help`. Account recovery is
`docker compose exec -T app mise user set-password <name>` with the new password on standard input.
