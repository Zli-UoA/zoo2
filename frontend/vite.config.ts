import { defineConfig, lazyPlugins } from "vite-plus";
import { devtools } from "@tanstack/devtools-vite";
import { nitro } from "nitro/vite";

import { tanstackStart } from "@tanstack/react-start/plugin/vite";

import viteReact from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const config = defineConfig(({ mode }) => ({
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
    ...(mode === "test" ? [] : [nitro({ preset: "node-server" })]),
    viteReact(),
  ]),
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
}));

export default config;
