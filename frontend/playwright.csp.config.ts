import { defineConfig, devices } from "@playwright/test";

/**
 * nginx が配信する本番ビルドの CSP をブラウザで検証する専用設定。
 * `pnpm test:e2e` / `pnpm test:e2e:oidc` には含めない。
 *
 *   pnpm build
 *   docker run --rm -d --name zoo-frontend-csp-test -p 127.0.0.1:3100:3000 zoo-frontend:stg
 *   E2E_BASE_URL=http://127.0.0.1:3100 pnpm test:e2e:csp
 *
 * CI の `container` ジョブはビルド済みイメージを起動して同じテストを実行する。
 */
const baseURL = process.env.E2E_BASE_URL ?? "http://127.0.0.1:3100";

export default defineConfig({
  testDir: "./e2e",
  testMatch: /csp\.spec\.ts/,
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: process.env.CI ? "github" : "list",
  timeout: 30_000,
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
