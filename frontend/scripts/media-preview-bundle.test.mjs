import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = path.resolve(
  frontend,
  "../frontend-compose/build/generated/media-preview-assets/media-preview",
);
let source, sourceReads;
const mime = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".css": "text/css",
  ".woff2": "font/woff2",
};
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://localhost");
    if (url.pathname === "/source") {
      sourceReads++;
      response.setHeader("Content-Type", source.type);
      response.end(await readFile(path.join(frontend, ".storybook/fixtures", source.name)));
      return;
    }
    const file = path.resolve(output, "." + decodeURIComponent(url.pathname));
    if (!file.startsWith(output + path.sep)) throw new Error("Invalid asset path");
    response.setHeader("Content-Type", mime[path.extname(file)] ?? "application/octet-stream");
    response.end(await readFile(file));
  } catch {
    response.writeHead(404);
    response.end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({
  executablePath: process.env.CHROME_BIN,
  headless: true,
  args: ["--no-sandbox"],
});
try {
  const cases = [
    ["sample.eml", "message/rfc822", "Hello from email."],
    ["sample.msg", "application/vnd.ms-outlook", "A memo."],
    [
      "sample.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      "Document preview",
    ],
    ["sample.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "Example"],
    [
      "sample.pptx",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      "Slide preview",
    ],
    ["sample.odt", "application/vnd.oasis.opendocument.text", "OpenDocument preview"],
    ["sample.zip", "application/zip", "readme.txt"],
    ["media.pdf", "application/pdf", "1 / 12"],
  ];
  for (const [name, type, expected] of cases) {
    source = { name, type };
    sourceReads = 0;
    const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
    const external = [],
      errors = [];
    await context.route("**/*", (route) => {
      if (
        route
          .request()
          .url()
          .startsWith(origin + "/") ||
        route.request().url().startsWith("blob:")
      )
        return route.continue();
      external.push(route.request().url());
      return route.abort();
    });
    await context.addInitScript(() => {
      window.savedPreviews = [];
      window.NativePreview = {
        save: (...args) => window.savedPreviews.push(args),
        download: () => {},
      };
    });
    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(origin + "/media-preview.html");
    await page.waitForFunction(() => !!window.openMediaPreview);
    await page.evaluate(([name, type]) => window.openMediaPreview(name, type, "ja"), [name, type]);
    await page.getByText(expected, { exact: false }).first().waitFor();
    if (name === "sample.zip") {
      await page.getByRole("button", { name: "readme.txtをプレビュー" }).click();
      await page.getByText("Archive text").waitFor();
      await page.getByRole("dialog").getByRole("link", { name: "ダウンロード" }).click();
      await page.waitForFunction(() => window.savedPreviews.length === 1);
      const saved = await page.evaluate(() => window.savedPreviews[0]);
      assert.equal(saved[0], "readme.txt");
      assert.equal(Buffer.from(saved[2], "base64").toString(), "Archive text");
    }
    if (name === "media.pdf") {
      await page.getByRole("button", { name: "次のページ" }).click();
      await page.getByText("2 / 12").waitFor();
      await page.waitForFunction(() => document.querySelector("canvas")?.width > 300);
      assert.equal(sourceReads, 1, "page changes must reuse the loaded PDF");
    }
    assert.deepEqual(external, [], "previews must not load external tracking resources");
    assert.deepEqual(errors, [], "the APK viewer bundle must run without page errors");
    await context.close();
    console.log(`Passed APK viewer bundle: ${name}`);
  }
} finally {
  await browser.close();
  await new Promise((resolve) => server.close(resolve));
}
