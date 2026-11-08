import { defineConfig } from "@playwright/test";
import compose from "./playwright.compose.config";

export default defineConfig({
  ...compose,
  testMatch: /frontend-selection\.spec\.ts$/,
  outputDir: "./test-results/frontend-selection",
  webServer: (Array.isArray(compose.webServer) ? compose.webServer : []).map((server) => ({
    ...server,
    env: { ...server.env, VITE_EXPERIMENTAL_COMPOSE: "false" },
  })),
});
