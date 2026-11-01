import { defineConfig } from "@playwright/test";
import base from "./playwright.compose.config";

// Independent fixture state lets this contract suite run beside the main application E2E.
const servers = Array.isArray(base.webServer) ? base.webServer : [];
const api = "https://127.0.0.1:4197";
const app = "http://127.0.0.1:4196";
export default defineConfig({
  ...base,
  testMatch: /compose-shell-api-contracts\.spec\.ts$/,
  outputDir: "./test-results/compose-shell-api-contracts",
  use: { ...base.use, baseURL: app, trace: "off", screenshot: "only-on-failure" },
  webServer: [
    {
      ...servers[0],
      command: "node e2e/compose-api.mjs",
      url: `${api}/health`,
      env: { COMPOSE_API_PORT: "4197" },
    },
    {
      ...servers[1],
      command: "pnpm exec vp dev --port 4196 --host 127.0.0.1 --mode production",
      url: app,
      env: {
        ...servers[1]?.env,
        VITE_API_BASE_URL: api,
        VITE_SUMMALY_URL: `${api}/__summaly`,
        VITE_MEDIA_PROXY_URL: `${api}/__media`,
      },
    },
  ],
});
