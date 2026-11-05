import { ZooApp } from "../../sdk/src/index.ts";
const baseUrl = process.env.ZOO_URL;
const access = process.env.ZOO_ACCESS_TOKEN;
const refresh = process.env.ZOO_REFRESH_TOKEN;
if (!baseUrl || !access || !refresh) throw new Error("Set ZOO_URL, ZOO_ACCESS_TOKEN, and ZOO_REFRESH_TOKEN");
const app = new ZooApp({ baseUrl, credentials: { access_token: access, refresh_token: refresh } });
const stop = new AbortController();
process.on("SIGINT", () => stop.abort());
await app.connect(async (event) => {
  if (event.type === "app.mentioned" && event.channel_id) {
    const root = String(event.data.parent_id ?? event.data.id);
    await app.postMessage({ channel_id: event.channel_id, content: "こんにちは！ Zoo Botからの返信です。", parent_id: root });
  }
}, stop.signal);
