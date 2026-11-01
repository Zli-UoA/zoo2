import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  testMatch: /compose-storybook\.spec\.ts$/,
  workers: 1,
  timeout: 120_000,
  outputDir: "./test-results/compose-storybook",
  // Match Karma and normal Chromium; legacy headless-shell rounds font advances.
  use: {
    channel: "chromium",
    baseURL: "http://127.0.0.1:6008",
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
  },
  webServer: {
    command: "node e2e/compose-storybook-static.mjs",
    url: "http://127.0.0.1:6008",
    reuseExistingServer: !process.env.CI,
  },
});
