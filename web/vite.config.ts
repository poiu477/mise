/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the API runs separately (cargo run -p mise-api); Vite forwards /api to it.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { "/api": "http://127.0.0.1:8080" },
  },
  // Unit tests live next to the code; the browser journeys in e2e/ run under Playwright (npm run e2e).
  test: {
    include: ["src/**/*.test.ts"],
  },
});
