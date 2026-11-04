// Is the web app ready for the next TypeScript major? Run from web/ (npm run typescript:ready) after `npm ci`. Ready means a newer TypeScript
// major has been released and the latest release of every dependency that declares a TypeScript peer range accepts
// it. Reads the npm registry directly (no shell); prints `ready=`, `version=` and `blockers=` lines for GitHub
// Actions on stdout, and one line per tool on stderr.
import { readFileSync } from "node:fs";
import semver from "semver";

const latest = async (name) => {
  const res = await fetch(`https://registry.npmjs.org/${name.replace("/", "%2f")}/latest`);
  if (!res.ok) throw new Error(`npm registry: ${name}: ${res.status}`);
  return res.json();
};

const pkg = JSON.parse(readFileSync("package.json", "utf8"));
const newest = (await latest("typescript")).version;
const ahead = semver.major(newest) > semver.minVersion(pkg.devDependencies.typescript).major;

const blockers = [];
for (const dep of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies })) {
  if (dep === "typescript") continue;
  const range = (await latest(dep)).peerDependencies?.typescript;
  if (!range) continue;
  const ok = semver.satisfies(newest, range);
  console.error(`${dep}@latest wants typescript ${range}: ${ok ? "accepts" : "rejects"} ${newest}`);
  if (!ok) blockers.push(dep);
}

console.log(`ready=${ahead && blockers.length === 0}`);
console.log(`version=${newest}`);
console.log(`blockers=${blockers.join(", ") || "none"}`);
