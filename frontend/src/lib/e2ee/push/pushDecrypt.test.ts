import { readFileSync } from "node:fs";

import SQLiteESMFactory from "@journeyapps/wa-sqlite/dist/wa-sqlite.mjs";
import { initSync, type EncryptedMessage, type JsonObject } from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it, vi } from "vite-plus/test";

import { cryptoDatabaseName } from "../identity";
import { CryptoOperations } from "../operations";
import { MemoryCryptoBackend } from "../store/backend";
import { CryptoStore } from "../store/cryptoStore";
import { PUSH_DECRYPT_REQUEST, type PushDecryptRequest } from "./protocol";
import {
  decryptFromStore,
  decryptPush,
  serializeDecryption,
  type PushDecryptEnvironment,
} from "./pushDecrypt";
import { ALICE, exchange as exchangeKeys, memoryKeys, sqliteFile } from "./pushTestKit";
import { openSnapshotBackend } from "./snapshotBackend";

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const BOB = "00000000-0000-4000-8000-00000000000b";
const BOB_DB = cryptoDatabaseName(`user-${BOB}`);
const exchange = (body: string) => exchangeKeys(body, BOB);

function loadSqlite(): Promise<unknown> {
  const wasm = readFileSync(
    new URL("../../../../node_modules/@journeyapps/wa-sqlite/dist/wa-sqlite.wasm", import.meta.url),
  );
  return SQLiteESMFactory({ wasmBinary: wasm });
}

describe("decryptFromStore", () => {
  it("暗号ストアの写しと Push の中身だけで本文を復号し、通知の長さに切る", async () => {
    const long = `秘密🦊${"あ".repeat(300)}`;
    const { row, bob } = await exchange(long);
    expect(JSON.stringify(row)).not.toContain("秘密");
    const body = await decryptFromStore({
      name: BOB_DB,
      recipientId: BOB,
      backend: bob.backend,
      message: row,
      keys: bob.keys,
    });
    expect(body).toBe(Array.from(long).slice(0, 240).join(""));
    // 写しには何も書かない（受信用セッションも、照合の記録も保存しない）
    expect(bob.backend.inboundSessions.size).toBe(1);
  });

  it("照合できない行、別の宛先、鍵のない端末では復号しない", async () => {
    const { row, bob } = await exchange("hello");
    const run = (overrides: Partial<EncryptedMessage>, recipientId = BOB, keys = bob.keys) =>
      decryptFromStore({
        name: BOB_DB,
        recipientId,
        backend: bob.backend,
        message: { ...row, ...overrides },
        keys,
      });
    expect(await run({})).toBe("hello");
    // 行の照合（受信時の検証）に失敗する
    expect(await run({ message_id: "m2" })).toBeNull();
    expect(await run({ user_id: BOB })).toBeNull();
    expect(await run({ message_kind: "emote" })).toBeNull();
    // 受信用セッションがない
    expect(
      await run({ ciphertext: { ...row.ciphertext, session_id: "missing" } as JsonObject }),
    ).toBeNull();
    // 暗号ストアが別のユーザーのもの
    expect(await run({}, ALICE)).toBeNull();
    // 保存鍵を包む鍵がない（作り直さない）
    expect(await run({}, BOB, memoryKeys())).toBeNull();
  });

  it("OPFS と同じ表の SQLite の写しを、メモリーの VFS で読んで復号する", async () => {
    const { row, bob } = await exchange("from sqlite");
    const module = await loadSqlite();
    const database = await sqliteFile(module, bob.backend);
    const snapshot = await openSnapshotBackend(module, { database, journal: null });
    try {
      expect(
        await decryptFromStore({
          name: BOB_DB,
          recipientId: BOB,
          backend: snapshot,
          message: row,
          keys: bob.keys,
        }),
      ).toBe("from sqlite");
    } finally {
      await snapshot.close();
    }
  });
});

describe("ログアウトで消し切れなかった暗号ストア", () => {
  function failingRemoval(backend: MemoryCryptoBackend) {
    // ファイルの削除だけが失敗する（OPFS のファイルを別の接続が開いていた、など）。
    backend.destroy = async () => {
      throw new Error("NoModificationAllowedError");
    };
  }

  it("CryptoStore.destroy でファイルが残っても、包む鍵を先に消すので復号しない", async () => {
    const { row, bob } = await exchange("logged out secret");
    const { store } = await CryptoStore.open({
      backend: bob.backend,
      name: BOB_DB,
      userId: BOB,
      keys: bob.keys,
      reset: async () => bob.backend,
    });
    failingRemoval(bob.backend);
    await expect(store.destroy()).rejects.toThrow("NoModificationAllowedError");
    // DB は残っている
    expect(bob.backend.inboundSessions.size).toBe(1);
    expect(await bob.keys.get(BOB_DB)).toBeUndefined();
    expect(
      await decryptFromStore({
        name: BOB_DB,
        recipientId: BOB,
        backend: bob.backend,
        message: row,
        keys: bob.keys,
      }),
    ).toBeNull();
  });

  it("開いていない暗号ストアを消す操作（destroy）でも、包む鍵を先に消す", async () => {
    const { row, bob } = await exchange("logged out secret");
    failingRemoval(bob.backend);
    const operations = new CryptoOperations(
      { open: async () => bob.backend, remove: () => bob.backend.destroy() },
      bob.keys,
    );
    await expect(operations.run({ op: "destroy", namespace: `user-${BOB}` })).rejects.toThrow(
      "NoModificationAllowedError",
    );
    expect(await bob.keys.get(BOB_DB)).toBeUndefined();
    expect(
      await decryptFromStore({
        name: BOB_DB,
        recipientId: BOB,
        backend: bob.backend,
        message: row,
        keys: bob.keys,
      }),
    ).toBeNull();
  });
});

