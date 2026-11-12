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

  it("wasm の失敗を code 付きで返す", async () => {
    worker = startCryptoWorker();
    await expect(
      worker.request({ op: "generateDeviceKeys", userId: "not-a-uuid", deviceId: "x" }),
    ).rejects.toMatchObject({ name: "ZooCryptoError", code: "InvalidId" });
  });
});
