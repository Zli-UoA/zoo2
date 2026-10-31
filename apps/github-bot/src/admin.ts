import { Store } from "./store.ts";
const key = Buffer.from(process.env.BOT_ENCRYPTION_KEY ?? "", "base64");
if (key.length !== 32) throw new Error("Set BOT_ENCRYPTION_KEY");
const store = new Store(process.env.BOT_DATABASE ?? "data/github-bot.sqlite", key);
if (process.argv[2] === "retry" && process.argv[3]) {
  if (store.workerActive()) throw new Error("Stop the worker before retrying jobs");
  const result = store.db
    .prepare(
      "UPDATE jobs SET status='queued',attempts=0,available=?,error=NULL WHERE id=? AND status='dead'",
    )
    .run(Date.now(), process.argv[3]);
  console.info(JSON.stringify({ requeued: Number(result.changes) }));
} else
  console.info(
    JSON.stringify(
      store.db
        .prepare(
          "SELECT id,source,event,status,attempts,error,created FROM jobs WHERE status='dead' ORDER BY created LIMIT 100",
        )
        .all(),
    ),
  );
store.close();
