import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createSecureServer } from "node:http2";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

// 本番のTLS終端と同様にHTTP/2を提供する。HTTP/1.1へ直接接続すると、
// 6本のShape SSEがブラウザの接続枠を使い切り、REST送信が待たされる。
const port = Number(process.env.E2E_HTTP2_PORT ?? "8443");
const upstream = process.env.E2E_API_UPSTREAM ?? "http://localhost:8000";
const directory = mkdtempSync(join(tmpdir(), "zoo-e2e-tls-"));
const keyPath = join(directory, "key.pem");
const certPath = join(directory, "cert.pem");
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
    keyPath,
    "-out",
    certPath,
  ],
  { stdio: "ignore" },
);
const server = createSecureServer({
  key: readFileSync(keyPath),
  cert: readFileSync(certPath),
  allowHTTP1: true,
});
rmSync(directory, { recursive: true, force: true });

// HTTP/2で禁止されるhop-by-hopヘッダを双方で除去する。
const hopByHop = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
  "http2-settings",
]);
function forwardHeaders(headers: Record<string, string | string[] | undefined>) {
  const excluded = new Set([
    ...hopByHop,
    ...String(headers.connection ?? "")
      .toLowerCase()
      .split(",")
      .map((value) => value.trim()),
  ]);
  return Object.fromEntries(
    Object.entries(headers).filter(
      ([name]) => !name.startsWith(":") && !excluded.has(name.toLowerCase()),
    ),
  );
}
server.on("request", (incoming, outgoing) => {
  const target = new URL(incoming.url ?? "/", upstream);
  const forwarded = request(
    target,
    {
      method: incoming.method,
      headers: { ...forwardHeaders(incoming.headers), host: target.host },
    },
    (response) => {
      outgoing.writeHead(response.statusCode ?? 502, forwardHeaders(response.headers));
      response.on("error", () => outgoing.destroy());
      response.pipe(outgoing);
    },
  );
  forwarded.on("error", () => {
    if (!outgoing.headersSent) outgoing.writeHead(502);
    outgoing.end();
  });
  outgoing.on("close", () => forwarded.destroy());
  incoming.pipe(forwarded);
});
server.listen(port, "127.0.0.1", () => {
  console.log(`E2E HTTP/2 API proxy: https://localhost:${port}`);
});
