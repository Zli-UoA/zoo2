// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { applySink, loadSelectedDevice, saveSelectedDevice, toDeviceOptions } from "./call-devices";

describe("selected device storage", () => {
  beforeEach(() => window.localStorage.clear());
  it("既定は空文字", () => {
    expect(loadSelectedDevice("mic")).toBe("");
  });
  it("保存と削除", () => {
    saveSelectedDevice("mic", "abc");
    expect(loadSelectedDevice("mic")).toBe("abc");
    expect(loadSelectedDevice("speaker")).toBe("");
    saveSelectedDevice("mic", "");
    expect(loadSelectedDevice("mic")).toBe("");
  });
  describe("保存に失敗する環境", () => {
    afterEach(() => {
      vi.restoreAllMocks();
      // 成功する保存で現在ページ内の値を初期化する
      saveSelectedDevice("mic", "");
      saveSelectedDevice("speaker", "");
    });
    it("保存できなくても現在のページでは選択を反映する", () => {
      vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
        throw new Error("QuotaExceededError");
      });
      saveSelectedDevice("mic", "abc");
      expect(loadSelectedDevice("mic")).toBe("abc");
      expect(loadSelectedDevice("speaker")).toBe("");
    });
  });
});

describe("toDeviceOptions", () => {
  const devices = [
    { kind: "audioinput", deviceId: "default", label: "Default" },
    { kind: "audioinput", deviceId: "m1", label: "USB Mic" },
    { kind: "audioinput", deviceId: "m2", label: "" },
    { kind: "audiooutput", deviceId: "s1", label: "Speakers" },
    { kind: "videoinput", deviceId: "v1", label: "Cam" },
    { kind: "audioinput", deviceId: "", label: "" },
  ] as MediaDeviceInfo[];
  it("種別で絞り、default を除き、ラベル無しは汎用名にする", () => {
    expect(toDeviceOptions(devices, "audioinput", (n) => `Mic ${n}`)).toEqual([
      { deviceId: "m1", label: "USB Mic" },
      { deviceId: "m2", label: "Mic 2" },
    ]);
    expect(toDeviceOptions(devices, "audiooutput", (n) => `S ${n}`)).toEqual([
      { deviceId: "s1", label: "Speakers" },
    ]);
  });
});

describe("applySink", () => {
  const audioWith = (setSinkId?: (id: string) => Promise<void>) =>
    ({ setSinkId }) as unknown as HTMLMediaElement;

  it("適用できれば true", async () => {
    const setSinkId = vi.fn().mockResolvedValue(undefined);
    expect(await applySink(audioWith(setSinkId), "s1")).toBe(true);
    expect(setSinkId).toHaveBeenCalledWith("s1");
  });
  it("setSinkId が失敗したら false", async () => {
    expect(
      await applySink(
        audioWith(() => Promise.reject(new Error("NotAllowed"))),
        "s1",
      ),
    ).toBe(false);
  });
  it("非対応の環境では何もせず true", async () => {
    expect(await applySink(audioWith(undefined), "s1")).toBe(true);
  });
});
