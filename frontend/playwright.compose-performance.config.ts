import { defineConfig } from "@playwright/test";
import compose from "./playwright.compose.config";

export default defineConfig({
  ...compose,
  testMatch: /compose-performance\.spec\.ts$/,
  outputDir: "./test-results/compose-performance",
  use: {
    ...compose.use,
    launchOptions: {
      ...compose.use?.launchOptions,
      executablePath: process.env.CHROME_BIN,
      args: ["--no-sandbox", "--ignore-certificate-errors"],
    },
  },
});
