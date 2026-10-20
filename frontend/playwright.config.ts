import { defineConfig, devices } from "@playwright/test";

const baseURL = process.env.E2E_BASE_URL ?? "http://localhost:5173";
const apiBaseUrl = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const devUserId = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";

/**
 * スモークテスト（起動 → チャンネル作成 → メッセージ送信 → 反映）。
 *
 * 既定の `pnpm test`（vitest）には含めず、`pnpm test:e2e` で明示的に実行します。
 * PostgreSQL + Electric + バックエンドを先に起動しておく必要があります:
 *
 *   make db-up && make migrate
 *   make backend-dev        # DEV_AUTH_ENABLED=true (http://localhost:8000)
 *   cd frontend && pnpm test:e2e
 *
 * `webServer` はフロントエンド（proxy モード）だけを起動します。バックエンドは
 * 上記のとおり別途起動してください（`reuseExistingServer` により、手動起動済みの
 * 開発サーバがあれば再利用します）。
 */
export default defineConfig({
  testDir: "./e2e",
  // OIDC の通し確認はモック IdP と Rust を別途起動する必要があるため、
  // 既定のスモークからは除外する（`pnpm test:e2e:oidc` を参照）。
  testIgnore: /oidc\.spec\.ts/,
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  globalSetup: "./e2e/global-setup.ts",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // Devtools のボタンの重なりと、複数ブラウザ間のログ転送による干渉を避ける。
    command: "pnpm dev --mode production",
    url: baseURL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    env: {
      VITE_SHAPE_MODE: "proxy",
      VITE_DEV_USER_ID: devUserId,
      VITE_API_BASE_URL: apiBaseUrl,
    },
  },
});
