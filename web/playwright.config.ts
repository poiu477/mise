// Browser journeys (npm run e2e): real Chromium against mise-server on a fresh demo database (see e2e/serve.mjs).
// The journeys build on each other (first-run setup, then planning, then shopping), so they run in order in one
// worker, and a failure isn't retried: a retry would start from the state the failure left behind.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: "http://127.0.0.1:8095",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "node e2e/serve.mjs",
    url: "http://127.0.0.1:8095/api/health",
    reuseExistingServer: false,
    timeout: 180_000,
    stdout: "pipe",
  },
});
