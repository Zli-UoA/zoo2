import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, readdir } from "node:fs/promises";
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
  ".wasm": "application/wasm",
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
    ["cjk.pdf", "application/pdf", "1 / 1"],
    ["jpx.pdf", "application/pdf", "1 / 1"],
  ];
  for (const [name, type, expected] of cases) {
    source = { name, type };
    sourceReads = 0;
    const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
    const responses = [];
    context.on("response", (response) =>
      responses.push({ url: response.url(), status: response.status() }),
    );
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
    assert.equal(
      await page.getByRole("dialog").count(),
      0,
      "native host must not contain a second modal",
    );
    assert.equal(
      await page.locator('[data-slot="dialog-overlay"]').count(),
      0,
      "no blurred backdrop over the native viewer",
    );
    const bounds = await page.locator("#root > div").first().boundingBox();
    assert.ok(bounds.width <= 390 && bounds.height <= 844, "viewer must fit the native viewport");
    if (name === "sample.zip") {
      await page.getByRole("button", { name: "readme.txtをプレビュー" }).click();
      await page.getByText("Archive text").waitFor();
      await page.getByRole("dialog").getByRole("link", { name: "ダウンロード" }).click();
      await page.waitForFunction(() => window.savedPreviews.length === 1);
      await page.getByRole("button", { name: "閉じる", exact: true }).click();
      await page.getByRole("dialog").waitFor({ state: "detached" });
      await page.getByText("readme.txt", { exact: true }).first().waitFor();
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
    if (name === "cjk.pdf" || name === "jpx.pdf") {
      await page.getByRole("button", { name: "前のページ" }).waitFor();
      await page.waitForFunction(
        () =>
          !document.querySelector('[role="status"]') && document.querySelector("canvas")?.width > 0,
      );
      const painted = await page.locator("canvas").evaluate((canvas) => {
        const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
        return pixels.some((value, index) => index % 4 !== 3 && value < 200);
      });
      assert.ok(painted, `${name}: PDF contents must paint on the canvas`);
      assert.ok(
        responses.some(
          (response) =>
            response.status === 200 &&
            response.url.includes(
              name === "cjk.pdf" ? "/cmaps/90ms-RKSJ-H.bcmap" : "/wasm/openjpeg.wasm",
            ),
        ),
        `${name}: bundled decoder must be requested`,
      );
    }
    if (name === "cjk.pdf") {
      const pdfModule = (await readdir(path.join(output, "assets"))).find((file) =>
        /^pdf-.*\.js$/.test(file),
      );
      const bytes = [...(await readFile(path.join(frontend, ".storybook/fixtures/cjk.pdf")))];
      const text = await page.evaluate(
        async ({ moduleUrl, bytes }) => {
          const pdfjs = await import(moduleUrl);
          const task = pdfjs.getDocument({
            data: new Uint8Array(bytes),
            cMapUrl: `${location.origin}/assets/pdfjs/${pdfjs.version}/cmaps/`,
            cMapPacked: true,
            useSystemFonts: true,
          });
          try {
            const pdf = await task.promise;
            const page = await pdf.getPage(1);
            const content = await page.getTextContent();
            return content.items.map((item) => item.str ?? "").join("");
          } finally {
            await task.destroy();
          }
        },
        { moduleUrl: `${origin}/assets/${pdfModule}`, bytes },
      );
      assert.ok(text.includes("あい"), `Japanese CMap must decode characters: ${text}`);
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
