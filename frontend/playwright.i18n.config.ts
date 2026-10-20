import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

/** Runs without a backend: the tests exercise the application shell and dialogs. */
export default defineConfig({
  ...base,
  globalSetup: undefined,
  use: { ...base.use, baseURL: "http://localhost:4174" },
  testMatch: "i18n.spec.ts",
  testIgnore: /oidc\.spec\.ts/,
  webServer: {
    command: process.env.CI ? "pnpm preview --port 4174" : "pnpm build && pnpm preview --port 4174",
    url: "http://localhost:4174",
    reuseExistingServer: false,
    timeout: 120_000,
    env: {
      VITE_SHAPE_MODE: "proxy",
      VITE_DEV_USER_ID: "00000000-0000-0000-0000-000000000001",
    },
  },
});
