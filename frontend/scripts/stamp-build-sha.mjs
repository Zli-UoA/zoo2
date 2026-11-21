import { readFileSync, writeFileSync } from "node:fs";

const [, , htmlPath] = process.argv;
const sha = process.env.ZOO_BUILD_SHA;
if (sha) {
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error("Invalid ZOO_BUILD_SHA");

  const html = readFileSync(htmlPath, "utf8");
  const placeholder = 'data-zoo-build-sha=""';
  if (!html.includes(placeholder)) throw new Error(`Missing build SHA placeholder in ${htmlPath}`);

  writeFileSync(htmlPath, html.replace(placeholder, `data-zoo-build-sha="${sha}"`));
}
