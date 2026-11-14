import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError, type ApiClient } from "../api/client";
import { resolveConfig } from "../config";
import type { CryptoWorker } from "./cryptoWorker";
import {
  E2eeDeviceManager,
  loginNotFoundCounter,
  MAX_NOT_FOUND,
  retryPendingDeviceDeletion,
} from "./deviceManager";
import {
  CRYPTO_PENDING_DEVICE_DELETE_KEY,
  CRYPTO_PENDING_DEVICE_DELETE_UNREADABLE_KEY,
  markDeviceDeletionPending,
  readPendingDeviceDeletions,
  updatePendingDeviceDeletion,
} from "./identity";
import type { CryptoOperation, ToDeviceRow } from "./protocol";

const USER = "00000000-0000-4000-8000-00000000000a";
const DEVICE = "00000000-0000-4000-8000-0000000000d1";

function fakeWorker(overrides: Partial<Record<CryptoOperation["op"], unknown>> = {}) {
  const calls: CryptoOperation[] = [];
  const results: Partial<Record<CryptoOperation["op"], unknown>> = {
    open: { userId: USER, deviceId: DEVICE, registered: false, replacedDeviceId: null },
    registrationRequest: {
      device_keys: { device_id: DEVICE },
      one_time_keys: {},
      fallback_keys: {},
    },
    prepareKeyUpload: null,
    processToDevice: {
      processed: ["1"],
      roomKeys: 1,
      dropped: 0,
      newOlmSessions: 1,
      verifications: [],
    },
    pendingBackupCount: 2,
    ...overrides,
  };
  const worker = {
    request: vi.fn(async (operation: CryptoOperation) => {
      calls.push(operation);
      const result = results[operation.op];
      return (typeof result === "function" ? result(operation) : result) ?? null;
    }),
    terminate: vi.fn(),
  };
  return { worker: worker as unknown as CryptoWorker, calls };
}

function fakeClient(
  handlers: Record<string, (body: unknown, signal?: AbortSignal) => unknown> = {},
) {
  const requests: { method: string; path: string; body: unknown }[] = [];
  const counts = {
    one_time_key_counts: { signed_curve25519: 50 },
    unused_fallback_key_types: ["signed_curve25519"],
  };
  const respond = (method: string, path: string, body: unknown, signal?: AbortSignal) => {
    requests.push({ method, path, body });
    const handler = handlers[`${method} ${path}`];
    if (handler) return handler(body, signal);
    if (path === "/api/e2ee/keys/query") return { device_keys: {} };
    return counts;
  };
  const client = {
    request: vi.fn(
      async (
        path: string,
        options: { method?: string; body?: unknown; signal?: AbortSignal } = {},
      ) => respond(options.method ?? "GET", path, options.body, options.signal),
    ),
    post: vi.fn(async (path: string, body: unknown, options: { signal?: AbortSignal } = {}) => ({
      data: await respond("POST", path, body, options.signal),
      txid: null,
    })),
    delete: vi.fn(async (path: string) => ({
      data: await respond("DELETE", path, null),
      txid: null,
    })),
  };
  return { client: client as unknown as ApiClient, requests };
}

let rowsListener: ((rows: ToDeviceRow[]) => void) | null = null;
const subscribe = vi.fn((_config, _userId, _deviceId, onRows: (rows: ToDeviceRow[]) => void) => {
  rowsListener = onRows;
  return { unsubscribe: vi.fn() };
});

function manager(worker: CryptoWorker, client: ApiClient, settleTimeoutMs?: number) {
  return new E2eeDeviceManager({
    settleTimeoutMs,
    config: resolveConfig({ DEV: false }),
    client,
    worker,
    userId: USER,
    namespace: `user-${USER}`,
    displayName: "Chrome（macOS）",
    subscribe,
    retryDelayMs: 10,
  });
}

