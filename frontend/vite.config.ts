import { defineConfig, lazyPlugins } from "vite-plus";
import { devtools } from "@tanstack/devtools-vite";
import { nitro } from "nitro/vite";

import { tanstackStart } from "@tanstack/react-start/plugin/vite";

import viteReact from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Nitro はビルドと本番サーバー用。テスト中に有効にすると、Nitro の SSR 環境が
// React の CJS を読み込み、`ReferenceError: module is not defined` を出すので外す。
// `mode` は `--mode` で変わり得るため、テストランナー自身が設定する VITEST で判定する。
const isVitest = process.env.VITEST === "true";

const config = defineConfig({
  fmt: {},
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  resolve: { tsconfigPaths: true },
  plugins: lazyPlugins(() => [
    devtools(),
    tailwindcss(),
    tanstackStart(),
    ...(isVitest ? [] : [nitro({ preset: "node-server" })]),
    viteReact(),
  ]),
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});

export default config;
