import { defineConfig } from "vite-plus";
export default defineConfig({
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  build: {
    outDir: "../frontend-compose/androidApp/build/generated/phone-engine",
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
