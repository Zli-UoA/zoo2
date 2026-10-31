import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: /compose\.spec\.ts$/,
  workers: 1,
  outputDir: "./test-results/compose",
  fullyParallel: false,
  timeout: 120_000,
  use: {
    ignoreHTTPSErrors: true,
    launchOptions: { args: ["--ignore-certificate-errors"] },
    actionTimeout: 15_000,
    baseURL: "http://127.0.0.1:4186",
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
  },
  webServer: [
    {
      command: "node e2e/compose-api.mjs",
      reuseExistingServer: !process.env.CI,
      url: "https://127.0.0.1:4187/health",
      ignoreHTTPSErrors: true,
    },
    {
      command: "pnpm exec vp dev --port 4186 --host 127.0.0.1 --mode production",
      reuseExistingServer: !process.env.CI,
      url: "http://127.0.0.1:4186",
      timeout: 120_000,
      env: {
        VITE_API_BASE_URL: "https://127.0.0.1:4187",
        VITE_DEV_USER_ID: "00000000-0000-0000-0000-000000000001",
        VITE_SHAPE_MODE: "proxy",
        VITE_TXID_MODE: "awaitTxId",
        VITE_EXPERIMENTAL_COMPOSE: "true",
        VITE_SUMMALY_URL: "https://127.0.0.1:4187/__summaly",
        VITE_MEDIA_PROXY_URL: "https://127.0.0.1:4187/__media",
      },
    },
  ],
});
