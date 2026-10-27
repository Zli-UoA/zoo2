import { createServer } from "node:http";
import { ZooApp, verifySignature, type Delivery } from "../../sdk/src/index.ts";
const secret = process.env.ZOO_SIGNING_SECRET;
if (!secret) throw new Error("Set ZOO_SIGNING_SECRET");
const app = new ZooApp({ baseUrl: process.env.ZOO_URL!, credentials: { access_token: process.env.ZOO_ACCESS_TOKEN!, refresh_token: process.env.ZOO_REFRESH_TOKEN! } });
createServer(async (request, response) => {
  if (request.method !== "POST" || request.url !== "/events") { response.writeHead(404).end(); return; }
  try {
    const chunks: Buffer[] = []; let size = 0;
    for await (const chunk of request) { size += chunk.length; if (size > 2 * 1024 * 1024) { response.writeHead(413).end(); return; } chunks.push(chunk); }
    const body = Buffer.concat(chunks).toString("utf8");
    if (!await verifySignature(secret, String(request.headers["x-zoo-timestamp"] ?? ""), body, String(request.headers["x-zoo-signature"] ?? ""))) { response.writeHead(401).end(); return; }
    const payload = JSON.parse(body);
    if (payload.type === "url_verification" && typeof payload.challenge === "string") { response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ challenge: payload.challenge })); return; }
    const delivery = payload as Delivery;
    await app.dispatch(delivery, async (event) => {
      if (event.type === "app.mentioned" && event.channel_id) await app.postMessage({ channel_id: event.channel_id, content: "HTTP Botからの返信です。", parent_id: String(event.data.parent_id ?? event.data.id) });
    });
    response.writeHead(200).end();
  } catch { response.writeHead(500).end(); }
}).listen(Number(process.env.PORT ?? "3001"));
