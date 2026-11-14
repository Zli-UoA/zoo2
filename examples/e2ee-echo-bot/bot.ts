// An echo bot for encrypted channels. See README.md.
import { ZooApp, type Credentials } from "../../sdk/src/index.ts";
import { ZooE2ee, fileStore, loadZooCrypto } from "../../sdk/src/e2ee.ts";
import { handleEvent } from "./echo.ts";

const baseUrl = process.env.ZOO_URL;
const storageKey = Buffer.from(process.env.ZOO_E2EE_STORAGE_KEY ?? "", "base64");
if (!baseUrl) throw new Error("Set ZOO_URL");
if (storageKey.length !== 32) throw new Error("Set ZOO_E2EE_STORAGE_KEY to 32 random bytes in base64 (openssl rand -base64 32)");

// Zoo rotates the refresh token on every refresh, so keep the latest credentials in a file (owner-only,
// written atomically) and prefer them over the environment, which holds only the first ones.
const credentialStore = fileStore(process.env.ZOO_CREDENTIALS_FILE ?? "credentials.json");
const saved = await credentialStore.load();
const access = process.env.ZOO_ACCESS_TOKEN;
const refresh = process.env.ZOO_REFRESH_TOKEN;
const credentials: Credentials | null = saved ? (JSON.parse(saved) as Credentials) : access && refresh ? { access_token: access, refresh_token: refresh } : null;
if (!credentials) throw new Error("Set ZOO_ACCESS_TOKEN and ZOO_REFRESH_TOKEN for the first start");

const app = new ZooApp({ baseUrl, credentials, saveCredentials: (next) => credentialStore.save(JSON.stringify(next)) });
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
