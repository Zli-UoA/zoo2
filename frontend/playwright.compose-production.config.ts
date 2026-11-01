import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  testMatch: /compose-production\.spec\.ts$/,
  workers: 1,
  outputDir: "./test-results/compose-production",
  timeout: 120_000,
  // Match Karma and normal Chromium; legacy headless-shell rounds font advances.
  use: {
    channel: "chromium",
    baseURL: "https://127.0.0.1:4188",
    ignoreHTTPSErrors: true,
    launchOptions: { args: ["--ignore-certificate-errors"] },
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    trace: "retain-on-failure",
  },
  webServer: [
    {
      command: "node e2e/compose-api.mjs",
      env: { COMPOSE_API_PORT: "4189" },
      url: "https://127.0.0.1:4189/health",
      ignoreHTTPSErrors: true,
    },
    {
      command: "node e2e/compose-static.mjs",
      url: "https://127.0.0.1:4188",
      ignoreHTTPSErrors: true,
    },
  ],
});
