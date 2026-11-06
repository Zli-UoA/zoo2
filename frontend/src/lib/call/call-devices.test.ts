// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vite-plus/test";

import { loadSelectedDevice, saveSelectedDevice, toDeviceOptions } from "./call-devices";

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
});

describe("toDeviceOptions", () => {
  const devices = [
    { kind: "audioinput", deviceId: "default", label: "Default" },
    { kind: "audioinput", deviceId: "m1", label: "USB Mic" },
    { kind: "audioinput", deviceId: "m2", label: "" },
    { kind: "audiooutput", deviceId: "s1", label: "Speakers" },
    { kind: "videoinput", deviceId: "v1", label: "Cam" },
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
