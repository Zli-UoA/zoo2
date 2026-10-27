// Copies the Emojibase locale data served from public/emojibase-data.
// Usage: pnpm sync:emojibase
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const sourceRoot = dirname(require.resolve("emojibase-data/package.json"));
const packageJson = JSON.parse(await readFile(join(sourceRoot, "package.json"), "utf8"));
const targetRoot = fileURLToPath(new URL("../public/emojibase-data/", import.meta.url));
const locales = ["en", "ja", "zh-hant"];
const files = ["data.json", "messages.json"];

for (const locale of locales) {
  await mkdir(join(targetRoot, locale), { recursive: true });
  for (const file of files) {
    await copyFile(join(sourceRoot, locale, file), join(targetRoot, locale, file));
  }
}

await copyFile(join(sourceRoot, "LICENSE"), join(targetRoot, "LICENSE"));
console.log(
  `emojibase-data@${packageJson.version}: copied ${locales.join(", ")} to public/emojibase-data`,
);

// MFM accepts the same GitHub shortcodes as backend reaction validation.
const codes = JSON.parse(await readFile(join(sourceRoot, "en/shortcodes/github.json"), "utf8"));
const shortcodes = {};
for (const [hex, names] of Object.entries(codes)) {
  for (const name of [names].flat()) {
    shortcodes[name] = String.fromCodePoint(...hex.split("-").map((part) => parseInt(part, 16)));
  }
}
await writeFile(
  new URL("../src/lib/emoji/shortcodes.json", import.meta.url),
  JSON.stringify(shortcodes, null, 2) + "\n",
);
