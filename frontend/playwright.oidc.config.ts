import { defineConfig, devices } from "@playwright/test";

/**
 * OIDC PKCE の E2E 専用設定。既定の `pnpm test` / `pnpm test:e2e` には含めない。
 *
 *   make db-up && make migrate
 *   DEV_AUTH_ENABLED=false ENVIRONMENT=production \
 *     OIDC_ISSUER=http://127.0.0.1:4100/oauth2/openid/zoo OIDC_AUDIENCE=zoo \
 *     CORS_ALLOWED_ORIGINS=http://localhost:5173 \
 *     DATABASE_URL=... ELECTRIC_URL=http://localhost:3000 \
 *     cargo run   # backend
 *   cd frontend && pnpm test:e2e:oidc
 *
 * モック IdP とフロントの dev サーバは Playwright が起動する。
 */
const baseURL = process.env.E2E_BASE_URL ?? "http://localhost:5173";
const apiBaseUrl = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const mockIdpPort = process.env.MOCK_IDP_PORT ?? "4100";
const mockIdpIssuer = `http://127.0.0.1:${mockIdpPort}/oauth2/openid/zoo`;

export default defineConfig({
  testDir: "./e2e",
  testMatch: /oidc\.spec\.ts/,
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: "list",
  timeout: 60_000,
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
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
