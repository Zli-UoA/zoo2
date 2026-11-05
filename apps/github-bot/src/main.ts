import { createHash } from "node:crypto";
import { ZooApp, type Credentials } from "../../../sdk/src/index.ts";
import { loadConfig } from "./config.ts";
import { Store } from "./store.ts";
import { GitHub } from "./github.ts";
import { GitHubBot } from "./bot.ts";
import { server } from "./server.ts";
import type { Json } from "./types.ts";

async function main() {
  process.umask(0o077);
  const config = loadConfig(),
    store = new Store(config.database, config.encryptionKey);
  const worker = store.acquireWorker(),
    stop = new AbortController();
  let bot: GitHubBot | undefined,
    http: ReturnType<typeof server> | undefined,
    timer: ReturnType<typeof setInterval> | undefined;
  let closing = false;
  async function shutdown() {
    if (closing) return;
    closing = true;
    clearInterval(heartbeat);
    if (timer) clearInterval(timer);
    stop.abort();
    const closed = http?.listening
      ? new Promise<void>((resolve) => http!.close(() => resolve()))
      : Promise.resolve();
    http?.closeIdleConnections();
    await bot?.stop();
    await closed;
    store.releaseWorker(worker);
    store.close();
  }
  const heartbeat = setInterval(() => {
    if (!store.heartbeat(worker)) {
      console.error("Worker lease lost");
      process.exitCode = 1;
      void shutdown();
    }
  }, 5000);
  process.once("SIGTERM", () => {
    void shutdown();
  });
  process.once("SIGINT", () => {
    void shutdown();
  });
  try {
    const access = process.env.ZOO_ACCESS_TOKEN,
      refresh = process.env.ZOO_REFRESH_TOKEN;
    if (Boolean(access) !== Boolean(refresh))
      throw new Error("Set ZOO_ACCESS_TOKEN and ZOO_REFRESH_TOKEN together");
    if (access && refresh) {
      const fingerprint = createHash("sha256").update(access).digest("hex");
      if (store.get("bootstrap-credential") !== fingerprint) {
        store.saveSecret("zoo", { access_token: access, refresh_token: refresh });
        store.set("bootstrap-credential", fingerprint);
        store.remove("zoo-disabled");
      }
    }
    const credentials = store.secret<Credentials>("zoo");
    if (!credentials)
      throw new Error("Set ZOO_ACCESS_TOKEN and ZOO_REFRESH_TOKEN for the first start");
    const zoo = new ZooApp({
      baseUrl: config.zooUrl,
      credentials,
      saveCredentials: async (value) => store.saveSecret("zoo", value),
      fetch: async (input, init) =>
        fetch(input, {
          ...init,
          redirect: "error",
          signal: AbortSignal.any([
            AbortSignal.timeout(15000),
            stop.signal,
            ...(init?.signal ? [init.signal] : []),
          ]),
        }),
    });
    const identity = await zoo.request<Json>("GET", "/auth");
    if (closing) return;
    if (
      store.get<Json>("zoo-identity")?.app_id &&
      store.get<Json>("zoo-identity")!.app_id !== identity.app_id
    )
      throw new Error("This database belongs to another Zoo App; use a separate database");
    store.set("zoo-identity", identity);
    const github = new GitHub(config, store);
    bot = new GitHubBot(config, store, github, zoo);
    http = server(config, store);
    http.on("error", () => {
      console.error("Bot HTTP server failed");
      process.exitCode = 1;
      void shutdown();
    });
    http.listen(config.port, config.host, () =>
      console.info(`GitHub notification bot listening on ${config.port}`),
    );
    timer = setInterval(() => {
      void bot!.tick().catch(() => {
        if (!stop.signal.aborted) console.error("Bot maintenance failed; retrying");
      });
    }, 250);
  } catch (error) {
    const interrupted = closing;
    await shutdown();
    if (!interrupted) throw error;
  }
}
main().catch((error) => {
  console.error(error instanceof Error ? error.message : "Bot startup failed");
  process.exitCode = 1;
});
