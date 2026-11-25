// Starts mise for the browser journeys: a fresh `mise_e2e` database on the dev Postgres (or the server in
// DATABASE_URL), the demo bundle imported and curated (plus one retired dish address and one question for the review
// queue), then mise-server on 127.0.0.1:8095 serving web/dist.
// A small site on 127.0.0.1:8096 serves the sample recipe pages (fixtures/web-pages, written for mise) for the
// import journey; mise-server may fetch from it because IMPORT_ALLOW_PRIVATE is on here, as it never is by default.
// Playwright runs this as its webServer and stops it when the tests end; each start drops and rebuilds the
// database, so every run begins from an empty install. Needs `cargo build -p mise-cli -p mise-api` and
// `npm run build` first.
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import pg from "pg";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const exe = process.platform === "win32" ? ".exe" : "";
const bin = (name) => path.join(root, "target", process.env.E2E_PROFILE ?? "debug", name + exe);
const server = process.env.DATABASE_URL ?? "postgres://mise:mise@localhost:5433/mise";
const name = "mise_e2e";
const target = new URL(server);
target.pathname = `/${name}`;
const url = target.toString();

const admin = new pg.Client({ connectionString: server });
await admin.connect();
await admin.query(`DROP DATABASE IF EXISTS ${name} WITH (FORCE)`);
await admin.query(`CREATE DATABASE ${name}`);
await admin.end();

const env = { ...process.env, DATABASE_URL: url, RUST_LOG: "warn" };
// Quiet unless something goes wrong: errors still reach the terminal.
const mise = (...args) =>
  execFileSync(bin("mise"), args, { env, stdio: ["ignore", "ignore", "inherit"], cwd: root });
mise("migrate");
mise("import-bundle", "fixtures/demo-bundle");
mise("match-foods");
mise("food", "apply", "fixtures/demo-bundle/curation.json");
mise("group-dishes");
mise("derive-options");
mise("parse-steps");

// An old address for one dish, as grouping leaves when a dish is renamed, for the journey that follows one.
const db = new pg.Client({ connectionString: url });
await db.connect();
await db.query(
  "INSERT INTO retired_slugs (slug, dish_id) SELECT 'tomato-pasta', id FROM dishes WHERE slug = 'weeknight-tomato-pasta'",
);
// A question for the review queue, as matching asks it of a name that could mean two foods. The demo recipes leave
// none of their own.
await db.query(
  `INSERT INTO review_items (id, kind, subject, proposal, confidence)
   SELECT gen_random_uuid(), 'food_match',
          jsonb_build_object('food_id', id, 'name', name, 'recipe_uses', 1, 'all_uses', 1),
          '{"note": "black pepper or bell pepper (as a pantry item it is usually black pepper)"}', 0.3
   FROM foods WHERE name = 'Black Pepper'`,
);
// A near-duplicate pair, as matching raises one when two names it doesn't know are the same food written twice.
// The demo recipes name each food one way, so the second spelling is made here.
await db.query(
  `WITH extra AS (
     INSERT INTO foods (id, name, slug, kind, origin)
     VALUES (gen_random_uuid(), 'Shredded Cheddar Cheese', 'shredded-cheddar-cheese', 'food', 'matched')
     RETURNING id, name
   )
   INSERT INTO review_items (id, kind, subject, proposal, confidence)
   SELECT gen_random_uuid(), 'food_duplicate',
          jsonb_build_object('food_id', f.id, 'name', f.name, 'recipe_uses', 1, 'all_uses', 1),
          jsonb_build_object('other_id', extra.id, 'other_name', extra.name, 'other_uses', 0), 0.83
   FROM foods f, extra WHERE f.name = 'Cheddar Cheese'`,
);
await db.end();

// The sample pages, by name: /web-pages/no-image.html. Nothing else.
const pages = path.join(root, "fixtures", "web-pages");
const site = http.createServer((req, res) => {
  const name = /^\/web-pages\/([a-z-]+\.html)$/.exec(req.url ?? "")?.[1];
  try {
    const body = readFileSync(path.join(pages, name ?? "missing"));
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" }).end(body);
  } catch {
    res.writeHead(404).end();
  }
});
site.listen(8096, "127.0.0.1");

const child = spawn(bin("mise-server"), [], {
  cwd: root,
  stdio: "inherit",
  env: {
    ...env,
    MISE_BIND: "127.0.0.1:8095",
    MISE_WEB_DIR: path.join(root, "web", "dist"),
    MEDIA_ROOT: mkdtempSync(path.join(tmpdir(), "mise-e2e-media-")),
    AUTH_MODES: "local",
    IMPORT_ALLOW_PRIVATE: "true",
  },
});
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => child.kill(signal));
child.on("exit", (code) => {
  site.close();
  process.exit(code ?? 0);
});
