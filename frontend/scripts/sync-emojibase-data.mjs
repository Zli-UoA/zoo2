// Copies the Emojibase locale data served from public/emojibase-data.
// Usage: pnpm sync:emojibase
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { pinyin } from "pinyin-pro";

const require = createRequire(import.meta.url);
const sourceRoot = dirname(require.resolve("emojibase-data/package.json"));
const packageJson = JSON.parse(await readFile(join(sourceRoot, "package.json"), "utf8"));
const targetRoot = fileURLToPath(new URL("../public/emojibase-data/", import.meta.url));
const locales = ["en", "ja", "ko", "zh-hant"];
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

// Search index used to find emojis in any supported language regardless of the
// app language: labels and tags of every locale below, plus toneless pinyin
// (spaced, joined and initials) generated from the Chinese labels and tags.
const searchLocales = ["en", "ja", "ko", "zh", "zh-hant"];
const chinese = /\p{Script=Han}/u;
const searchIndex = {};
const addKeywords = (emoji, words) => {
  const set = (searchIndex[emoji] ??= new Set());
  for (const word of words) {
    const normalized = word.trim().toLowerCase();
    if (normalized) set.add(normalized);
  }
};
for (const locale of searchLocales) {
  const data = JSON.parse(await readFile(join(sourceRoot, locale, "data.json"), "utf8"));
  for (const entry of data) {
    if (!("group" in entry)) continue;
    const words = [entry.label, ...(entry.tags ?? [])];
    addKeywords(entry.emoji, words);
    if (locale === "zh" || locale === "zh-hant") {
      for (const word of words.filter((w) => chinese.test(w))) {
        const syllables = pinyin(word, { toneType: "none", type: "array", nonZh: "removed" });
        addKeywords(entry.emoji, [
          syllables.join(" "),
          syllables.join(""),
          // Single-letter initials would match almost every query.
          ...(syllables.length > 1 ? [syllables.map((syllable) => syllable[0]).join("")] : []),
        ]);
      }
    }
  }
}
await writeFile(
  join(targetRoot, "search-index.json"),
  JSON.stringify(Object.fromEntries(Object.entries(searchIndex).map(([k, v]) => [k, [...v]]))),
);
console.log(`search-index.json: ${Object.keys(searchIndex).length} emojis`);
