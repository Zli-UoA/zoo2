import { defineConfig, devices } from "@playwright/test";

// 実コンポーネントのレイアウトを、バックエンドなしで検証する。
export default defineConfig({
  testDir: "./e2e",
  testMatch: "layout.spec.ts",
  use: { baseURL: "http://localhost:4175" },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "mobile", use: { ...devices["iPhone 13"], defaultBrowserType: "chromium" } },
  ],
  webServer: {
    command: "pnpm exec vp dev --config e2e/layout.vite.config.ts --port 4175",
    url: "http://localhost:4175/e2e/fixtures/layout.html",
    timeout: 60_000,
  },
});
