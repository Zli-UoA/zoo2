import { defineConfig } from "vite-plus";
export default defineConfig({
  build: {
    outDir: "build/android-phone",
    emptyOutDir: true,
    sourcemap: false,
    lib: {
      entry: "src/android-phone/engine.ts",
      formats: ["iife"],
      name: "ZooAndroidPhone",
      fileName: () => "engine.js",
    },
  },
});
