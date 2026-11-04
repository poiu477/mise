// Checks the server's OpenAPI document (web/openapi.json, written by `mise-server openapi`) for what OpenAPI 3.1
// requires and the generator can leave out: every response needs a description. Exits non-zero, naming each gap.
import { readFileSync } from "node:fs";

const doc = JSON.parse(readFileSync(new URL("../openapi.json", import.meta.url), "utf8"));
const gaps = [];
for (const [path, ops] of Object.entries(doc.paths ?? {})) {
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
  process.exit(1);
}
console.log(`OpenAPI ${doc.openapi}: every response has a description.`);
