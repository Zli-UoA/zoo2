import { defineConfig, devices } from "@playwright/test";

/**
 * E2EE の E2E（#363）専用の設定。既定の `pnpm test:e2e` と `pnpm test:e2e:oidc` には含めない。
 *
 * パスキーの RP ID は OIDC の issuer のホスト名で、IP アドレスは RP ID にできないので、
 * モック IdP の issuer とフロントのオリジンを `localhost` にする（docs/e2ee.md の「RP ID の決め方」）。
 * そのため、issuer が `127.0.0.1` の OIDC の E2E とは backend を分けて起動する。
 *
 *   make db-up && make migrate
 *   DEV_AUTH_ENABLED=false ENVIRONMENT=production \
 *     OIDC_ISSUER=http://localhost:4101/oauth2/openid/zoo OIDC_AUDIENCE=zoo \
 *     OIDC_REAUTH_AUDIENCES=zoo E2EE_ENCRYPTED_CHANNELS_ENABLED=true \
 *     CORS_ALLOWED_ORIGINS=http://localhost:5173 \
 *     DATABASE_URL=... ELECTRIC_URL=http://127.0.0.1:3000 \
 *     cargo run   # backend
 *   cd frontend && pnpm test:e2e:e2ee
 *
 * テストは DB を `docker compose -f ../infra/db/compose.yml exec postgres` で読む。
 * compose のプロジェクト名を変えて DB を起動したときは、`COMPOSE_PROJECT_NAME` を合わせる。
 * CI（.github/workflows/ci-frontend-auth-e2e.yaml の e2ee-e2e）も同じ手順で実行する。
 */
const baseURL = process.env.E2E_BASE_URL ?? "http://localhost:5173";
const devServerPort = new URL(baseURL).port || "5173";
const apiUpstream = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const http2Port = process.env.E2E_HTTP2_PORT ?? "8444";
const apiBaseUrl = `https://localhost:${http2Port}`;
const mockIdpPort = process.env.MOCK_IDP_PORT ?? "4101";
const mockIdpIssuer = `http://localhost:${mockIdpPort}/oauth2/openid/zoo`;

export default defineConfig({
  testDir: "./e2e",
  testMatch: /e2ee\.spec\.ts/,
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: process.env.CI ? "github" : "list",
  // 2 人 × 2 端末の初回設定、解錠、送受信、絵文字の検証、失効、退出を 1 本で通す。
  timeout: 10 * 60_000,
  // dev サーバーの初回のモジュール変換、Electric の同期、wasm の初期化を待つ。
  expect: { timeout: 30_000 },
  use: {
    baseURL,
    // E2E専用プロキシの一時的な自己署名証明書を信頼する。
    ignoreHTTPSErrors: true,
    locale: "ja-JP",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: "node e2e/http2-api-server.ts",
      url: `${apiBaseUrl}/health`,
      ignoreHTTPSErrors: true,
      timeout: 30_000,
      env: { E2E_HTTP2_PORT: http2Port, E2E_API_UPSTREAM: apiUpstream },
    },
    {
      command: "node e2e/mock-idp-server.ts",
      url: `${mockIdpIssuer}/.well-known/openid-configuration`,
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
      env: { MOCK_IDP_PORT: mockIdpPort, MOCK_IDP_CLIENT_ID: "zoo", MOCK_IDP_HOST: "localhost" },
    },
    {
      // 既定のスモークと同じく mode を production にして、TanStack Devtools のログ転送を止める。
      // 4 つのブラウザーのコンテキストのあいだでログが転送し合って膨らみ、dev サーバーが
      // メモリーを使い切る。dev サーバーなので NODE_ENV が development の間は import.meta.env.DEV は true。
      command: `pnpm exec vp dev --port ${devServerPort} --strictPort --mode production`,
      url: baseURL,
      reuseExistingServer: !process.env.CI,
      timeout: 180_000,
      env: {
        VITE_SHAPE_MODE: "proxy",
        VITE_API_BASE_URL: apiBaseUrl,
        VITE_OIDC_ISSUER: mockIdpIssuer,
        VITE_OIDC_CLIENT_ID: "zoo",
        VITE_OIDC_REDIRECT_URI: `${baseURL}/auth/callback`,
      },
    },
  ],
});
