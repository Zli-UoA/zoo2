import * as SQLite from "@journeyapps/wa-sqlite";
import { verifyDeviceKeys, type EncryptedMessage, type JsonObject } from "@zoo/crypto-wasm";

import { cryptoDatabaseName } from "../identity";
import { MemoryCryptoBackend } from "../store/backend";
import { CryptoStore } from "../store/cryptoStore";
import { CRYPTO_SCHEMA } from "../store/schema";
import type { WrappingKeyStore } from "../store/storageKey";
import { createMemoryVfs } from "./snapshotBackend";

/**
 * Push の復号のテスト（node の `pushDecrypt.test.ts` と、ブラウザーの `sw/push.browser.test.ts`）で
 * 共有する準備。zoo-crypto の wasm は呼び出し側で読み込んでおく。
 */

export const ALICE = "00000000-0000-4000-8000-00000000000a";
export const CHANNEL = "00000000-0000-4000-8000-0000000000c1";
const NO_MENTIONS = { user_ids: [], channel: false, here: false };

export function memoryKeys(): WrappingKeyStore {
  const keys = new Map<string, CryptoKey>();
  return {
    get: async (name) => keys.get(name),
    put: async (name, key) => {
      keys.set(name, key);
    },
    delete: async (name) => {
      keys.delete(name);
    },
  };
}

async function openStore(userId: string, name: string, keys: WrappingKeyStore = memoryKeys()) {
  const backend = new MemoryCryptoBackend();
  const { store } = await CryptoStore.open({
    backend,
    name,
    userId,
    keys,
    reset: async () => {
      await backend.destroy();
      return backend;
    },
  });
  return { store, backend, keys };
}

/**
 * Alice が Bob に鍵を配り、`body` を暗号化した行（サーバーが Push に入れる `e2ee.message` と同じ）を返す。
 * Bob の暗号ストアは、to-device を処理して受信用セッションを保存した状態。
 */
export async function exchange(
  body: string,
  bobId: string,
  bobKeys: WrappingKeyStore = memoryKeys(),
) {
  const BOB = bobId;
  const alice = await openStore(ALICE, cryptoDatabaseName(`user-${ALICE}`));
  const bob = await openStore(BOB, cryptoDatabaseName(`user-${BOB}`), bobKeys);
  const request = bob.store.registrationRequest();
  const [keyId, oneTimeKey] = Object.entries(request.one_time_keys)[0]!;
  alice.store.megolm.setDistribution(CHANNEL, 2, [
    verifyDeviceKeys(request.device_keys, BOB, bob.store.deviceId),
  ]);
  await alice.store.megolm.start(CHANNEL);
  const { messages } = await alice.store.megolm.share(CHANNEL, {
    [BOB]: { [bob.store.deviceId]: { [keyId]: oneTimeKey } },
  });
  await bob.store.processToDevice(
    [
      {
        id: "1",
        sender_user_id: ALICE,
        sender_device_id: alice.store.deviceId,
        type: "m.room.encrypted",
        content: messages[BOB]![bob.store.deviceId] as JsonObject,
      },
    ],
    { [ALICE]: { [alice.store.deviceId]: alice.store.deviceKeys() } },
  );
  const encrypted = alice.store.megolm.encrypt(CHANNEL, {
    message_id: "m1",
    parent_id: null,
    revision: 0,
    content: { body, format: "mfm", kind: "message" },
    mentions: NO_MENTIONS,
  });
  const row: EncryptedMessage = {
    channel_id: CHANNEL,
    message_id: "m1",
    parent_id: null,
    user_id: ALICE,
    message_kind: "message",
    epoch: encrypted.epoch,
    revision: 0,
    ciphertext: encrypted.ciphertext,
  };
  await alice.store.close();
  await bob.store.close();
  return { row, bob };
}

/** Bob の暗号ストアを、OPFS に置くのと同じ表の SQLite のファイル（バイト列）にする。 */
export async function sqliteFile(
  module: unknown,
  backend: MemoryCryptoBackend,
): Promise<ArrayBuffer> {
  const sqlite3 = SQLite.Factory(module);
  const vfs = await createMemoryVfs("writer", module);
  sqlite3.vfs_register(vfs, false);
  const db = await sqlite3.open_v2("/crypto.sqlite", undefined, "writer");
  await sqlite3.exec(db, CRYPTO_SCHEMA);
  const run = async (sql: string, params: SQLiteCompatibleType[]) => {
    for await (const stmt of sqlite3.statements(db, sql)) {
      sqlite3.bind_collection(stmt, params);
      await sqlite3.step(stmt);
    }
  };
  for (const [key, value] of backend.meta) {
    await run("INSERT INTO meta (key, value) VALUES (?, ?)", [key, value]);
  }
  await run("INSERT INTO account (id, pickle) VALUES (1, ?)", [backend.account!]);
  for (const [key, { pickle }] of backend.inboundSessions) {
    await run("INSERT INTO inbound_sessions (key, pickle) VALUES (?, ?)", [key, pickle]);
  }
  await sqlite3.close(db);
  const file = vfs.mapNameToFile.get("/crypto.sqlite")!;
  return file.data.slice(0, file.size);
}
