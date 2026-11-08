// Opens one GitLab issue when the tools accept the next TypeScript major. Reads `npm run -s typescript:ready`'s
// output on standard input; when it says ready=true and no such issue is open, creates one. Run by the scheduled
// `typescript watch` job in .gitlab-ci.yml, which provides CI_API_V4_URL and CI_PROJECT_ID, and RENOVATE_TOKEN
// (a project variable with the `api` scope) to write with.
import { text } from "node:stream/consumers";

const fields = Object.fromEntries(
  (await text(process.stdin))
    .split("\n")
    .filter((line) => line.includes("="))
    .map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]),
);
if (fields.ready !== "true") {
  console.log(`Not ready for TypeScript ${fields.version}: ${fields.blockers} still reject it.`);
  process.exit(0);
}

const api = `${process.env.CI_API_V4_URL}/projects/${process.env.CI_PROJECT_ID}`;
const headers = { "PRIVATE-TOKEN": process.env.RENOVATE_TOKEN, "Content-Type": "application/json" };
const marker = "lift the TypeScript hold";

const search = new URLSearchParams({ state: "opened", search: marker, in: "title" });
const found = await fetch(`${api}/issues?${search}`, { headers });
if (!found.ok) throw new Error(`listing issues: ${found.status} ${await found.text()}`);
if ((await found.json()).length > 0) {
  console.log("An issue to lift the hold is already open.");
  process.exit(0);
}

const description = `Every web dependency that declares a TypeScript peer range now accepts TypeScript ${fields.version} (checked by \`npm run typescript:ready\` in \`web/\`).

To upgrade: approve the TypeScript update on Renovate's Dependency Dashboard issue. Renovate then opens the merge request on its next run, and CI checks it.`;
const created = await fetch(`${api}/issues`, {
  method: "POST",
  headers,
  body: JSON.stringify({
    title: `TypeScript ${fields.version} is supported by our tools: ${marker}`,
    description,
    labels: "dependencies",
  }),
});
if (!created.ok) throw new Error(`creating the issue: ${created.status} ${await created.text()}`);
console.log(`Opened ${(await created.json()).web_url}`);
