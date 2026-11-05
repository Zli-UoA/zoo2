import { readFile, writeFile } from "node:fs/promises";
const svg = await readFile(new URL("../public/icons/zoo.svg", import.meta.url), "utf8");
const paths = [...svg.matchAll(/<path\s+d="([^"]+)"/g)].map((match) => match[1]);
if (paths.length !== 4)
  throw Error("Zoo SVG geometry changed; update the native transform with the source mark.");
await writeFile(
  new URL("../../frontend-compose/src/commonMain/kotlin/zoo/BrandData.kt", import.meta.url),
  "// Generated from frontend/public/icons/zoo.svg by frontend/scripts/sync-compose-brand.mjs.\npackage zoo\ninternal val zooBrandPaths = listOf(" +
    paths.map((value) => JSON.stringify(value)).join(",") +
    ")\n",
);
