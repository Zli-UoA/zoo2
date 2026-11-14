import SQLiteESMFactory from "@journeyapps/wa-sqlite/dist/wa-sqlite.mjs";
import sqliteWasmUrl from "@journeyapps/wa-sqlite/dist/wa-sqlite.wasm?url";
import init from "@zoo/crypto-wasm";
import { afterAll, beforeAll, describe, expect, it } from "vite-plus/test";

import { cryptoDatabaseName, cryptoLockName } from "../lib/e2ee/identity";
import { answerPushDecryptRequest, type PushDecryptWorker } from "../lib/e2ee/push/protocol";
import { startCryptoWorker } from "../lib/e2ee/cryptoWorker";
import { exchange, sqliteFile } from "../lib/e2ee/push/pushTestKit";
import { openSnapshotBackend, readCryptoDatabaseFiles } from "../lib/e2ee/push/snapshotBackend";
import { indexedDbWrappingKeys } from "../lib/e2ee/store/storageKey";

/**
 * 本物の Service Worker（Chromium）で、Push の `e2ee` を復号する（#365）。
 *
 * - 暗号ストアを開いているタブがない：Service Worker が Web Locks を取り、OPFS の暗号ストアの
 *   ファイルと IndexedDB の保存鍵を包む鍵を読んで復号する。
 * - 開いているタブがある（ロックを持っている）：Service Worker がタブに頼み、タブの答えを使う。
 */

const BOB = crypto.randomUUID();
const NAMESPACE = `user-${BOB}`;
const DB = cryptoDatabaseName(NAMESPACE);
let registration: ServiceWorkerRegistration;

async function activeWorker(): Promise<ServiceWorker> {
  registration = await navigator.serviceWorker.register(
    new URL("./push.harness.ts", import.meta.url),
    { type: "module", scope: "/src/sw/" },
  );
  const worker = registration.installing ?? registration.waiting ?? registration.active!;
  if (worker.state !== "activated") {
    await new Promise<void>((resolve) => {
      worker.addEventListener("statechange", () => {
        if (worker.state === "activated") resolve();
      });
    });
  }
  return worker;
}

function decryptInWorker(worker: ServiceWorker, e2ee: unknown): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const channel = new MessageChannel();
    channel.port1.onmessage = ({ data }: MessageEvent<{ body: string | null; error?: string }>) =>
      data.error ? reject(new Error(data.error)) : resolve(data.body);
    worker.postMessage(e2ee, [channel.port2]);
  });
}

async function writeOpfsFile(name: string, bytes: ArrayBuffer) {
  const root = await navigator.storage.getDirectory();
  const handle = await root.getFileHandle(name, { create: true });
  const writable = await (
    handle as unknown as {
      createWritable(): Promise<{
        write(data: ArrayBuffer): Promise<void>;
        close(): Promise<void>;
      }>;
    }
  ).createWritable();
  await writable.write(bytes);
  await writable.close();
}

let row: Awaited<ReturnType<typeof exchange>>["row"];
const SECRET = "通知に出す秘密の本文 🦊";

beforeAll(async () => {
  await init();
  // Bob の暗号ストアを、本物の IndexedDB の鍵で包み、OPFS に置く（Worker が置くのと同じ名前と表）。
  const result = await exchange(SECRET, BOB, indexedDbWrappingKeys);
  row = result.row;
  const sqlite = await SQLiteESMFactory({ locateFile: () => sqliteWasmUrl });
  await writeOpfsFile(DB, await sqliteFile(sqlite, result.bob.backend));
});

afterAll(async () => {
  await registration?.unregister();
  const root = await navigator.storage.getDirectory();
  await root.removeEntry(DB).catch(() => undefined);
  await indexedDbWrappingKeys.delete(DB);
});

describe("sw-push（Service Worker での Push の復号）", () => {
  it("開いているタブがなければ、Service Worker が暗号ストアを読んで復号する", async () => {
    const worker = await activeWorker();
    expect(await decryptInWorker(worker, { recipient_id: BOB, message: row })).toBe(SECRET);
    // 行と照合できない暗号文は復号しない
    expect(
      await decryptInWorker(worker, { recipient_id: BOB, message: { ...row, message_id: "x" } }),
    ).toBeNull();
  });

  it("暗号ストアを開いているタブがあれば、そのタブの答えを使う", async () => {
    const worker = await activeWorker();
    const requests: unknown[] = [];
    const tabWorker: PushDecryptWorker = {
      request: async (operation) => {
        requests.push(operation);
        return [{ status: "decrypted", body: "タブで復号した本文" }];
      },
    };
    const onMessage = (event: MessageEvent) =>
      answerPushDecryptRequest(event.data, event.ports[0], {
        worker: tabWorker,
        namespace: NAMESPACE,
      });
    navigator.serviceWorker.addEventListener("message", onMessage);
    navigator.serviceWorker.startMessages();
    try {
      // タブが暗号ストアのロックを持っているあいだは、Service Worker はファイルを読まない。
      const body = await navigator.locks.request(cryptoLockName(NAMESPACE), () =>
        decryptInWorker(worker, { recipient_id: BOB, message: row }),
      );
      expect(body).toBe("タブで復号した本文");
      expect(requests).toEqual([{ op: "megolmDecrypt", rows: [row] }]);
    } finally {
      navigator.serviceWorker.removeEventListener("message", onMessage);
    }
  });

  it("暗号ストアの Worker が OPFS に書いたファイルを、写しとして読める", async () => {
    const user = crypto.randomUUID();
    const namespace = `user-${user}`;
    const worker = startCryptoWorker();
    try {
      const opened = await worker.request({ op: "open", namespace, userId: user });
      await worker.request({ op: "close" });
      const files = await readCryptoDatabaseFiles(cryptoDatabaseName(namespace));
      expect(files).not.toBeNull();
      const sqlite = await SQLiteESMFactory({ locateFile: () => sqliteWasmUrl });
      const snapshot = await openSnapshotBackend(sqlite, files!);
      const read = await snapshot.read();
      await snapshot.close();
      expect(read.meta).toMatchObject({ user_id: user, device_id: opened.deviceId });
      expect(read.account).toEqual(expect.any(String));
      expect(
        await readCryptoDatabaseFiles(cryptoDatabaseName(`user-${crypto.randomUUID()}`)),
      ).toBeNull();
      await worker.request({ op: "destroy", namespace });
    } finally {
      worker.terminate();
    }
  });
});
