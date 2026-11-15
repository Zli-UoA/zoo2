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
import { MemoryCryptoBackend, type CryptoStoreWrite, type DeviceReplacement } from "./backend";
import { CryptoStore, type KeysQueryResponse } from "./cryptoStore";
import type { DeviceRecord, DeviceRecordStore, ReplacedDevice } from "./deviceRecord";
import type { WrappingKeyStore } from "./storageKey";

/** 端末鍵だけの `keys/query` の応答（クロス署名鍵がないので、送信端末は検証済みにならない）。 */
function keysQueryOf(deviceKeys: Record<string, Record<string, JsonObject>>): KeysQueryResponse {
  return {
    device_keys: deviceKeys,
    master_keys: {},
    self_signing_keys: {},
    user_signing_keys: {},
    tofu_pins: {},
    signatures: [],
  };
}

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

function memoryDevices(): DeviceRecordStore & {
  records: Map<string, DeviceRecord>;
  failPut: boolean;
  failGet: boolean;
} {
  const records = new Map<string, DeviceRecord>();
  const store = {
    records,
    failPut: false,
    failGet: false,
    get: async (name: string) => {
      if (store.failGet) throw new Error("idb down");
      return records.get(name);
    },
    put: async (name: string, record: DeviceRecord) => {
      if (store.failPut) throw new Error("idb down");
      records.set(name, structuredClone(record));
    },
    delete: async (name: string) => {
      records.delete(name);
    },
  };
  return store;
}

