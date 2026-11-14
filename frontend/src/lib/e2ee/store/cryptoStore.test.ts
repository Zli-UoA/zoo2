import { readFileSync } from "node:fs";

import {
  DeviceAccount,
  OutboundSession,
  initSync,
  verifyDeviceKeys,
  type JsonObject,
} from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it, vi } from "vite-plus/test";

import { CryptoOperations } from "../operations";
import { MemoryCryptoBackend, type CryptoStoreWrite } from "./backend";
import { CryptoStore } from "./cryptoStore";
import type { WrappingKeyStore } from "./storageKey";

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const ALICE = "00000000-0000-4000-8000-00000000000a";
const BOB = "00000000-0000-4000-8000-00000000000b";
const CHANNEL = "00000000-0000-4000-8000-0000000000c1";
const NAME = "zoo-crypto-test.sqlite";

function memoryKeys(): WrappingKeyStore & { keys: Map<string, CryptoKey> } {
  const keys = new Map<string, CryptoKey>();
  return {
    keys,
    get: async (name) => keys.get(name),
    put: async (name, key) => {
      keys.set(name, key);
    },
    delete: async (name) => {
      keys.delete(name);
    },
  };
}

async function open(backend: MemoryCryptoBackend, keys: WrappingKeyStore, userId = BOB) {
  return CryptoStore.open({
    backend,
    name: NAME,
    userId,
    keys,
    reset: async () => {
      await backend.destroy();
      return backend;
    },
  });
}

/** Alice の端末から Bob の端末へ、Megolm セッション鍵を Olm で送る。 */
function sendRoomKey(alice: DeviceAccount, bobKeys: JsonObject, bobOneTimeKey: JsonObject) {
  const recipient = verifyDeviceKeys(bobKeys, BOB, bobKeys.device_id as string);
  const session = alice.createOutboundOlmSession(recipient, bobOneTimeKey);
  const outbound = new OutboundSession(CHANNEL, 1, Date.now());
  return alice.encryptRoomKey(session, recipient, outbound.roomKey());
}