beforeEach(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("window", {
    location: { href: "http://localhost/" },
    localStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    },
  });
  rowsListener = null;
});

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("E2eeDeviceManager", () => {
  it("未登録の端末を登録してから、to-device を購読する", async () => {
    const { worker, calls } = fakeWorker();
    const { client, requests } = fakeClient();
    await manager(worker, client).start();
    expect(calls.map((call) => call.op)).toEqual([
      "open",
      "registrationRequest",
      "markRegistered",
      "prepareKeyUpload",
    ]);
    expect(requests[0]).toMatchObject({
      method: "POST",
      path: "/api/e2ee/devices",
      body: { display_name: "Chrome（macOS）", device_keys: { device_id: DEVICE } },
    });
    expect(subscribe).toHaveBeenCalledWith(expect.anything(), USER, DEVICE, expect.any(Function));
  });

  it("登録済みなら、空のアップロードで数を読み、足りない鍵を送る", async () => {
    const upload = { one_time_keys: { "signed_curve25519:AAAA": {} }, fallback_keys: {} };
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
      prepareKeyUpload: upload,
    });
    const { client, requests } = fakeClient();
    await manager(worker, client).start();
    expect(requests.map((request) => request.body)).toEqual([
      { device_id: DEVICE, one_time_keys: {}, fallback_keys: {} },
      { device_id: DEVICE, ...upload },
    ]);
    expect(calls.at(-1)).toEqual({ op: "markKeysPublished" });
  });

  it("別の端末から消された端末は、作り直して登録し直す", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    let first = true;
    const { client } = fakeClient({
      "POST /api/e2ee/keys/upload": () => {
        if (first) {
          first = false;
          throw new ApiError(404, "gone");
        }
        return { one_time_key_counts: { signed_curve25519: 50 }, unused_fallback_key_types: [] };
      },
    });
    await manager(worker, client).start();
    expect(calls.map((call) => call.op).slice(0, 3)).toEqual(["open", "destroy", "open"]);
  });

  it("to-device を処理して ack し、Olm セッションが増えたら鍵を補充する", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    const { client, requests } = fakeClient();
    const instance = manager(worker, client);
    await instance.start();
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    await vi.waitFor(() => expect(calls.some((call) => call.op === "acknowledged")).toBe(true));
    expect(requests).toContainEqual({
      method: "POST",
      path: "/api/e2ee/keys/query",
      body: { user_ids: ["u2"] },
    });
    expect(requests).toContainEqual({
      method: "POST",
      path: "/api/e2ee/to_device/ack",
      body: { device_id: DEVICE, ids: ["1"] },
    });
    await vi.waitFor(() =>
      expect(calls.filter((call) => call.op === "prepareKeyUpload")).toHaveLength(2),
    );
    await instance.stop();
  });

  it("ログアウトでは暗号ストアを消してから端末を消し、失敗したら次のログインで消し直す", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    const { client } = fakeClient({
      [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
        throw new ApiError(503, "offline");
      },
    });
    const instance = manager(worker, client);
    await instance.start();
    expect(await instance.pendingBackupCount()).toBe(2);
    await instance.destroy();
    expect(calls.at(-1)).toEqual({ op: "destroy", namespace: `user-${USER}` });
    expect(readPendingDeviceDeletions()).toEqual([{ userId: USER, deviceId: DEVICE }]);

    const retry = fakeClient();
    await retryPendingDeviceDeletion(retry.client, USER);
    expect(retry.requests).toEqual([
      { method: "DELETE", path: `/api/e2ee/devices/${DEVICE}`, body: null },
    ]);
    expect(readPendingDeviceDeletions()).toEqual([]);
  });

  it("登録の応答をなくした端末も、ログアウトで削除 API を呼ぶ", async () => {
    // open は registered: false を返す（登録はサーバーで済んだが、応答をなくした）。
    const { worker } = fakeWorker();
    const { client, requests } = fakeClient();
    const instance = manager(worker, client);
    await instance.attach();
    await instance.destroy();
    expect(requests).toContainEqual({
      method: "DELETE",
      path: `/api/e2ee/devices/${DEVICE}`,
      body: null,
    });
    expect(readPendingDeviceDeletions()).toEqual([]);
  });

  it("登録中にログアウトしたら、登録の結果を待ってから端末を消す", async () => {
    const { worker } = fakeWorker();
    let finishRegistration: () => void = () => {};
    const order: string[] = [];
    const { client, requests } = fakeClient({
      "POST /api/e2ee/devices": () =>
        new Promise((resolve) => {
          finishRegistration = () => {
            order.push("registered");
            resolve({
              one_time_key_counts: { signed_curve25519: 50 },
              unused_fallback_key_types: [],
            });
          };
        }),
      [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
        order.push("deleted");
        return {};
      },
    });
    const instance = manager(worker, client);
    const started = instance.start().catch(() => undefined);
    await vi.waitFor(() =>
      expect(requests.some((request) => request.path === "/api/e2ee/devices")).toBe(true),
    );
    const destroyed = instance.destroy();
    finishRegistration();
    await Promise.all([started, destroyed]);
    expect(order).toEqual(["registered", "deleted"]);
  });

  it("鍵の補充に失敗したら、行を処理し終えたあとも補充を試し直す", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
      prepareKeyUpload: { one_time_keys: { "signed_curve25519:AAAA": {} }, fallback_keys: {} },
    });
    let uploads = 0;
    const { client } = fakeClient({
      "POST /api/e2ee/keys/upload": (body) => {
        uploads += 1;
        const keys = (body as { one_time_keys: Record<string, unknown> }).one_time_keys;
        // 起動時の補充（2 回目の送信）のあと、新しいセッションの補充の 1 回目だけ失敗させる。
        if (uploads === 3 && Object.keys(keys).length === 0) throw new ApiError(503, "busy");
        return { one_time_key_counts: { signed_curve25519: 49 }, unused_fallback_key_types: [] };
      },
    });
    const instance = manager(worker, client);
    await instance.start();
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    await vi.waitFor(() =>
      expect(calls.filter((call) => call.op === "markKeysPublished")).toHaveLength(2),
    );
    await instance.stop();
  });

  it("ログアウトの確認のあいだは to-device を止め、取りやめたら再開する", async () => {
    const { worker } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    const { client } = fakeClient();
    const instance = manager(worker, client);
    await instance.start();
    expect(subscribe).toHaveBeenCalledTimes(1);
    const first = subscribe.mock.results[0]!.value as { unsubscribe: ReturnType<typeof vi.fn> };
    await instance.suspend();
    expect(first.unsubscribe).toHaveBeenCalled();
    instance.resume();
    expect(subscribe).toHaveBeenCalledTimes(2);
    await instance.stop();
  });

  it("ack が失敗し続けても、鍵の補充は行う", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    const { client, requests } = fakeClient({
      "POST /api/e2ee/to_device/ack": () => {
        throw new ApiError(503, "busy");
      },
    });
    const instance = manager(worker, client);
    await instance.start();
    const uploadsBefore = requests.filter((r) => r.path === "/api/e2ee/keys/upload").length;
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    // 新しい Olm セッションを作ったので、ack が失敗しても補充の数を読み、補充を試す。
    await vi.waitFor(() =>
      expect(requests.filter((r) => r.path === "/api/e2ee/keys/upload").length).toBeGreaterThan(
        uploadsBefore,
      ),
    );
    expect(calls.filter((call) => call.op === "prepareKeyUpload").length).toBeGreaterThan(1);
    await instance.stop();
  });

  it("登録の結果が分からないうちは、ログアウトで端末を消さずに削除待ちに残す", async () => {
    const { worker } = fakeWorker();
    const { client, requests } = fakeClient({
      // 登録の応答が返らない（タイムアウトより長くかかる）。
      "POST /api/e2ee/devices": () => new Promise(() => {}),
    });
    const instance = manager(worker, client, 20);
    void instance.start().catch(() => undefined);
    await vi.waitFor(() =>
      expect(requests.some((request) => request.path === "/api/e2ee/devices")).toBe(true),
    );
    await instance.destroy();
    expect(requests.some((request) => request.method === "DELETE")).toBe(false);
    expect(readPendingDeviceDeletions()).toEqual([
      { userId: USER, deviceId: DEVICE, unconfirmed: true },
    ]);
  });

  it("ログアウトの確認で止めるときは、通信中の要求を中止する", async () => {
    const { worker } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    let aborted = false;
    const { client, requests } = fakeClient({
      // keys/query の応答が返らない。中止されたら終わる。
      "POST /api/e2ee/keys/query": (_body, signal) =>
        new Promise((_resolve, reject) => {
          signal?.addEventListener("abort", () => {
            aborted = true;
            reject(new DOMException("aborted", "AbortError"));
          });
        }),
    });
    const instance = manager(worker, client);
    await instance.start();
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    await vi.waitFor(() =>
      expect(requests.some((request) => request.path === "/api/e2ee/keys/query")).toBe(true),
    );
    await instance.suspend();
    expect(aborted).toBe(true);
    await instance.stop();
  });

  it("ログアウトのために開いて保存鍵を作り直していたら、前の端末も消す", async () => {
    const OLD = "00000000-0000-4000-8000-0000000000d0";
    const { worker } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: false, replacedDeviceId: OLD },
    });
    const { client, requests } = fakeClient();
    const instance = manager(worker, client);
    await instance.attach();
    expect(readPendingDeviceDeletions()).toContainEqual({
      userId: USER,
      deviceId: OLD,
      unconfirmed: true,
    });
    await instance.destroy();
    const deleted = requests.filter((r) => r.method === "DELETE").map((r) => r.path);
    expect(deleted).toEqual([`/api/e2ee/devices/${DEVICE}`, `/api/e2ee/devices/${OLD}`]);
    expect(readPendingDeviceDeletions()).toEqual([]);
  });

  it("受信中に端末が消されていたら（ack が 404）、作り直して登録し直す", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    const { client } = fakeClient({
      "POST /api/e2ee/to_device/ack": () => {
        throw new ApiError(404, "gone");
      },
    });
    const instance = manager(worker, client);
    await instance.start();
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    await vi.waitFor(() =>
      expect(calls.map((call) => call.op)).toEqual(expect.arrayContaining(["destroy"])),
    );
    await vi.waitFor(() => expect(calls.filter((call) => call.op === "open")).toHaveLength(2));
    await instance.stop();
  });

  it("再登録で新しい端末を登録したあとで補充に失敗しても、試し直しで暗号ストアを消し直さない", async () => {
    const NEW_DEVICE = "00000000-0000-4000-8000-0000000000d2";
    let destroyed = false;
    let registered = false;
    const upload = { one_time_keys: { "signed_curve25519:AAAA": {} }, fallback_keys: {} };
    const { worker, calls } = fakeWorker({
      // 作り直したあとは、新しい未登録の端末を返す。登録が済んだら登録済みとして返す。
      open: () =>
        destroyed
          ? { userId: USER, deviceId: NEW_DEVICE, registered, replacedDeviceId: null }
          : { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
      destroy: () => {
        destroyed = true;
        return null;
      },
      markRegistered: () => {
        registered = true;
        return null;
      },
      // 受信の処理の中で補充しないよう、新しい Olm セッションは作らない。
      processToDevice: {
        processed: ["1"],
        roomKeys: 0,
        dropped: 0,
        newOlmSessions: 0,
        verifications: [],
      },
      // 新しい端末は、鍵を補充しなければならない。
      prepareKeyUpload: () => (destroyed ? upload : null),
    });
    let failedUploads = 0;
    const { client, requests } = fakeClient({
      "POST /api/e2ee/to_device/ack": () => {
        throw new ApiError(404, "gone");
      },
      "POST /api/e2ee/keys/upload": (body) => {
        const keys = (body as { one_time_keys: Record<string, unknown> }).one_time_keys;
        // 新しい端末の登録のあとの補充（鍵のある送信）を、最初の 2 回だけ失敗させる。
        if (Object.keys(keys).length > 0 && failedUploads < 2) {
          failedUploads += 1;
          throw new ApiError(503, "busy");
        }
        return { one_time_key_counts: { signed_curve25519: 50 }, unused_fallback_key_types: [] };
      },
    });
    const instance = manager(worker, client);
    await instance.start();
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    // 補充が通るまで起動を試し直し、新しい端末の補充を公開済みにする。
    await vi.waitFor(() =>
      expect(calls.filter((call) => call.op === "markKeysPublished")).toHaveLength(1),
    );
    expect(failedUploads).toBe(2);
    // 暗号ストアを消したのは、端末が消されていたと分かったときの 1 回だけ。
    expect(calls.filter((call) => call.op === "destroy")).toHaveLength(1);
    // 新しい端末の登録も 1 回だけで、試し直しは同じ端末を使う。
    expect(requests.filter((r) => r.path === "/api/e2ee/devices")).toHaveLength(1);
    expect(instance.currentDeviceId).toBe(NEW_DEVICE);
    await instance.stop();
  });

  it("ログアウトの確認で止めていたあいだに再登録が失敗したら、取りやめたときに再開する", async () => {
    const { worker, calls } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
      processToDevice: {
        processed: ["1"],
        roomKeys: 0,
        dropped: 0,
        newOlmSessions: 0,
        verifications: [],
      },
    });
    let uploads = 0;
    let failRestart: (error: Error) => void = () => {};
    const { client } = fakeClient({
      "POST /api/e2ee/to_device/ack": () => {
        throw new ApiError(404, "gone");
      },
      "POST /api/e2ee/keys/upload": () => {
        uploads += 1;
        // 再登録の起動（2 回目）の要求は、ログアウトの確認で止めてから失敗させる。
        if (uploads === 2)
          return new Promise((_resolve, reject) => {
            failRestart = reject;
          });
        return { one_time_key_counts: { signed_curve25519: 50 }, unused_fallback_key_types: [] };
      },
    });
    const instance = manager(worker, client);
    await instance.start();
    rowsListener!([
      {
        id: "1",
        sender_user_id: "u2",
        sender_device_id: "d2",
        type: "m.room.encrypted",
        content: {},
      },
    ]);
    await vi.waitFor(() => expect(uploads).toBe(2));
    await instance.suspend();
    failRestart(new ApiError(503, "busy"));
    // 止めているあいだは試し直さない。
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(calls.filter((call) => call.op === "open")).toHaveLength(2);
    instance.resume();
    await vi.waitFor(() => expect(calls.filter((call) => call.op === "open")).toHaveLength(3));
    // 作り直しは最初の 1 回だけ。
    expect(calls.filter((call) => call.op === "destroy")).toHaveLength(1);
    await instance.stop();
  });

  it("別のユーザーの削除待ちを上書きしない", async () => {
    markDeviceDeletionPending({ userId: "other", deviceId: "d-other" });
    markDeviceDeletionPending({ userId: USER, deviceId: DEVICE });
    const { client } = fakeClient();
    await retryPendingDeviceDeletion(client, USER);
    expect(readPendingDeviceDeletions()).toEqual([{ userId: "other", deviceId: "d-other" }]);
  });

  it("別のユーザーの削除待ちは消し直さない", async () => {
    markDeviceDeletionPending({ userId: "other", deviceId: DEVICE });
    const { client, requests } = fakeClient();
    await retryPendingDeviceDeletion(client, USER);
    expect(requests).toEqual([]);
    expect(readPendingDeviceDeletions()).toHaveLength(1);
  });

  describe("削除 API の 404", () => {
    const notFound = () => {
      throw new ApiError(404, "not found");
    };

    it("登録済みと分かっている端末の 404 は、済んだものとする", async () => {
      const { worker } = fakeWorker({
        open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
      });
      const { client } = fakeClient({ [`DELETE /api/e2ee/devices/${DEVICE}`]: notFound });
      const instance = manager(worker, client);
      await instance.start();
      await instance.destroy();
      expect(readPendingDeviceDeletions()).toEqual([]);
    });

    it("このログインで登録できた端末の 404 は、済んだものとする", async () => {
      // open は registered: false。登録の応答を受け取ったので、登録できたと分かっている。
      const { worker } = fakeWorker();
      const { client } = fakeClient({ [`DELETE /api/e2ee/devices/${DEVICE}`]: notFound });
      const instance = manager(worker, client);
      await instance.start();
      await instance.destroy();
      expect(readPendingDeviceDeletions()).toEqual([]);
    });

    it("登録の応答をなくした端末の 404 は削除待ちに残し、続けて 3 回 404 なら済んだものとする", async () => {
      const { worker } = fakeWorker();
      const { client, requests } = fakeClient({
        "POST /api/e2ee/devices": () => {
          throw new TypeError("Failed to fetch");
        },
        [`DELETE /api/e2ee/devices/${DEVICE}`]: notFound,
      });
      const instance = manager(worker, client);
      await instance.start().catch(() => undefined);
      await instance.destroy();
      expect(requests.filter((r) => r.method === "DELETE")).toHaveLength(1);
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: DEVICE, unconfirmed: true, notFound: 1 },
      ]);

      // 次のログインごとに消し直す。
      await retryPendingDeviceDeletion(client, USER);
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: DEVICE, unconfirmed: true, notFound: 2 },
      ]);
      await retryPendingDeviceDeletion(client, USER);
      expect(MAX_NOT_FOUND).toBe(3);
      expect(readPendingDeviceDeletions()).toEqual([]);
    });

    it("同じログインの中の試し直しでは、404 を 1 回だけ数える", async () => {
      markDeviceDeletionPending({ userId: USER, deviceId: "d-old", unconfirmed: true });
      const { worker, calls } = fakeWorker({
        open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
      });
      let uploads = 0;
      const { client, requests } = fakeClient({
        "DELETE /api/e2ee/devices/d-old": notFound,
        // 起動の補充を何回か失敗させ、起動を試し直させる。
        "POST /api/e2ee/keys/upload": () => {
          uploads += 1;
          if (uploads <= 3) throw new ApiError(503, "busy");
          return { one_time_key_counts: { signed_curve25519: 50 }, unused_fallback_key_types: [] };
        },
      });
      const instance = manager(worker, client);
      for (let attempt = 0; attempt < 4; attempt += 1) {
        await instance.start().catch(() => undefined);
      }
      expect(calls.filter((call) => call.op === "open")).toHaveLength(4);
      expect(requests.filter((r) => r.path === "/api/e2ee/devices/d-old")).toHaveLength(4);
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: "d-old", unconfirmed: true, notFound: 1 },
      ]);
      await instance.stop();
    });

    it("起動の試し直しで manager を作り直しても、同じログインでは 404 を 1 回だけ数える", async () => {
      markDeviceDeletionPending({ userId: USER, deviceId: "d-old", unconfirmed: true });
      // E2eeProvider と同じく、起動に失敗したら止めて、manager を作り直す。
      const { worker } = fakeWorker({
        open: () => {
          throw new Error("暗号ストアを開けません");
        },
      });
      const { client, requests } = fakeClient({ "DELETE /api/e2ee/devices/d-old": notFound });
      const counted = new Set<string>();
      for (let attempt = 0; attempt < 3; attempt += 1) {
        const instance = new E2eeDeviceManager({
          config: resolveConfig({ DEV: false }),
          client,
          worker,
          userId: USER,
          namespace: `user-${USER}`,
          displayName: "Chrome（macOS）",
          subscribe,
          countedNotFound: counted,
        });
        await instance.start().catch(() => undefined);
        await instance.stop();
      }
      expect(requests.filter((r) => r.path === "/api/e2ee/devices/d-old")).toHaveLength(3);
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: "d-old", unconfirmed: true, notFound: 1 },
      ]);
    });

    it("ログインごとの集合は、ユーザーごとにページのあいだ同じものを返す", () => {
      expect(loginNotFoundCounter(USER)).toBe(loginNotFoundCounter(USER));
      expect(loginNotFoundCounter(USER)).not.toBe(loginNotFoundCounter("other"));
    });

    it("404 以外の失敗をはさんだら、404 の回数を数え直す", async () => {
      markDeviceDeletionPending({ userId: USER, deviceId: DEVICE, unconfirmed: true });
      const responses = [404, 404, 503, 404];
      const { client } = fakeClient({
        [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
          const status = responses.shift()!;
          throw new ApiError(status, "failed");
        },
      });
      const notFounds: (number | undefined)[] = [];
      // 次のログインごとに 1 回ずつ消し直す（404 → 404 → 503 → 404）。
      for (let login = 0; login < 4; login += 1) {
        await retryPendingDeviceDeletion(client, USER, new Set());
        notFounds.push(readPendingDeviceDeletions()[0]?.notFound);
      }
      expect(notFounds).toEqual([1, 2, undefined, 1]);
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: DEVICE, unconfirmed: true, notFound: 1 },
      ]);
    });

    it("あとから登録が届いた端末は、次のログインで消す", async () => {
      // ログアウトのために開いた（登録の応答をなくしていた）端末。
      const { worker } = fakeWorker();
      let registeredLate = false;
      const { client } = fakeClient({
        [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
          if (!registeredLate) notFound();
          return {};
        },
      });
      const instance = manager(worker, client);
      await instance.attach();
      await instance.destroy();
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: DEVICE, unconfirmed: true, notFound: 1 },
      ]);
      registeredLate = true;
      await retryPendingDeviceDeletion(client, USER);
      expect(readPendingDeviceDeletions()).toEqual([]);
    });

    it("作り直した前の端末は、登録済みでなければ 404 でも残す", async () => {
      const OLD = "00000000-0000-4000-8000-0000000000d0";
      const REGISTERED_OLD = "00000000-0000-4000-8000-0000000000d2";
      const unregistered = fakeWorker({
        open: {
          userId: USER,
          deviceId: DEVICE,
          registered: true,
          replacedDeviceId: OLD,
          replacedRegistered: false,
        },
      });
      const { client } = fakeClient({
        [`DELETE /api/e2ee/devices/${OLD}`]: notFound,
        [`DELETE /api/e2ee/devices/${REGISTERED_OLD}`]: notFound,
      });
      const first = manager(unregistered.worker, client);
      await first.start();
      await first.stop();
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: OLD, unconfirmed: true, notFound: 1 },
      ]);

      const registered = fakeWorker({
        open: {
          userId: USER,
          deviceId: DEVICE,
          registered: true,
          replacedDeviceId: REGISTERED_OLD,
          replacedRegistered: true,
        },
      });
      const second = manager(registered.worker, client);
      await second.start();
      await second.stop();
      // 登録済みだった前の端末は、404 で済んだものとする（OLD は 2 回目の 404）。
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: OLD, unconfirmed: true, notFound: 2 },
      ]);
    });

    it("同じ端末を登録済みとして足し直しても、unconfirmed と回数を保つ", () => {
      markDeviceDeletionPending({ userId: USER, deviceId: DEVICE, unconfirmed: true });
      updatePendingDeviceDeletion({
        userId: USER,
        deviceId: DEVICE,
        unconfirmed: true,
        notFound: 2,
      });
      markDeviceDeletionPending({ userId: USER, deviceId: DEVICE });
      expect(readPendingDeviceDeletions()).toEqual([
        { userId: USER, deviceId: DEVICE, unconfirmed: true, notFound: 2 },
      ]);
    });
  });

  it("削除待ちの一覧が壊れていたら、消し直さず、退避してから記録し直す", async () => {
    window.localStorage.setItem(CRYPTO_PENDING_DEVICE_DELETE_KEY, "{broken");
    const { client, requests } = fakeClient();
    await retryPendingDeviceDeletion(client, USER);
    expect(requests).toEqual([]);
    expect(() => readPendingDeviceDeletions()).toThrow();

    // ログアウトで削除 API が失敗しても、削除待ちに記録できる。読めない値は別のキーに残す。
    const { worker } = fakeWorker({
      open: { userId: USER, deviceId: DEVICE, registered: true, replacedDeviceId: null },
    });
    const failing = fakeClient({
      [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
        throw new ApiError(503, "offline");
      },
    });
    const instance = manager(worker, failing.client);
    await instance.start();
    await instance.destroy();
    expect(window.localStorage.getItem(CRYPTO_PENDING_DEVICE_DELETE_UNREADABLE_KEY)).toBe(
      "{broken",
    );
    expect(readPendingDeviceDeletions()).toEqual([{ userId: USER, deviceId: DEVICE }]);
  });

  it("同じ端末の削除待ちが重なっていても、1 回だけ消し直して足し直さない", async () => {
    const entry = { userId: USER, deviceId: DEVICE, unconfirmed: true, notFound: 2 };
    window.localStorage.setItem(CRYPTO_PENDING_DEVICE_DELETE_KEY, JSON.stringify([entry, entry]));
    const { client, requests } = fakeClient({
      [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
        throw new ApiError(404, "not found");
      },
    });
    await retryPendingDeviceDeletion(client, USER, new Set());
    expect(requests.filter((r) => r.method === "DELETE")).toHaveLength(1);
    expect(readPendingDeviceDeletions()).toEqual([]);
  });

  it.each(["null", "123", "{}", JSON.stringify([{ user_id: USER, device_id: DEVICE }])])(
    "形式の違う削除待ち（%s）は読めないものとして退避する",
    (raw) => {
      window.localStorage.setItem(CRYPTO_PENDING_DEVICE_DELETE_KEY, raw);
      expect(() => readPendingDeviceDeletions()).toThrow();
      markDeviceDeletionPending({ userId: USER, deviceId: DEVICE });
      expect(window.localStorage.getItem(CRYPTO_PENDING_DEVICE_DELETE_UNREADABLE_KEY)).toBe(raw);
      expect(readPendingDeviceDeletions()).toEqual([{ userId: USER, deviceId: DEVICE }]);
    },
  );

  it("前の形式の削除待ち（unconfirmed なし）は、登録済みの端末として扱う", async () => {
    window.localStorage.setItem(
      CRYPTO_PENDING_DEVICE_DELETE_KEY,
      JSON.stringify({ userId: USER, deviceId: DEVICE }),
    );
    const { client } = fakeClient({
      [`DELETE /api/e2ee/devices/${DEVICE}`]: () => {
        throw new ApiError(404, "not found");
      },
    });
    await retryPendingDeviceDeletion(client, USER);
    expect(readPendingDeviceDeletions()).toEqual([]);
  });
});
