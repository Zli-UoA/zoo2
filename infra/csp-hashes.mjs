#!/usr/bin/env node
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

/**
 * ビルド済み `index.html` に含まれるインライン `<script>` の SHA-256 を
 * CSP の `script-src` 用に列挙する（`'sha256-...'` を空白区切り）。
 *
 * TanStack Start は SPA シェルにテーマ初期化とストリーム用のインライン
 * script を埋め込むため、`'unsafe-inline'` を避けてハッシュで許可する。
 * ハッシュは index.html と同時に生成するので、ビルドごとに変化してよい。
 *
 *   node infra/csp-hashes.mjs frontend/dist/client/index.html out.txt
 */
const [, , htmlPath = "dist/client/index.html", outputPath = "csp-script-src.txt"] = process.argv;

const html = readFileSync(htmlPath, "utf8");
const hashes = [
  ...html.matchAll(/<script(?![^>]*\ssrc=)[^>]*>([\s\S]*?)<\/script>/g),
].map((match) => {
  const digest = createHash("sha256").update(match[1]).digest("base64");
  return `'sha256-${digest}'`;
});

const unique = [...new Set(hashes)];
if (unique.length === 0) {
  throw new Error(`no inline scripts found in ${htmlPath}; refusing to build a CSP without script hashes`);
}

writeFileSync(outputPath, unique.join(" "));
console.log(`csp: ${unique.length} inline script hash(es) -> ${outputPath}`);
