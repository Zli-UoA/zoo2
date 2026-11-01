import { composeVisualColorThreshold } from "./e2e/compose-visual";
import { defineConfig } from "@playwright/test";
import { resolve, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const outputFlag = process.argv.findIndex(
  (arg) => arg === "--output" || arg.startsWith("--output="),
);
const outputOption =
  outputFlag < 0
    ? undefined
    : (process.argv[outputFlag].split("=")[1] ?? process.argv[outputFlag + 1]);
const outputDir = resolve(
  dirname(fileURLToPath(import.meta.url)),
  outputOption ??
    process.env.STORYBOOK_PARITY_OUTPUT_DIR ??
    "test-results/compose-storybook-parity",
);
// Workers reload this config without the runner CLI; inherit the resolved domain path.
process.env.STORYBOOK_PARITY_OUTPUT_DIR = outputDir;

export default defineConfig({
  testDir: "./e2e",
  testMatch: /compose-storybook-parity\.spec\.ts$/,
  outputDir,
  snapshotPathTemplate: `${outputDir}/reference/{projectName}/{testFilePath}/{testName}/{arg}{ext}`,
  updateSnapshots: "none",
  fullyParallel: true,
  workers: process.env.CI ? 2 : 2,
  timeout: 90 * 60_000,
  expect: {
    timeout: 15_000,
    toHaveScreenshot: { threshold: composeVisualColorThreshold, maxDiffPixels: 0 },
  },
  reporter: [
    ["list"],
    [
      "html",
      {
        outputFolder: resolve(
          dirname(fileURLToPath(import.meta.url)),
          "playwright-report",
          basename(outputDir),
        ),
        open: "never",
      },
    ],
  ],
  // Match Karma and normal Chromium; legacy headless-shell rounds font advances.
  use: {
    channel: "chromium",
    baseURL: process.env.STORYBOOK_PARITY_URL ?? "http://127.0.0.1:6006",
    viewport: { width: 1440, height: 900 },
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    reducedMotion: "reduce",
    actionTimeout: 15_000,
    trace: process.env.STORYBOOK_PARITY_TRACE === "1" ? "retain-on-failure" : "off",
  },
  webServer: process.env.STORYBOOK_PARITY_URL
    ? undefined
    : {
        command: "./node_modules/.bin/storybook dev --ci --no-open --port 6006 --host 127.0.0.1",
        url: "http://127.0.0.1:6006/index.json",
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
      },
});
