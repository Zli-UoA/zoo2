import { defineConfig } from "vite-plus";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Also packaged in Android: the same MediaPreview and parsers, with no remote web app.
export default defineConfig({
  base: "./",
  publicDir: false,
  plugins: [react(), tailwindcss()],
  resolve: { tsconfigPaths: true, alias: { buffer: "buffer/" } },
  worker: { format: "es" },
  build: {
    target: "es2022",
    outDir: "../frontend-compose/build/generated/media-preview-assets/media-preview",
    emptyOutDir: true,
    rollupOptions: { input: "media-preview.html" },
    assetsInlineLimit: 0,
  },
});
