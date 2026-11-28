import { pdfAssets } from "./build/pdf-assets";
import { defineConfig, lazyPlugins, type Plugin } from "vite-plus";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { devtools } from "@tanstack/devtools-vite";

import { tanstackStart } from "@tanstack/react-start/plugin/vite";

import viteReact from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { playwright } from "vite-plus/test/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";

const dirname = path.dirname(fileURLToPath(import.meta.url));

// Include lazy chunks, SQLite workers and fonts before publishing a new offline shell.
const offlineAssets: Plugin = {
  name: "offline-build-assets",
  apply: "build",
  generateBundle(_options, bundle) {
    if (this.environment.name !== "client") return;
    const assets = Object.keys(bundle)
      .filter((name) => name.startsWith("assets/") && !name.endsWith(".map"))
      .map((name) => `/${name}`)
      .sort();
    const entry = assets.find((name) => /^\/assets\/index-[^/]+\.js$/.test(name));
    if (!entry) this.error("Missing offline app entry");
    this.emitFile({
      type: "asset",
      fileName: "pwa-assets.json",
      source: JSON.stringify({ entry, assets }),
    });
  },
};

const config = defineConfig({
  worker: { format: "es" },
  optimizeDeps: {
    include: [
      "postal-mime",
      "@kenjiuno/msgreader",
      "fflate",
      "saxes",
      "buffer/",
      "@base-ui/react/toggle-group",
      "@base-ui/react/toggle",
      // 暗号ストアの Worker（lib/e2ee）が読む。dev サーバーがログインのあとで初めて見つけると、
      // 依存を最適化し直してページを読み直し、操作中の画面が消える（OIDC E2E）。
      "@journeyapps/wa-sqlite",
      "@journeyapps/wa-sqlite/dist/wa-sqlite.mjs",
      "@journeyapps/wa-sqlite/src/examples/OPFSCoopSyncVFS.js",
    ],
  },
  fmt: {},
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }, "@shadcn/lint"],
    settings: {
      shadcn: {
        // UI は相対 import も多く、components.json の alias だけでは認識できない。
        componentImports: ["^@/components/ui/", "(\\.\\./)+ui/"],
        themeImports: ["tw-animate-css"],
        note: "デザイン上の例外は frontend/README.md を参照。",
      },
    },
    rules: {
      "vite-plus/prefer-vite-plus-imports": "error",
      "shadcn/no-restyle": [
        "error",
        {
          allow: ["layout"],
          // 呼び出し側が責任を持つ範囲だけを component ごとに開ける。
          // 色と形は基本 variant で表現し、contract では開けない。
          contracts: [
            { pattern: "^Skeleton$", allow: ["layout", "shape"] },
            { pattern: "^Input$", allow: ["layout", "typography", "spacing"] },
            {
              pattern: "^Textarea$",
              allow: ["layout", "typography", "spacing", "shape", "effects"],
            },
            {
              pattern: "^DialogContent$",
              allow: ["layout", "spacing", "shape", "motion"],
            },
            {
              pattern: "^DialogHeader$",
              allow: ["layout", "spacing", "shape"],
            },
            {
              pattern: "^DialogTitle$",
              allow: ["layout", "spacing", "typography"],
            },
            {
              pattern: "^DialogDescription$",
              allow: ["layout", "typography"],
            },
            {
              pattern: "^ExternalLink$",
              allow: ["layout", "typography", "color"],
            },
            { pattern: "^SettingsSection$", allow: ["layout", "color"] },
            { pattern: "^Spinner$", allow: ["layout", "color", "spacing"] },
            {
              pattern: "^TooltipTrigger$",
              allow: ["layout", "shape", "color"],
            },
            { pattern: "^ScrollArea$", allow: ["layout", "spacing"] },
            { pattern: "^AvatarFallback$", allow: ["layout", "typography"] },
            { pattern: "^ContentLink$", allow: ["layout", "typography"] },
            {
              pattern: "^QuotePostLink$",
              allow: ["layout", "typography", "spacing", "color"],
            },
            { pattern: "^Image$", allow: ["layout", "shape"] },
            { pattern: "^DropdownMenuTrigger$", allow: ["layout", "effects"] },
            { pattern: "^AppAvatar$", allow: ["layout", "typography"] },
          ],
          message: {
            spacing:
              "Use a {{component}} size: {{sizes|none defined}}, or put margin here / gap on the parent.",
            default:
              "Use a {{component}} variant: {{variants|none defined}}. Add one in {{file}} only if the design calls for it.",
          },
        },
      ],
      "shadcn/no-raw-colors": [
        "error",
        { message: "Use a theme color from {{file}}: {{suggestions|a declared token}}." },
      ],
      "shadcn/no-arbitrary-values": ["error", { allow: ["layout"] }],
      "shadcn/no-inline-styles": "error",
      "shadcn/no-unknown-classes": ["error", { allow: ["mfm-math", "mfm-search", "toaster"] }],
      "shadcn/require-static-classes": "error",
    },
    overrides: [
      {
        // コンポーネント自身は内部スタイルを所有する（上流の推奨に従う）。
        files: ["src/components/ui/**"],
        rules: {
          "shadcn/no-restyle": "off",
          "shadcn/no-arbitrary-values": "off",
          "shadcn/require-static-classes": "off",
        },
      },
      {
        // テスト／Story の固定サイズや style はハーネスなので対象外。
        files: ["**/*.browser.test.tsx", "**/*.stories.tsx", "src/test/vrt/**", "e2e/**"],
        rules: {
          "shadcn/no-restyle": "off",
          "shadcn/no-arbitrary-values": "off",
          "shadcn/no-inline-styles": "off",
          "shadcn/require-static-classes": "off",
        },
      },
    ],
    options: { typeAware: true, typeCheck: true },
  },
  resolve: { tsconfigPaths: true, alias: { buffer: "buffer/" } },
  preview: { host: "127.0.0.1" },
  server: {
    // Local PBX development only; production uses the matching Traefik route.
    proxy: process.env.SIP_DEV_PROXY_URL
      ? {
          "/sip/": {
            target: process.env.SIP_DEV_PROXY_URL,
            ws: true,
            rewrite: (path: string) => path.replace(/^\/sip/, ""),
          },
        }
      : undefined,
    // @zoo/crypto-wasm は link: で frontend の外（crates/zoo-crypto-wasm/pkg）を指す。
    // dev サーバーとブラウザーのテストが *_bg.wasm を返せるよう、そのディレクトリーだけ許可する。
    fs: { allow: [dirname, path.resolve(dirname, "../crates/zoo-crypto-wasm/pkg")] },
  },
  build: {
    // perf-kit の late-chunk 解析に使う。描画には影響しない。
    manifest: true,
    // Production CSP permits same-origin fonts only. Keep small KaTeX fonts out of data URLs.
    assetsInlineLimit: (filePath) => (/\.(?:woff2?|ttf|otf)$/i.test(filePath) ? false : undefined),
  },
  plugins: lazyPlugins(() => [
    pdfAssets(),
    offlineAssets,
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
          // Worker などブラウザーの API が要るテストは browser プロジェクトで動かす。
          exclude: ["**/node_modules/**", "src/**/*.browser.test.ts"],
        },
      },
      {
        extends: true,
        optimizeDeps: {
          include: [
            "vite-plus/test/browser/context",
            // Lazy imports must be optimized before browser tests start;
            // discovering them mid-run reloads tests and splits React state.
            "@base-ui/react/tooltip",
            "@tanstack/browser-db-sqlite-persistence",
            "jsqr",
            "qrcode-generator",
            // 暗号ストアの Worker（lib/e2ee）が読む。テストの途中で最適化し直して読み込み直さないように。
            "@journeyapps/wa-sqlite",
            "@journeyapps/wa-sqlite/dist/wa-sqlite.mjs",
            "@journeyapps/wa-sqlite/src/examples/OPFSCoopSyncVFS.js",
          ],
        },
        test: {
          name: "browser",
          include: ["src/**/*.browser.test.{ts,tsx}"],
          setupFiles: ["./src/test/browser-setup.ts"],
          browser: {
            enabled: true,
            headless: true,
            // Linux の chromium-headless-shell はアニメーション停止時に rAF が停滞することがあるため、
            // CHROME_BIN 未指定時はフル Chromium（channel: "chromium"）を使う。
            provider: playwright({
              launchOptions: process.env.CHROME_BIN
                ? { executablePath: process.env.CHROME_BIN }
                : { channel: "chromium" },
            }),
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
        resolve: {
          alias: {
            // The addon imports this pnpm-scoped dependency after stories have started.
            "axe-core": createRequire(import.meta.resolve("@storybook/addon-a11y")).resolve(
              "axe-core",
            ),
          },
        },
        optimizeDeps: { include: ["@testing-library/react", "axe-core"] },
        test: {
          name: "storybook",
          setupFiles: ["./.storybook/vitest.setup.ts"],
          browser: {
            enabled: true,
            headless: true,
            provider: playwright({ launchOptions: { executablePath: process.env.CHROME_BIN } }),
            instances: [{ browser: "chromium" }],
          },
        },
      },
    ],
  },
});

export default config;
