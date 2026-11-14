import { afterEach, describe, expect, it } from "vite-plus/test";

import { startCryptoWorker, type CryptoWorker } from "./cryptoWorker";

const USER = "00000000-0000-4000-8000-00000000000a";

let worker: CryptoWorker | undefined;

afterEach(() => {
  worker?.terminate();
  worker = undefined;
});

describe("zoo-crypto Worker", () => {
  it("ブラウザーの Worker で wasm を読み込み、鍵を作れる", async () => {
    worker = startCryptoWorker();
    const deviceId = crypto.randomUUID();
    const first = await worker.request({ op: "generateDeviceKeys", userId: USER, deviceId });
    expect(first.deviceKeys).toMatchObject({ user_id: USER, device_id: deviceId });
    expect(first.oneTimeKeyCount).toBe(50);
    // getrandom の wasm_js バックエンド（crypto.getRandomValues）が Worker で動く。
    const second = await worker.request({ op: "generateDeviceKeys", userId: USER, deviceId });
    expect(second.identityKeys.curve25519).not.toBe(first.identityKeys.curve25519);
  });

  it("OPFS の SQLite に暗号ストアを置き、開き直すと同じ端末に戻る", async () => {
    const namespace = `test-${crypto.randomUUID()}`;
    worker = startCryptoWorker();
    const opened = await worker.request({ op: "open", namespace, userId: USER });
    expect(opened).toMatchObject({ userId: USER, registered: false, replacedDeviceId: null });
    const request = await worker.request({ op: "registrationRequest" });
    expect(request.device_keys).toMatchObject({ device_id: opened.deviceId });
    await worker.request({ op: "markRegistered" });
    await worker.request({ op: "close" });
    worker.terminate();

    // 別の Worker（タブを開き直したのと同じ）で開いても、同じ端末に戻る。
    worker = startCryptoWorker();
    const reopened = await worker.request({ op: "open", namespace, userId: USER });
    expect(reopened).toMatchObject({ deviceId: opened.deviceId, registered: true });
    expect(await worker.request({ op: "pendingBackupCount" })).toBe(0);
    await worker.request({ op: "destroy", namespace });

    const root = await navigator.storage.getDirectory();
    const names: string[] = [];
    for await (const name of (root as unknown as { keys(): AsyncIterable<string> }).keys()) {
      names.push(name);
    }
    expect(names.filter((name) => name.startsWith("zoo-crypto-"))).toEqual([]);
  });

  it("端末だけを作り直すと、OPFS の SQLite に新しい端末が残り、開き直しても同じ端末に戻る", async () => {
    // 別の端末から消された端末を作り直すとき（deviceManager の replaceDevice）。
    const namespace = `test-${crypto.randomUUID()}`;
    worker = startCryptoWorker();
    const first = await worker.request({ op: "open", namespace, userId: USER });
    await worker.request({ op: "markRegistered" });
    const replaced = await worker.request({ op: "replaceDevice", namespace, userId: USER });
    expect(replaced).toMatchObject({ userId: USER, registered: false, replacedDeviceId: null });
    expect(replaced.deviceId).not.toBe(first.deviceId);
    const request = await worker.request({ op: "registrationRequest" });
    expect(request.device_keys).toMatchObject({ device_id: replaced.deviceId });
    await worker.request({ op: "close" });
    worker.terminate();

    worker = startCryptoWorker();
    const reopened = await worker.request({ op: "open", namespace, userId: USER });
    expect(reopened).toMatchObject({ deviceId: replaced.deviceId, registered: false });
    await worker.request({ op: "destroy", namespace });
  });

  it("同じ Worker で暗号ストアを消してから、同じ名前で作り直せる", async () => {
    // 別の端末から消された端末を作り直すとき（deviceManager の start）と同じ順。
    const namespace = `test-${crypto.randomUUID()}`;
    worker = startCryptoWorker();
    const first = await worker.request({ op: "open", namespace, userId: USER });
    await worker.request({ op: "destroy", namespace });
    const second = await worker.request({ op: "open", namespace, userId: USER });
    expect(second.deviceId).not.toBe(first.deviceId);
    const request = await worker.request({ op: "registrationRequest" });
    expect(request.device_keys).toMatchObject({ device_id: second.deviceId });
    await worker.request({ op: "destroy", namespace });
  });

  it("wasm の失敗を code 付きで返す", async () => {
    worker = startCryptoWorker();
    await expect(
      worker.request({ op: "generateDeviceKeys", userId: "not-a-uuid", deviceId: "x" }),
    ).rejects.toMatchObject({ name: "ZooCryptoError", code: "InvalidId" });
  });
});
