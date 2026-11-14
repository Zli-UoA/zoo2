import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite-plus";

const outDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "dist/client");

/**
 * 出来上がった sw-push.js が、Service Worker の classic script として動く形かを確かめる。崩れると
 * CI もビルドも通ったまま、本番の通知が黙って「新しいメッセージ」に戻るので、ビルドを止める。
 *
 * - public/sw-push.js（dev 用の空のもの）を置き換えている。
 * - `import.meta` と `import(` が残っていない（Service Worker では使えない）。
 * - 指している `/assets/*.wasm` が、出力にある。
 */
const verifyServiceWorkerScript: Plugin = {
  name: "verify-sw-push",
  apply: "build",
  writeBundle() {
    const source = readFileSync(path.join(outDir, "sw-push.js"), "utf8");
    const problems: string[] = [];
    if (!source.includes("zooDecryptPush")) problems.push("zooDecryptPush を置いていない");
    if (source.includes("import.meta")) problems.push("import.meta が残っている");
    if (/\bimport\s*\(/.test(source)) problems.push("import() が残っている");
    const wasm = [...new Set(source.match(/\/assets\/[\w.-]+\.wasm/g) ?? [])];
    if (wasm.length !== 2) problems.push(`wasm の参照が 2 つでない: ${wasm.join(", ")}`);
    for (const file of wasm) {
      if (!existsSync(path.join(outDir, file))) problems.push(`${file} がない`);
    }
    if (problems.length > 0) this.error(`sw-push.js: ${problems.join("; ")}`);
  },
};

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
  plugins: [verifyServiceWorkerScript],
  // IIFE には import.meta がない。wasm の場所は明示して渡すが、wasm-bindgen と Emscripten の
  // 既定の場所の計算が壊れないよう、Service Worker の URL にする。
  define: { "import.meta.url": "self.location.href" },
  resolve: { tsconfigPaths: true },
  build: {
    outDir,
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
