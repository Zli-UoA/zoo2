import { createSecureServer } from "node:http2";
import { request as upstreamRequest } from "node:https";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, extname } from "node:path";
const directory = mkdtempSync(join(tmpdir(), "zoo-compose-static-"));
execFileSync(
  "openssl",
  [
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-days",
    "1",
    "-subj",
    "/CN=localhost",
    "-addext",
    "subjectAltName=DNS:localhost,IP:127.0.0.1",
    "-keyout",
    join(directory, "key.pem"),
    "-out",
    join(directory, "cert.pem"),
  ],
  { stdio: "ignore" },
);
const tls = {
  key: readFileSync(join(directory, "key.pem")),
  cert: readFileSync(join(directory, "cert.pem")),
  allowHTTP1: true,
};
rmSync(directory, { recursive: true, force: true });
const root = resolve("dist/client");
const html = await readFile(join(root, "index.html"), "utf8");
const hashes = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)]
  .filter((m) => m[1].trim())
  .map((m) => `'sha256-${createHash("sha256").update(m[1]).digest("base64")}'`)
  .join(" ");
const nginx = await readFile("../infra/frontend-nginx.conf", "utf8");
const policy = nginx
  .match(/add_header Content-Security-Policy "([^"]+)" always;/)[1]
  .replaceAll("${CSP_SCRIPT_SRC}", hashes)
  .replaceAll("${CSP_COMPOSE_CONNECT_SRC}", " https://www.gstatic.com")
  .replace(/\$\{CSP_[A-Z0-9_]+\}/g, "");
const mime = {
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".json": "application/json",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".html": "text/html",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ttf": "font/ttf",
  ".woff2": "font/woff2",
  ".webmanifest": "application/manifest+json",
};
const forbidden = new Set(["connection", "keep-alive", "transfer-encoding", "upgrade", "host"]);
const headers = (object) =>
  Object.fromEntries(
    Object.entries(object).filter(([k]) => !k.startsWith(":") && !forbidden.has(k.toLowerCase())),
  );
createSecureServer(tls, async (req, res) => {
  const url = new URL(req.url, "https://127.0.0.1:4188");
  if (url.pathname.startsWith("/api/")) {
    const outgoing = upstreamRequest(
      new URL(url.pathname + url.search, "https://127.0.0.1:4189"),
      { method: req.method, headers: headers(req.headers), rejectUnauthorized: false },
      (response) => {
        res.writeHead(response.statusCode, headers(response.headers));
        response.pipe(res);
        res.on("close", () => outgoing.destroy());
      },
    );
    outgoing.on("error", () => {
      if (!res.headersSent) res.writeHead(502);
      res.end();
    });
    req.pipe(outgoing);
    return;
  }
  res.setHeader("Content-Security-Policy", policy);
  res.setHeader("Cache-Control", "no-cache");
  res.setHeader("X-Content-Type-Options", "nosniff");
  let file = resolve(root, "." + decodeURIComponent(url.pathname));
  if (!file.startsWith(root + "/") && file !== root) {
    res.writeHead(404).end();
    return;
  }
  try {
    const body = await readFile(file);
    res.setHeader("Content-Type", mime[extname(file)] ?? "application/octet-stream");
    res.end(body);
  } catch {
    if (url.pathname.startsWith("/assets/") || url.pathname.startsWith("/compose/")) {
      res.writeHead(404).end();
      return;
    }
    res.setHeader("Content-Type", "text/html");
    res.end(html);
  }
}).listen(4188, "127.0.0.1");
