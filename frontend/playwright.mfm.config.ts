import { defineConfig, devices } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  testMatch: "mfm.spec.ts",
  use: { baseURL: "http://localhost:4176" },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "mobile", use: { ...devices["iPhone 13"], defaultBrowserType: "chromium" } },
  ],
  webServer: {
    command: "pnpm exec vp dev --config e2e/layout.vite.config.ts --port 4176",
    url: "http://localhost:4176/e2e/fixtures/mfm.html",
    timeout: 60_000,
  },
});
