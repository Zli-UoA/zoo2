import { defineConfig, lazyPlugins } from "vite-plus";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { devtools } from "@tanstack/devtools-vite";

import { tanstackStart } from "@tanstack/react-start/plugin/vite";

import viteReact from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { playwright } from "vite-plus/test/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";

const dirname = path.dirname(fileURLToPath(import.meta.url));

const config = defineConfig({
  optimizeDeps: { include: ["@base-ui/react/toggle-group", "@base-ui/react/toggle"] },
  fmt: {},
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  resolve: { tsconfigPaths: true },
  preview: { host: "127.0.0.1" },
  build: {
    // Production CSP permits same-origin fonts only. Keep small KaTeX fonts out of data URLs.
    assetsInlineLimit: (filePath) => (/\.(?:woff2?|ttf|otf)$/i.test(filePath) ? false : undefined),
  },
  plugins: lazyPlugins(() => [
    devtools(),
    tailwindcss(),
    tanstackStart({
      spa: {
        enabled: true,
        prerender: {
          outputPath: "/index.html",
        },
      },
    }),
    viteReact(),
  ]),
  test: {
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          environment: "node",
          include: ["src/**/*.test.ts"],
        },
      },
      {
        extends: true,
        optimizeDeps: { include: ["vite-plus/test/browser/context"] },
        test: {
          name: "browser",
          include: ["src/**/*.browser.test.{ts,tsx}"],
          setupFiles: ["./src/test/browser-setup.ts"],
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: "chromium" }],
          },
        },
      },
      {
        extends: true,
        test: {
          name: "vrt",
          include: ["src/**/*.vrt.test.tsx"],
          setupFiles: ["./src/test/browser-setup.ts", "./src/test/vrt-setup.ts"],
          browser: {
            enabled: true,
            headless: true,
            provider: playwright({
              // Keep text rasterization consistent across Linux hosts and GPUs.
              launchOptions: { args: ["--disable-lcd-text", "--font-render-hinting=none"] },
              contextOptions: {
                timezoneId: "Asia/Tokyo",
                locale: "ja-JP",
                colorScheme: "light",
                deviceScaleFactor: 1,
              },
            }),
            instances: [{ browser: "chromium" }],
            viewport: { width: 1440, height: 900 },
            expect: {
              toMatchScreenshot: {
                comparatorName: "pixelmatch",
                comparatorOptions: { allowedMismatchedPixelRatio: 0.003 },
              },
            },
          },
        },
      },
      {
        extends: true,
        plugins: [storybookTest({ configDir: path.join(dirname, ".storybook") })],
        optimizeDeps: { include: ["@testing-library/react"] },
        test: {
          name: "storybook",
          setupFiles: ["./.storybook/vitest.setup.ts"],
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: "chromium" }],
          },
        },
      },
    ],
  },
});

export default config;
