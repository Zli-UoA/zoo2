import { describe, expect, it, vi } from "vite-plus/test";

import { notifyTrustChanges, type CryptoWorker } from "./cryptoWorker";

function fakeWorker(fail = false) {
  const terminate = vi.fn();
  const worker: CryptoWorker = {
    request: (async () => {
      if (fail) throw new Error("failed");
      return null;
    }) as CryptoWorker["request"],
    terminate,
  };
  return { worker, terminate };
}

describe("notifyTrustChanges", () => {
  it("解錠、相手の受け入れ、失効の保存が終わったら知らせる", async () => {
    const trustChanged = vi.fn();
    const worker = notifyTrustChanges(fakeWorker().worker, trustChanged);
    await worker.request({ op: "finishSetup", secretsRevision: 1 });
    await worker.request({ op: "confirmAcceptedMasterKey", targetUserId: "u", masterKey: "k" });
    await worker.request({
      op: "revocationCrossSigningSaved",
      revocationId: "r",
      secretsRevision: 2,
    });
    expect(trustChanged).toHaveBeenCalledTimes(3);
  });

  it("TOFU のピンを置けたときだけ知らせる", async () => {
    const trustChanged = vi.fn();
    const worker = notifyTrustChanges(fakeWorker().worker, trustChanged);
    const pin = { user_id: "u", target_user_id: "t", master_key: "k" } as never;
    await worker.request({ op: "trustPinResult", pin, registered: false });
    await worker.request({ op: "megolmSettlePins", confirmed: [], discarded: [pin] });
    expect(trustChanged).not.toHaveBeenCalled();
    await worker.request({ op: "trustPinResult", pin, registered: true });
    await worker.request({ op: "megolmSettlePins", confirmed: [pin], discarded: [] });
    expect(trustChanged).toHaveBeenCalledTimes(2);
  });

  it("ほかの操作と、失敗した操作では知らせない", async () => {
    const trustChanged = vi.fn();
    await notifyTrustChanges(fakeWorker().worker, trustChanged).request({ op: "hasOwnKeys" });
    await expect(
      notifyTrustChanges(fakeWorker(true).worker, trustChanged).request({
        op: "finishSetup",
        secretsRevision: 1,
      }),
    ).rejects.toThrow("failed");
    expect(trustChanged).not.toHaveBeenCalled();
  });

  it("止めると元の Worker を止める", () => {
    const { worker, terminate } = fakeWorker();
    notifyTrustChanges(worker, vi.fn()).terminate();
    expect(terminate).toHaveBeenCalledTimes(1);
  });
});
