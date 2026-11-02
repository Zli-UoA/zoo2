import { createServer, type IncomingMessage } from "node:http";
import { createHmac, timingSafeEqual, createHash } from "node:crypto";
import { verifySignature } from "../../../sdk/src/index.ts";
import { GITHUB_EVENTS } from "./events.ts";
import { Store } from "./store.ts";
import { BotError, repoName, ruleFor, type Config, type Json } from "./types.ts";
export const ZOO_EVENTS = ["command.invoked", "link.shared", "app.uninstalled", "tokens.revoked"];
export function githubSignature(secret: string, body: Buffer, signature: string): boolean {
  if (!/^sha256=[a-f0-9]{64}$/.test(signature)) return false;
  return timingSafeEqual(
    createHmac("sha256", secret).update(body).digest(),
    Buffer.from(signature.slice(7), "hex"),
  );
}
async function readBody(request: IncomingMessage): Promise<Buffer> {
  const declared = Number(request.headers["content-length"] ?? 0);
  if (declared > 25 * 1024 * 1024) throw new BotError("body_too_large");
  let size = 0;
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 25 * 1024 * 1024) throw new BotError("body_too_large");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
export function server(config: Config, store: Store) {
  let active = 0;
  const http = createServer(async (request, response) => {
    const send = (status: number, body: unknown = {}) =>
      response
        .writeHead(status, { "content-type": "application/json", "cache-control": "no-store" })
        .end(JSON.stringify(body));
    if (request.method === "GET" && request.url === "/health") {
      send(store.get("zoo-disabled") ? 503 : 200, {
        status: store.get("zoo-disabled") ? "disabled" : "ok",
      });
      return;
    }
    if (
      request.method !== "POST" ||
      !["/github/webhook", "/zoo/events"].includes(request.url ?? "")
    ) {
      request.resume();
      send(404);
      return;
    }
    if (active >= 4) {
      request.resume();
      send(503);
      return;
    }
    active++;
    try {
      if (!/^application\/json(?:\s*;|$)/i.test(String(request.headers["content-type"] ?? ""))) {
        request.resume();
        send(415);
        return;
      }
      const body = await readBody(request),
        raw = body.toString("utf8");
      if (request.url === "/github/webhook") {
        if (
          !githubSignature(
            config.githubSecret,
            body,
            String(request.headers["x-hub-signature-256"] ?? ""),
          )
        ) {
          send(401);
          return;
        }
        const delivery = String(request.headers["x-github-delivery"] ?? ""),
          event = String(request.headers["x-github-event"] ?? "");
        if (!/^[a-z\d-]{1,128}$/i.test(delivery) || !event) {
          send(400);
          return;
        }
        const payload = JSON.parse(raw);
        if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
          send(400);
          return;
        }
        if (!GITHUB_EVENTS.includes(event)) {
          send(202);
          return;
        }
        // These lifecycle notifications have no repository, but must still match our installation.
        const lifecycle = ["installation", "installation_repositories", "ping"].includes(event);
        if (!config.rules.some((r) => r.installationId === Number(payload.installation?.id))) {
          send(202);
          return;
        }
        if (!lifecycle) {
          let repo: string;
          try {
            repo = repoName(payload.repository?.full_name ?? "");
          } catch {
            send(202);
            return;
          }
          const rule = ruleFor(config, repo);
          if (!rule || rule.installationId !== Number(payload.installation.id)) {
            send(202);
            return;
          }
        }
        store.enqueue(
          `github:${delivery}`,
          "github",
          event,
          payload,
          createHash("sha256").update(`${event}:`).update(body).digest("hex"),
        );
      } else {
        if (
          !(await verifySignature(
            config.zooSecret,
            String(request.headers["x-zoo-timestamp"] ?? ""),
            raw,
            String(request.headers["x-zoo-signature"] ?? ""),
          ))
        ) {
          send(401);
          return;
        }
        const delivery = JSON.parse(raw);
        if (
          delivery?.type === "url_verification" &&
          typeof delivery.challenge === "string" &&
          delivery.challenge.length <= 1024
        ) {
          send(200, { challenge: delivery.challenge });
          return;
        }
        const event = delivery?.event;
        if (
          !event ||
          typeof event.id !== "string" ||
          !/^[a-z\d-]{1,128}$/i.test(event.id) ||
          typeof event.type !== "string" ||
          !event.data ||
          typeof event.data !== "object" ||
          Array.isArray(event.data)
        ) {
          send(400);
          return;
        }
        if (!ZOO_EVENTS.includes(event.type)) {
          send(202);
          return;
        }
        if (
          ["command.invoked", "link.shared"].includes(event.type) &&
          (typeof event.actor_id !== "string" || typeof event.channel_id !== "string")
        ) {
          send(400);
          return;
        }
        store.enqueue(`zoo:${event.id}`, "zoo", event.type, event);
      }
      // Durable acceptance precedes ACK; business processing is independent of webhook timeouts.
      send(202);
    } catch (error) {
      const code =
        error instanceof BotError
          ? error.code
          : error instanceof SyntaxError
            ? "invalid_json"
            : "request_failed";
      send(
        code === "queue_full"
          ? 503
          : code === "body_too_large"
            ? 413
            : code === "invalid_json"
              ? 400
              : 500,
        { error: code },
      );
    } finally {
      active--;
    }
  });
  http.requestTimeout = 10000;
  http.headersTimeout = 10000;
  http.maxConnections = 128;
  return http;
}
