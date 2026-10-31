import { readFile, writeFile } from "node:fs/promises";
import { chromium } from "@playwright/test";

// All installed icons are rasterized from the same SVG used by the app and favicon.
// The mark fits within the central 80% diameter circle required for maskable icons.
const publicRoot = new URL("../public/", import.meta.url);
const svg = await readFile(new URL("icons/zoo.svg", publicRoot), "utf8");
const badge = svg.replace(/<rect[^>]+\/>/, "").replaceAll("#2C2C2C", "#FFFFFF");
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage();
  async function rasterize(source, size) {
    const png = await page.evaluate(
      async ({ source, size }) => {
        const image = new Image();
        image.src = `data:image/svg+xml;base64,${btoa(source)}`;
        await image.decode();
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = size;
        canvas.getContext("2d").drawImage(image, 0, 0, size, size);
        return canvas.toDataURL("image/png").split(",")[1];
      },
      { source, size },
    );
    return Buffer.from(png, "base64");
  }
  for (const size of [180, 192, 512]) {
    await writeFile(new URL(`icons/icon-${size}.png`, publicRoot), await rasterize(svg, size));
  }
  await writeFile(new URL("icons/favicon-32.png", publicRoot), await rasterize(svg, 32));
  // Notification badges use the mark's alpha silhouette; a filled tile becomes a square on Android.
  await writeFile(new URL("icons/badge-96.png", publicRoot), await rasterize(badge, 96));
  const sizes = [16, 32, 48];
  const images = await Promise.all(sizes.map((size) => rasterize(svg, size)));
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2); // ICO
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  images.forEach((png, index) => {
    const entry = 6 + index * 16;
    header[entry] = header[entry + 1] = sizes[index];
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(png.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += png.length;
  });
  await writeFile(new URL("favicon.ico", publicRoot), Buffer.concat([header, ...images]));
} finally {
  await browser.close();
}
