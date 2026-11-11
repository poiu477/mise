// Checks the server's OpenAPI document (web/openapi.json, written by `mise-server openapi`) for what OpenAPI 3.1
// requires and the generator can leave out, and for mise's own rules: every response needs a description, and every
// path sits under /api/v1 except single sign-on's browser redirects, which keep the addresses providers have on
// record. Exits non-zero, naming each gap.
import { readFileSync } from "node:fs";

/** Paths outside /api/v1 on purpose: a sign-in provider sends people back to the callback's address. */
const REDIRECTS = ["/api/auth/oidc/start", "/api/auth/oidc/callback"];

const doc = JSON.parse(readFileSync(new URL("../openapi.json", import.meta.url), "utf8"));
const gaps = [];
const outside = [];
for (const [path, ops] of Object.entries(doc.paths ?? {})) {
  if (!path.startsWith("/api/v1/") && !REDIRECTS.includes(path)) outside.push(path);
  for (const [method, op] of Object.entries(ops)) {
    for (const [status, response] of Object.entries(op.responses ?? {})) {
      if (!response.description) gaps.push(`${method.toUpperCase()} ${path} ${status}`);
    }
  }
}
if (gaps.length) {
  console.error(
    `Responses without a description (add description = "..." to their #[utoipa::path]):`,
  );
  for (const g of gaps) console.error(`  ${g}`);
}
if (outside.length) {
  console.error(`Paths outside /api/v1 (write path = "/api/v1/..." in their #[utoipa::path]):`);
  for (const p of outside) console.error(`  ${p}`);
}
if (gaps.length || outside.length) process.exit(1);
console.log(
  `OpenAPI ${doc.openapi}: every response has a description and every path is under /api/v1.`,
);