describe("decryptPush", () => {
  async function setup() {
    const { row, bob } = await exchange("push body");
    const e2ee = { recipient_id: BOB, message: row };
    const asked: PushDecryptRequest[] = [];
    const locks: string[] = [];
    const env = (held: Record<string, boolean>, files: string[]): PushDecryptEnvironment => ({
      withLockIfAvailable: async (name, callback) => {
        locks.push(name);
        return callback(held[name] ?? true);
      },
      openSnapshot: async (name) =>
        files.includes(name)
          ? {
              read: () => bob.backend.read(),
              readInboundSession: (key) => bob.backend.readInboundSession(key),
              close: async () => {},
            }
          : null,
      askTabs: async (request) => {
        asked.push(request);
        return "from tab";
      },
      keys: bob.keys,
    });
    return { e2ee, env, asked, locks };
  }

  it("暗号ストアを開いているタブがなければ、ロックを取って写しから復号する", async () => {
    const { e2ee, env, asked, locks } = await setup();
    expect(await decryptPush(e2ee, env({}, [BOB_DB]))).toBe("push body");
    expect(locks).toEqual([`zoo-crypto:${BOB_DB}`]);
    expect(asked).toEqual([]);
  });

  it("開いているタブがあれば、そのタブに頼む", async () => {
    const { e2ee, env, asked } = await setup();
    const result = await decryptPush(e2ee, env({ [`zoo-crypto:${BOB_DB}`]: false }, [BOB_DB]));
    expect(result).toBe("from tab");
    expect(asked).toEqual([
      { type: PUSH_DECRYPT_REQUEST, namespace: `user-${BOB}`, message: e2ee.message },
    ]);
  });

  it("OIDC の暗号ストアがなければ、開発用のログインの暗号ストアを探す", async () => {
    const { e2ee, env, locks } = await setup();
    const dev = cryptoDatabaseName(`dev-${BOB}`);
    expect(await decryptPush(e2ee, env({}, []))).toBeNull();
    expect(locks).toEqual([`zoo-crypto:${BOB_DB}`, `zoo-crypto:${dev}`]);
  });

  it("形の違う e2ee は、ロックも取らずに復号しない", async () => {
    const { e2ee, env, locks } = await setup();
    const open = vi.fn();
    for (const value of [
      null,
      { ...e2ee, recipient_id: "bob" },
      { ...e2ee, message: { ...e2ee.message, epoch: -1 } },
      { ...e2ee, message: { ...e2ee.message, ciphertext: { session_id: "s" } } },
    ]) {
      expect(await decryptPush(value, { ...env({}, [BOB_DB]), openSnapshot: open })).toBeNull();
    }
    expect(locks).toEqual([]);
    expect(open).not.toHaveBeenCalled();
  });
});

describe("serializeDecryption", () => {
  it("同時に届いた Push を 1 件ずつ復号する（前の処理がロックを持つあいだに次を始めない）", async () => {
    let running = 0;
    let overlapped = false;
    const order: string[] = [];
    const decrypt = serializeDecryption(async (e2ee) => {
      running += 1;
      if (running > 1) overlapped = true;
      await new Promise((resolve) => setTimeout(resolve, 10));
      order.push(String(e2ee));
      running -= 1;
      return `body:${String(e2ee)}`;
    }, 1_000);
    expect(await Promise.all([decrypt("a"), decrypt("b"), decrypt("c")])).toEqual([
      "body:a",
      "body:b",
      "body:c",
    ]);
    expect(order).toEqual(["a", "b", "c"]);
    expect(overlapped).toBe(false);
  });

  it("上限は前の処理を待つ時間を含み、止まった処理のあとも次を始める", async () => {
    vi.useFakeTimers();
    try {
      const wait = (ms: number, value: string) =>
        new Promise<string>((resolve) => setTimeout(() => resolve(value), ms));
      const decrypt = serializeDecryption(async (e2ee) => {
        if (e2ee === "slow") return wait(80, "slow");
        if (e2ee === "work") return wait(50, "work");
        return new Promise<string | null>(() => {});
      }, 100);
      // 2 件目は 80 ms 待ってから 50 ms 掛かるので、100 ms の上限を過ぎる
      const slow = decrypt("slow");
      const late = decrypt("work");
      await vi.advanceTimersByTimeAsync(100);
      expect(await slow).toBe("slow");
      expect(await late).toBeNull();
      // 止まった処理は上限（t = 200）で諦め、次の処理はそこから始める
      const stuck = decrypt("stuck");
      await vi.advanceTimersByTimeAsync(80);
      const next = decrypt("work");
      await vi.advanceTimersByTimeAsync(100);
      expect(await stuck).toBeNull();
      expect(await next).toBe("work");
      // 失敗は null にする
      const failing = serializeDecryption(async () => {
        throw new Error("x");
      }, 100);
      expect(await failing("e")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});
