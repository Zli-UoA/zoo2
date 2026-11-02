import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: /public-profile\.spec\.ts/,
  timeout: 120_000,
  fullyParallel: false,
  use: {
    baseURL: process.env.E2E_BASE_URL ?? "http://127.0.0.1:5173",
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    launchOptions: { executablePath: process.env.CHROME_BIN, args: ["--no-sandbox"] },
  },
  projects: [{ name: "react" }, { name: "compose" }],
  webServer: {
    command: "pnpm dev --host 127.0.0.1",
    url: "http://127.0.0.1:5173",
    reuseExistingServer: !process.env.CI,
  },
});
