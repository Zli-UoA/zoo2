import { defineConfig, devices } from "@playwright/test";

const baseURL = process.env.E2E_OFFLINE_BASE_URL ?? "http://localhost:5173";
const apiBaseUrl = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const devUserId = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";

/**
 * オフラインのリロードはビルド済み app shell と hashed assets を前提にする。
 * 開発サーバーはモジュールを都度配信するため、ここでは `pnpm build` の成果物を
 * preview で配信して Service Worker の precache を検証する。
 */
export default defineConfig({
  testDir: "./e2e",
  testMatch: "offline.spec.ts",
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  globalSetup: "./e2e/global-setup.ts",
  timeout: 60_000,
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `pnpm build && pnpm preview --port 5173`,
    url: baseURL,
    // 開発サーバーを誤って再利用すると、ビルド済みでない app shell を
    // 検証してしまうため、この設定では必ず preview を起動する。
    reuseExistingServer: false,
    timeout: 300_000,
    env: {
      VITE_SHAPE_MODE: "proxy",
      VITE_PREVIEW_AUTH: "true",
      VITE_DEV_USER_ID: devUserId,
      VITE_API_BASE_URL: apiBaseUrl,
    },
  },
});
