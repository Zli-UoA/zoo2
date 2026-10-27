import { defineConfig, devices } from "@playwright/test";

/**
 * OIDC PKCE の E2E 専用設定。既定の `pnpm test` / `pnpm test:e2e` には含めない。
 *
 *   make db-up && make migrate
 *   DEV_AUTH_ENABLED=false ENVIRONMENT=production \
 *     OIDC_ISSUER=http://127.0.0.1:4100/oauth2/openid/zoo OIDC_AUDIENCE=zoo \
 *     CORS_ALLOWED_ORIGINS=http://localhost:5173 \
 *     DATABASE_URL=... ELECTRIC_URL=http://127.0.0.1:3000 \
 *     cargo run   # backend
 *   cd frontend && pnpm test:e2e:oidc
 *
 * モック IdP とフロントの dev サーバは Playwright が起動する。
 * CI（.github/workflows/ci-frontend-auth-e2e.yaml）では同じ手順で
 * PostgreSQL + Electric + 本番同等設定の Rust を起動してから実行する。
 */
const baseURL = process.env.E2E_BASE_URL ?? "http://localhost:5173";
const apiUpstream = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const http2Port = process.env.E2E_HTTP2_PORT ?? "8443";
const apiBaseUrl = `https://localhost:${http2Port}`;
const mockIdpPort = process.env.MOCK_IDP_PORT ?? "4100";
const mockIdpIssuer = `http://127.0.0.1:${mockIdpPort}/oauth2/openid/zoo`;

export default defineConfig({
  testDir: "./e2e",
  testMatch: /oidc\.spec\.ts/,
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: process.env.CI ? "github" : "list",
  timeout: 90_000,
  // Cold dev-module loading and Electric/OPFS initialization can outlast the
  // default 5s on CI. Keep assertions bounded without retrying the whole flow.
  expect: { timeout: 15_000 },
  use: {
    baseURL,
    // E2E専用プロキシの一時的な自己署名証明書を信頼する。
    ignoreHTTPSErrors: true,
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
      env: { MOCK_IDP_PORT: mockIdpPort, MOCK_IDP_CLIENT_ID: "zoo" },
    },
    {
      command: "pnpm dev",
      url: baseURL,
      reuseExistingServer: !process.env.CI,
      timeout: 120_000,
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