describe("CryptoStore", () => {
  it("端末を作り、開き直すと同じ端末に戻る", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const { store, replacedDeviceId } = await open(backend, keys);
    expect(replacedDeviceId).toBeNull();
    expect(store.isRegistered).toBe(false);
    const request = store.registrationRequest();
    expect(request.device_keys).toMatchObject({ user_id: BOB, device_id: store.deviceId });
    expect(Object.keys(request.one_time_keys)).toHaveLength(50);
    expect(Object.keys(request.fallback_keys)).toHaveLength(1);
    // 保存したものに平文の鍵は入らない（包んだ保存鍵と暗号化した pickle だけ）。
    const identityKeys = Object.values(request.device_keys.keys as Record<string, string>);
    for (const key of identityKeys) expect(backend.account).not.toContain(key);
    const deviceId = store.deviceId;
    await store.markRegistered();
    await store.close();

    const reopened = await open(backend, keys);
    expect(reopened.store.deviceId).toBe(deviceId);
    expect(reopened.store.isRegistered).toBe(true);
    // 公開済みの鍵は送り直さない。サーバーの数が足りなければ作る。
    expect(
      await reopened.store.prepareKeyUpload({ oneTimeKeyCount: 50, hasUnusedFallbackKey: true }),
    ).toBeNull();
    const upload = await reopened.store.prepareKeyUpload({
      oneTimeKeyCount: 45,
      hasUnusedFallbackKey: false,
    });
    expect(Object.keys(upload!.one_time_keys)).toHaveLength(5);
    expect(Object.keys(upload!.fallback_keys)).toHaveLength(1);
    await reopened.store.close();
  });

  it("保存鍵を開けなければ作り直し、前の端末を知らせる", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const first = await open(backend, keys);
    const deviceId = first.store.deviceId;
    await first.store.close();
    keys.keys.clear();
    const second = await open(backend, keys);
    expect(second.replacedDeviceId).toBe(deviceId);
    expect(second.store.deviceId).not.toBe(deviceId);
    await second.store.close();
  });

  it("開き直しにも失敗したら、ストレージが戻ったあとの操作で開き直す", async () => {
    class DownBackend extends MemoryCryptoBackend {
      down = false;
      override async read() {
        if (this.down) throw new Error("storage down");
        return super.read();
      }
      override async write(changes: CryptoStoreWrite): Promise<void> {
        if (this.down) throw new Error("storage down");
        return super.write(changes);
      }
    }
    const backend = new DownBackend();
    const operations = new CryptoOperations(
      { open: async () => backend, remove: async () => backend.destroy() },
      memoryKeys(),
    );
    await operations.run({ op: "open", namespace: `user-${BOB}`, userId: BOB });
    backend.down = true;
    // 書き込みに失敗し、そのあとの開き直しも失敗する。
    await expect(operations.run({ op: "markRegistered" })).rejects.toThrow("storage down");
    await expect(operations.run({ op: "pendingBackupCount" })).rejects.toThrow("storage down");
    backend.down = false;
    expect(await operations.run({ op: "pendingBackupCount" })).toBe(0);
    await operations.run({ op: "markRegistered" });
  });

  it("開き直しに失敗したあとで消したストアは、次の操作で開き直さない", async () => {
    class DownBackend extends MemoryCryptoBackend {
      down = false;
      override async read() {
        if (this.down) throw new Error("storage down");
        return super.read();
      }
      override async write(changes: CryptoStoreWrite): Promise<void> {
        if (this.down) throw new Error("storage down");
        return super.write(changes);
      }
    }
    const backend = new DownBackend();
    const operations = new CryptoOperations(
      { open: async () => backend, remove: async () => backend.destroy() },
      memoryKeys(),
    );
    const namespace = `user-${BOB}`;
    await operations.run({ op: "open", namespace, userId: BOB });
    backend.down = true;
    await expect(operations.run({ op: "markRegistered" })).rejects.toThrow("storage down");
    backend.down = false;
    await operations.run({ op: "destroy", namespace });
    await expect(operations.run({ op: "pendingBackupCount" })).rejects.toThrow(
      "暗号ストアが開いていません",
    );
    expect(backend.account).toBeNull();
  });

  it("包んだ保存鍵の base64 が壊れていても作り直す", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const first = await open(backend, keys);
    const deviceId = first.store.deviceId;
    await first.store.close();
    backend.meta.set("wrapped_storage_key", "!");
    const second = await open(backend, keys);
    expect(second.replacedDeviceId).toBe(deviceId);
    await second.store.close();
  });

  it.each([
    ["新しい端末の最初の保存", "write"],
    ["保存した Olm セッションの復元", "restore"],
  ] as const)("%sに失敗したら、作った鍵のオブジェクトを解放して閉じる", async (_, failure) => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    if (failure === "restore") {
      await (await open(backend, keys)).store.close();
      backend.olmSessions.set("broken", {
        sessionId: "broken",
        theirIdentityKey: "x",
        pickle: "not a pickle",
      });
    } else {
      // 包んだ保存鍵の保存は通し、新しい端末の最初の保存だけ失敗させる。
      const write = backend.write.bind(backend);
      backend.write = async (changes) => {
        if (changes.account !== undefined) throw new Error("disk full");
        return write(changes);
      };
    }
    const free = vi.spyOn(CryptoStore.prototype as unknown as { free(): void }, "free");
    const close = vi.spyOn(backend, "close");
    try {
      await expect(open(backend, keys)).rejects.toThrow();
      expect(free).toHaveBeenCalledTimes(1);
      expect(close).toHaveBeenCalled();
    } finally {
      free.mockRestore();
    }
  });

  it("別のユーザーの暗号ストアは開かない", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    await (await open(backend, keys)).store.close();
    await expect(open(backend, keys, ALICE)).rejects.toThrow();
  });

  it("to-device の room key を保存し、同じメッセージを 2 回処理しない", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const { store } = await open(backend, keys);
    const request = store.registrationRequest();
    await store.markRegistered();
    const alice = new DeviceAccount(ALICE, crypto.randomUUID());
    const oneTimeKey = Object.values(request.one_time_keys)[0]!;
    const content = sendRoomKey(alice, request.device_keys, oneTimeKey);
    const row = {
      id: "7",
      sender_user_id: ALICE,
      sender_device_id: alice.deviceId,
      type: "m.room.encrypted",
      content,
    };
    const deviceKeys = { [ALICE]: { [alice.deviceId]: alice.deviceKeys() } };

    const result = await store.processToDevice([row], deviceKeys);
    expect(result).toEqual({
      processed: ["7"],
      roomKeys: 1,
      dropped: 0,
      newOlmSessions: 1,
      verifications: [],
    });
    expect(backend.olmSessions.size).toBe(1);
    expect(backend.inboundSessions.size).toBe(1);
    expect(await store.pendingBackupCount()).toBe(1);

    // ack を送る前に終了して開き直しても、処理済みとして扱う。
    await store.close();
    const reopened = (await open(backend, keys)).store;
    const again = await reopened.processToDevice([row], deviceKeys);
    expect(again).toEqual({
      processed: ["7"],
      roomKeys: 0,
      dropped: 0,
      newOlmSessions: 0,
      verifications: [],
    });
    await reopened.acknowledged(["7"]);
    expect(backend.processedToDevice.size).toBe(0);

    // 壊れたメッセージと、知らない type は捨てて ack する。
    const broken = await reopened.processToDevice(
      [
        { ...row, id: "8", content: { algorithm: "x" } },
        { ...row, id: "9", type: "m.other" },
      ],
      deviceKeys,
    );
    expect(broken).toMatchObject({ processed: ["8", "9"], dropped: 2 });

    await reopened.destroy();
    expect(backend.destroyed).toBe(true);
    expect(keys.keys.size).toBe(0);
    alice.free();
  });

  it.each([
    ["書き込み", "write"],
    ["保存済みのセッションの読み取り", "read"],
  ] as const)("%sに失敗したら、保存済みの状態から開き直してから処理し直す", async (_, failure) => {
    class FlakyBackend extends MemoryCryptoBackend {
      failNextWrite = false;
      failNextRead = false;
      override async write(changes: CryptoStoreWrite): Promise<void> {
        if (this.failNextWrite) {
          this.failNextWrite = false;
          throw new Error("I/O error");
        }
        return super.write(changes);
      }
      override async readInboundSession(key: string): Promise<string | null> {
        if (this.failNextRead) {
          this.failNextRead = false;
          throw new Error("I/O error");
        }
        return super.readInboundSession(key);
      }
    }
    const backend = new FlakyBackend();
    const operations = new CryptoOperations(
      { open: async () => backend, remove: async () => backend.destroy() },
      memoryKeys(),
    );
    const namespace = `user-${BOB}`;
    await operations.run({ op: "open", namespace, userId: BOB });
    const request = (await operations.run({ op: "registrationRequest" })) as Awaited<
      ReturnType<CryptoStore["registrationRequest"]>
    >;
    await operations.run({ op: "markRegistered" });
    const alice = new DeviceAccount(ALICE, crypto.randomUUID());
    const content = sendRoomKey(
      alice,
      request.device_keys,
      Object.values(request.one_time_keys)[0]!,
    );
    const row = {
      id: "7",
      sender_user_id: ALICE,
      sender_device_id: alice.deviceId,
      type: "m.room.encrypted",
      content,
    };
    const deviceKeys = { [ALICE]: { [alice.deviceId]: alice.deviceKeys() } };

    if (failure === "write") backend.failNextWrite = true;
    else backend.failNextRead = true;
    await expect(
      operations.run({ op: "processToDevice", rows: [row], deviceKeys }),
    ).rejects.toThrow("I/O error");
    // 復号で進んだメモリーの状態は捨ててあるので、同じメッセージをもう一度復号できる。
    expect(await operations.run({ op: "processToDevice", rows: [row], deviceKeys })).toEqual({
      processed: ["7"],
      roomKeys: 1,
      dropped: 0,
      newOlmSessions: 1,
      verifications: [],
    });
    alice.free();
  });
});
