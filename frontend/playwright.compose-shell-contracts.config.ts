import { defineConfig } from "@playwright/test";
import base from "./playwright.compose-storybook.config";

export default defineConfig({
  ...base,
  testMatch: /compose-shell-contracts\.spec\.ts$/,
  outputDir: "./test-results/compose-shell-contracts",
  use: {
    ...base.use,
    baseURL: process.env.STORYBOOK_PARITY_URL ?? base.use?.baseURL,
    trace: process.env.STORYBOOK_PARITY_TRACE === "1" ? "retain-on-failure" : "off",
    screenshot: "only-on-failure",
  },
  webServer: process.env.STORYBOOK_PARITY_URL ? undefined : base.webServer,
});
