import path from "node:path";
import { fileURLToPath } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import viteReact from "@vitejs/plugin-react";
import { defineConfig } from "vite-plus";

const dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  // emojibase-data と mockServiceWorker.js を Storybook でも配信する。
  publicDir: path.resolve(dirname, "../public"),
  plugins: [tailwindcss(), viteReact()],
  resolve: {
    alias: {
      "@": path.resolve(dirname, "../src"),
      "#": path.resolve(dirname, "../src"),
    },
  },
});
