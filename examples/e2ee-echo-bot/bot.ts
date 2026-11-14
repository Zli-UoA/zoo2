// An echo bot for encrypted channels. See README.md.
import { ZooApp } from "../../sdk/src/index.ts";
import { ZooE2ee, fileStore, loadZooCrypto } from "../../sdk/src/e2ee.ts";
import { handleEvent } from "./echo.ts";

const baseUrl = process.env.ZOO_URL;
const access = process.env.ZOO_ACCESS_TOKEN;
const refresh = process.env.ZOO_REFRESH_TOKEN;
const storageKey = Buffer.from(process.env.ZOO_E2EE_STORAGE_KEY ?? "", "base64");
if (!baseUrl || !access || !refresh) throw new Error("Set ZOO_URL, ZOO_ACCESS_TOKEN, and ZOO_REFRESH_TOKEN");
if (storageKey.length !== 32) throw new Error("Set ZOO_E2EE_STORAGE_KEY to 32 random bytes in base64 (openssl rand -base64 32)");

const app = new ZooApp({ baseUrl, credentials: { access_token: access, refresh_token: refresh } });
const e2ee = await ZooE2ee.open({
  app,
  crypto: await loadZooCrypto(),
  store: fileStore(process.env.ZOO_E2EE_STORE ?? "e2ee-store.json"),
  storageKey,
  deviceName: "e2ee-echo-bot",
});
console.log(`E2EE device ${e2ee.deviceId} is ready`);
const stop = new AbortController();
process.on("SIGINT", () => stop.abort());
await app.connect(async (event) => {
  await handleEvent(e2ee, event);
}, stop.signal);
e2ee.close();
