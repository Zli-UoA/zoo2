import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
const root = resolve("storybook-static");
const mime = {
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".html": "text/html",
  ".json": "application/json",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".ttf": "font/ttf",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};
createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1:6008");
    const file = resolve(
      root,
      "." + decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname),
    );
    if (!file.startsWith(root + sep)) {
      res.writeHead(404).end();
      return;
    }
    const body = await readFile(file);
    res.setHeader("Content-Type", mime[extname(file)] ?? "application/octet-stream");
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
}).listen(6008, "127.0.0.1");
