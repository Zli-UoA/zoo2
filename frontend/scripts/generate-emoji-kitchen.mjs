// Usage: node scripts/generate-emoji-kitchen.mjs /path/to/metadata.json
// Source: https://github.com/xsalazar/emoji-kitchen-backend/blob/main/app/metadata.json
import { readFile, writeFile } from "node:fs/promises";

const metadata = JSON.parse(await readFile(process.argv[2], "utf8"));
const entries = Object.values(metadata.data).sort((a, b) => a.gBoardOrder - b.gBoardOrder);
const emojis = entries.map(({ emojiCodepoint, emoji, alt }) => [emojiCodepoint, emoji, alt]);
const indices = new Map(emojis.map(([codepoint], index) => [codepoint, index]));
const dates = [];
const pairs = {};
for (const entry of entries) {
  for (const combinations of Object.values(entry.combinations)) {
    const latest = combinations.find((combination) => combination.isLatest) ?? combinations.at(-1);
    const left = indices.get(latest.leftEmojiCodepoint);
    const right = indices.get(latest.rightEmojiCodepoint);
    if (left === undefined || right === undefined) throw new Error("Unknown emoji");
    const date = latest.date;
    if (!dates.includes(date)) dates.push(date);
    // Keep the source orientation: Google does not serve both URL permutations.
    const key = `${Math.min(left, right)}_${Math.max(left, right)}`;
    pairs[key] = dates.indexOf(date) * 2 + Number(left > right);
  }
}
await writeFile(
  new URL("../src/lib/emoji/emoji-kitchen.json", import.meta.url),
  JSON.stringify({ emojis, dates, pairs }) + "\n",
);
console.log(`${emojis.length} emojis, ${Object.keys(pairs).length} combinations`);