async function open(
  backend: MemoryCryptoBackend,
  keys: WrappingKeyStore,
  userId = BOB,
  devices?: DeviceRecordStore,
) {
  return CryptoStore.open({
    backend,
    name: NAME,
    userId,
    keys,
    devices,
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
    const { store, replacedDevices } = await open(backend, keys);
    expect(replacedDevices).toEqual([]);
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
    // 登録していなかった前の端末は、削除 API の 404 でもすぐには消えたものとしない。
    expect(second.replacedDevices).toEqual([{ deviceId, registered: false }]);
    expect(second.store.deviceId).not.toBe(deviceId);
    await second.store.close();
  });

  it("作り直した前の端末が登録済みだったかを知らせる", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const first = await open(backend, keys);
    const deviceId = first.store.deviceId;
    await first.store.markRegistered();
    await first.store.close();
    keys.keys.clear();
    const second = await open(backend, keys);
    expect(second.replacedDevices).toEqual([{ deviceId, registered: true }]);
    await second.store.close();
  });

  describe("暗号ストアの外の端末の記録", () => {
    const record = (deviceId: string, registered: boolean, replaced: ReplacedDevice[] = []) => ({
      userId: BOB,
      deviceId,
      registered,
      replaced,
    });

    it("暗号ストアのファイルだけが消えたら、記録にある前の端末を知らせる", async () => {
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      const deviceId = first.store.deviceId;
      expect(devices.records.get(NAME)).toEqual(record(deviceId, false));
      await first.store.markRegistered();
      expect(devices.records.get(NAME)).toEqual(record(deviceId, true));
      await first.store.close();

      // OPFS のファイルだけが消えた（包む鍵と記録は IndexedDB に残る）。
      const second = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      expect(second.store.deviceId).not.toBe(deviceId);
      expect(second.replacedDevices).toEqual([{ deviceId, registered: true }]);
      // 記録は新しい端末に進むが、前の端末は削除待ちに渡すまで `replaced` に残す。
      expect(devices.records.get(NAME)).toEqual(
        record(second.store.deviceId, false, [{ deviceId, registered: true }]),
      );
      await second.store.close();
    });

    it("前の端末を削除待ちに渡す前に閉じても、次に開いたときにもう一度知らせる", async () => {
      const backend = new MemoryCryptoBackend();
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      const previous = first.store.deviceId;
      await first.store.markRegistered();
      await first.store.close();

      const second = await open(backend, keys, BOB, devices);
      const current = second.store.deviceId;
      expect(second.replacedDevices).toEqual([{ deviceId: previous, registered: true }]);
      // 削除待ちに渡す前にタブを閉じた。
      await second.store.close();

      const third = await open(backend, keys, BOB, devices);
      expect(third.store.deviceId).toBe(current);
      expect(third.replacedDevices).toEqual([{ deviceId: previous, registered: true }]);
      // 新しい端末の登録でも、渡していない前の端末を落とさない。
      await third.store.markRegistered();
      expect(devices.records.get(NAME)).toEqual(
        record(current, true, [{ deviceId: previous, registered: true }]),
      );
      // 渡せたら外し、次からは知らせない。
      await third.store.forgetReplacedDevices([previous]);
      expect(devices.records.get(NAME)).toEqual(record(current, true));
      await third.store.close();

      const fourth = await open(backend, keys, BOB, devices);
      expect(fourth.replacedDevices).toEqual([]);
      await fourth.store.close();
    });

    it("前の端末は、渡すまで重ねて持つ", async () => {
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      const a = first.store.deviceId;
      await first.store.close();
      const second = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      const b = second.store.deviceId;
      await second.store.close();
      // A を渡さないうちに、B のファイルもなくなった。
      const third = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      expect(third.replacedDevices).toEqual([
        { deviceId: a, registered: false },
        { deviceId: b, registered: false },
      ]);
      await third.store.close();
    });

    it("別のユーザーの記録は、前の端末として知らせず、上書きする", async () => {
      const devices = memoryDevices();
      const other = "6f1e2d3c-4b5a-4987-8a6b-5c4d3e2f1a0b";
      devices.records.set(NAME, {
        userId: ALICE,
        deviceId: other,
        registered: true,
        replaced: [{ deviceId: "1f2e3d4c-5b6a-4789-8a6b-5c4d3e2f1a0b", registered: true }],
      });
      const opened = await open(new MemoryCryptoBackend(), memoryKeys(), BOB, devices);
      expect(opened.replacedDevices).toEqual([]);
      expect(devices.records.get(NAME)).toEqual(record(opened.store.deviceId, false));
      await opened.store.close();
    });

    it("開き直しでは知らせない。記録のない暗号ストア（前の版）は、今の端末を記録する", async () => {
      const backend = new MemoryCryptoBackend();
      const keys = memoryKeys();
      // 前の版で作った（記録のない）暗号ストア。
      const first = await open(backend, keys);
      const deviceId = first.store.deviceId;
      await first.store.markRegistered();
      await first.store.close();

      const devices = memoryDevices();
      const reopened = await open(backend, keys, BOB, devices);
      expect(reopened.store.deviceId).toBe(deviceId);
      expect(reopened.replacedDevices).toEqual([]);
      expect(devices.records.get(NAME)).toEqual(record(deviceId, true));
      await reopened.store.close();

      const again = await open(backend, keys, BOB, devices);
      expect(again.replacedDevices).toEqual([]);
      await again.store.close();
    });

    it("新しい端末の記録を書けなかったら、次に開いたときに前の端末を知らせる", async () => {
      const backend = new MemoryCryptoBackend();
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      const previous = first.store.deviceId;
      await first.store.markRegistered();
      await first.store.close();

      devices.failPut = true;
      const second = await open(backend, keys, BOB, devices);
      // 記録を書けなくても、暗号ストアは開く。
      expect(second.replacedDevices).toEqual([{ deviceId: previous, registered: true }]);
      const current = second.store.deviceId;
      await second.store.close();
      expect(devices.records.get(NAME)?.deviceId).toBe(previous);

      devices.failPut = false;
      const third = await open(backend, keys, BOB, devices);
      expect(third.store.deviceId).toBe(current);
      expect(third.replacedDevices).toEqual([{ deviceId: previous, registered: true }]);
      expect(devices.records.get(NAME)).toEqual(
        record(current, false, [{ deviceId: previous, registered: true }]),
      );
      await third.store.close();
    });

    it("記録を読めなかったら書き換えず、読めるようになってから前の端末を知らせる", async () => {
      const backend = new MemoryCryptoBackend();
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(new MemoryCryptoBackend(), keys, BOB, devices);
      const previous = first.store.deviceId;
      await first.store.markRegistered();
      await first.store.close();

      // ファイルが消えたうえ、IndexedDB の読み取りが一時的に失敗した。
      devices.failGet = true;
      const second = await open(backend, keys, BOB, devices);
      const current = second.store.deviceId;
      expect(second.replacedDevices).toEqual([]);
      await second.store.markRegistered();
      await second.store.close();
      // 前の端末の記録を上書きしない。
      expect(devices.records.get(NAME)).toEqual(record(previous, true));

      devices.failGet = false;
      const third = await open(backend, keys, BOB, devices);
      expect(third.store.deviceId).toBe(current);
      expect(third.replacedDevices).toEqual([{ deviceId: previous, registered: true }]);
      await third.store.close();
    });

    it("保存鍵を開けず、記録に別の端末もあれば、両方を前の端末として知らせる", async () => {
      const backend = new MemoryCryptoBackend();
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(backend, keys, BOB, devices);
      const lost = first.store.deviceId;
      await first.store.close();
      // 記録には、さらに前の（渡していない）端末が残っている。
      const older = "2a3b4c5d-6e7f-4a8b-9c0d-1e2f3a4b5c6d";
      devices.records.set(NAME, record(older, true));
      keys.keys.clear();
      const second = await open(backend, keys, BOB, devices);
      expect(second.replacedDevices).toEqual([
        { deviceId: lost, registered: false },
        { deviceId: older, registered: true },
      ]);
      await second.store.close();
    });

    it("別の端末から削除されて作り直した端末は、記録を進めて、前の端末を知らせない", async () => {
      const backend = new MemoryCryptoBackend();
      const keys = memoryKeys();
      const devices = memoryDevices();
      const first = await open(backend, keys, BOB, devices);
      await first.store.markRegistered();
      const { deviceId } = await first.store.replaceDevice();
      await first.store.close();
      expect(devices.records.get(NAME)).toEqual(record(deviceId, false));

      const reopened = await open(backend, keys, BOB, devices);
      expect(reopened.store.deviceId).toBe(deviceId);
      expect(reopened.replacedDevices).toEqual([]);
      await reopened.store.close();
    });

    it("暗号ストアを消すと、記録も消す", async () => {
      const devices = memoryDevices();
      const opened = await open(new MemoryCryptoBackend(), memoryKeys(), BOB, devices);
      await opened.store.destroy();
      expect(devices.records.has(NAME)).toBe(false);
    });
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

  it("端末を作り直したあとの開き直しに失敗しても、試し直しで作り直した端末から続ける", async () => {
    class FlakyBackend extends MemoryCryptoBackend {
      down = false;
      override async read() {
        if (this.down) throw new Error("storage down");
        return super.read();
      }
      override async replaceDevice(replacement: DeviceReplacement): Promise<void> {
        await super.replaceDevice(replacement);
        // 作り直しは保存できたが、そのあとの開き直しで読めない。
        this.down = true;
      }
    }
    const backend = new FlakyBackend();
    const operations = new CryptoOperations(
      { open: async () => backend, remove: async () => backend.destroy() },
      memoryKeys(),
    );
    const namespace = `user-${BOB}`;
    await operations.run({ op: "open", namespace, userId: BOB });
    await operations.run({ op: "markRegistered" });
    const first = { deviceId: backend.meta.get("device_id")! };
    const request = {
      op: "replaceDevice" as const,
      namespace,
      userId: BOB,
      deviceId: first.deviceId,
    };
    await expect(operations.run(request)).rejects.toThrow("storage down");
    const replacedDeviceId = backend.meta.get("device_id");
    expect(replacedDeviceId).not.toBe(first.deviceId);
    const account = backend.account;

    // ストレージが戻ったら、同じ要求（消された前の端末）でも、保存済みの新しい端末を開いて返す。
    // 新しい端末をもう一度作り直さない。
    backend.down = false;
    const retried = await operations.run(request);
    expect(retried).toMatchObject({ deviceId: replacedDeviceId, registered: false });
    expect(backend.account).toBe(account);
    // 開き直したストアで、そのまま登録に進める。
    const registration = (await operations.run({ op: "registrationRequest" })) as {
      device_keys: JsonObject;
    };
    expect(registration.device_keys).toMatchObject({ device_id: replacedDeviceId });
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
    expect(second.replacedDevices).toEqual([{ deviceId, registered: false }]);
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
    const deviceKeys = keysQueryOf({ [ALICE]: { [alice.deviceId]: alice.deviceKeys() } });

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
    const deviceKeys = keysQueryOf({ [ALICE]: { [alice.deviceId]: alice.deviceKeys() } });

    if (failure === "write") backend.failNextWrite = true;
    else backend.failNextRead = true;
    await expect(
      operations.run({ op: "processToDevice", rows: [row], keysQuery: deviceKeys }),
    ).rejects.toThrow("I/O error");
    // 復号で進んだメモリーの状態は捨ててあるので、同じメッセージをもう一度復号できる。
    expect(
      await operations.run({ op: "processToDevice", rows: [row], keysQuery: deviceKeys }),
    ).toEqual({
      processed: ["7"],
      roomKeys: 1,
      dropped: 0,
      newOlmSessions: 1,
      verifications: [],
    });
    alice.free();
  });
});
