import { defineConfig } from "vite-plus";

/**
 * Service Worker が `importScripts` で読む、Push の復号の script（`dist/client/sw-push.js`）を作る
 * （`src/sw/push.ts`。docs/e2ee.md の「Web Push の復号」）。
 *
 * `pnpm build` が、アプリのビルドのあとに同じ `dist/client` へ出す。Service Worker は classic
 * script なので IIFE にし、名前は固定にする（`public/sw.js` が名前で読む）。wasm は assets に
 * ハッシュ付きで出し、アプリの Worker と中身が同じなら同じファイルを使う。
 */
export default defineConfig({
  publicDir: false,
  // IIFE には import.meta がない。wasm の場所は明示して渡すが、wasm-bindgen と Emscripten の
  // 既定の場所の計算が壊れないよう、Service Worker の URL にする。
  define: { "import.meta.url": "self.location.href" },
  resolve: { tsconfigPaths: true },
  build: {
    outDir: "dist/client",
    emptyOutDir: false,
    copyPublicDir: false,
    // wasm を data: URL にせず、assets のファイルとして読む。
    assetsInlineLimit: 0,
    rolldownOptions: {
      input: "src/sw/push.ts",
      output: {
        format: "iife",
        entryFileNames: "sw-push.js",
        assetFileNames: "assets/[name]-[hash][extname]",
      },
    },
  },
});
