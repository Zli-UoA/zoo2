import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError, type ApiClient } from "../api/client";
import { resolveConfig } from "../config";
import type { CryptoWorker } from "./cryptoWorker";
import { E2eeDeviceManager, retryPendingDeviceDeletion } from "./deviceManager";
import { markDeviceDeletionPending, readPendingDeviceDeletions } from "./identity";
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
    processToDevice: { processed: ["1"], roomKeys: 1, dropped: 0, newOlmSessions: 1 },
    pendingBackupCount: 2,
    ...overrides,
  };
  const worker = {
    request: vi.fn(async (operation: CryptoOperation) => {
      calls.push(operation);
      return results[operation.op] ?? null;
    }),
    terminate: vi.fn(),
  };
  return { worker: worker as unknown as CryptoWorker, calls };
}

function fakeClient(handlers: Record<string, (body: unknown) => unknown> = {}) {
  const requests: { method: string; path: string; body: unknown }[] = [];
  const counts = {
    one_time_key_counts: { signed_curve25519: 50 },
    unused_fallback_key_types: ["signed_curve25519"],
  };
  const respond = (method: string, path: string, body: unknown) => {
    requests.push({ method, path, body });
    const handler = handlers[`${method} ${path}`];
    if (handler) return handler(body);
    if (path === "/api/e2ee/keys/query") return { device_keys: {} };
    return counts;
  };
  const client = {
    request: vi.fn(async (path: string, options: { method?: string; body?: unknown } = {}) =>
      respond(options.method ?? "GET", path, options.body),
    ),
    post: vi.fn(async (path: string, body: unknown) => ({
      data: await respond("POST", path, body),
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

function manager(worker: CryptoWorker, client: ApiClient) {
  return new E2eeDeviceManager({
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
});
