import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

/** Runs without a backend: the tests exercise the application shell and dialogs. */
export default defineConfig({
  ...base,
  globalSetup: undefined,
  testMatch: "i18n.spec.ts",
  testIgnore: /oidc\.spec\.ts/,
  webServer: {
    command: "pnpm build && pnpm preview --port 5173",
    url: "http://localhost:5173",
    reuseExistingServer: false,
    timeout: 120_000,
    env: {
      VITE_SHAPE_MODE: "proxy",
      VITE_DEV_USER_ID: "00000000-0000-0000-0000-000000000001",
    },
  },
});
