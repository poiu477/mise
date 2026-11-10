// Starts mise for the browser journeys: a fresh `mise_e2e` database on the dev Postgres (or the server in
// DATABASE_URL), the demo bundle imported and curated (plus one retired dish address), then mise-server on 127.0.0.1:8095 serving web/dist.
// Playwright runs this as its webServer and stops it when the tests end; each start drops and rebuilds the
// database, so every run begins from an empty install. Needs `cargo build -p mise-cli -p mise-api` and
// `npm run build` first.
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
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
await db.end();

const child = spawn(bin("mise-server"), [], {
  cwd: root,
  stdio: "inherit",
  env: {
    ...env,
    MISE_BIND: "127.0.0.1:8095",
    MISE_WEB_DIR: path.join(root, "web", "dist"),
    MEDIA_ROOT: mkdtempSync(path.join(tmpdir(), "mise-e2e-media-")),
    AUTH_MODES: "local",
  },
});
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => child.kill(signal));
child.on("exit", (code) => process.exit(code ?? 0));
