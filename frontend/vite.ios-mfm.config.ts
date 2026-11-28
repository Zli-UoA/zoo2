import { defineConfig } from "vite-plus";
import { readFileSync, readdirSync } from "node:fs";
import { MFM_FUNCTIONS } from "./src/components/chat/messages/mfmFunctions";
import { insertMfm } from "./src/components/chat/messages/mfmSuggestions";
import ja from "./src/lib/i18n/locales/ja.json";

// MFM本体と数式フォントを単一HTMLに同梱し、表示時の通信を不要にする。
export default defineConfig({
  publicDir: false,
  plugins: [
    {
      name: "ios-mfm-document",
      generateBundle: {
        order: "post",
        handler(_, bundle) {
          const script = Object.values(bundle).find((file) => file.type === "chunk");
          const css = Object.values(bundle).find(
            (file) => file.type === "asset" && file.fileName.endsWith(".css"),
          );
          if (script?.type !== "chunk" || css?.type !== "asset")
            throw new Error("MFMの生成物が不足しています");
          const policy =
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'";
          const cssText =
            typeof css.source === "string" ? css.source : new TextDecoder().decode(css.source);
          const html = `<!doctype html><html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1"><meta http-equiv="Content-Security-Policy" content="${policy}"><style>${cssText}</style></head><body><main id="content" class="mfm-body"></main><script>${script.code.replace(/<\/script/gi, "<\\/script")}</script></body></html>`;
          for (const key of Object.keys(bundle)) delete bundle[key];
          this.emitFile({ type: "asset", fileName: "mfm.html", source: html });
          const files = [
            "mfm-js.txt",
            "katex.txt",
            "highlight-js.txt",
            ...readdirSync("public/licenses/misskey-emoji-data").map(
              (name) => `misskey-emoji-data/${name}`,
            ),
          ];
          this.emitFile({
            type: "asset",
            fileName: "mfm-licenses.txt",
            source: files
              .map((name) => `${name}\n\n${readFileSync(`public/licenses/${name}`, "utf8")}`)
              .join("\n\n"),
          });
          this.emitFile({
            type: "asset",
            fileName: "mfm-functions.json",
            source: JSON.stringify(
              MFM_FUNCTIONS.map((name) => ({
                name,
                description: name === "blur" ? "ぼかし（タップで解除）" : ja.mfm.functions[name],
                template: insertMfm("", { start: 0, end: 0, query: "" }, name).value,
              })),
            ),
          });
        },
      },
    },
  ],
  build: {
    target: "es2022",
    outDir: "../ios/Sources/Resources/Mfm",
    emptyOutDir: true,
    lib: {
      entry: "src/ios-mfm/renderer.ts",
      formats: ["iife"],
      name: "ZooMfm",
      fileName: () => "renderer.js",
    },
  },
});
