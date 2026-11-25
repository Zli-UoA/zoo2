import { defineConfig } from "@x18n/core";

export default defineConfig({
  // ja が翻訳の源の辞書（DEFAULT_LOCALE）。英語ではなく日本語を基準に検査する。
  sourceLocale: "ja",
  resources: [{ pattern: "src/lib/i18n/locales/{locale}.json" }],
  // 設定ファイル自身の文字列（checks の ID など）を翻訳キーとしてスキャンしない。
  usage: { exclude: ["x18n.config.ts"] },
  checks: {
    // 変数だけで渡されるキーは検査できないので、CI で落とす。
    // 動的な部分は `prefix.${suffix}` のように静的な前方一致を持たせるか、
    // @x18n-key / @x18n-keys でキーを明示する（docs/i18n.md）。
    "dynamic-key": "error",
    // 他ロケールのキー欠落も CI で落とす（locale.test.ts と二重の担保）。
    "missing-translation": "error",
  },
});
