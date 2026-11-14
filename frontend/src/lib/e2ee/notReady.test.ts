import { describe, expect, it } from "vite-plus/test";

import { ApiError } from "../api/client";
import { describeStartFailure, notReadyReason, type NotReadyInput } from "./notReady";

const base: NotReadyInput = {
  supported: true,
  userId: "00000000-0000-4000-8000-00000000000a",
  isOnline: true,
  status: "starting",
  ready: false,
  lockState: "held",
  deviceLimit: false,
  startFailure: null,
};

describe("notReadyReason", () => {
  it("開けていれば null", () => {
    expect(notReadyReason({ ...base, status: "ready", ready: true })).toBeNull();
  });

  it("非対応のブラウザーは、ほかの理由より先に出す", () => {
    expect(notReadyReason({ ...base, supported: false, isOnline: false })).toEqual({
      kind: "unsupported",
    });
    expect(notReadyReason({ ...base, status: "unsupported" })).toEqual({ kind: "unsupported" });
  });

  it("ユーザーが分からない", () => {
    expect(notReadyReason({ ...base, userId: null })).toEqual({ kind: "signedOut" });
  });

  it("オフライン", () => {
    expect(notReadyReason({ ...base, isOnline: false, lockState: "otherTab" })).toEqual({
      kind: "offline",
    });
  });

  it("ほかのタブがロックを持っている", () => {
    expect(notReadyReason({ ...base, status: "idle", lockState: "otherTab" })).toEqual({
      kind: "otherTab",
    });
  });

  it("端末の上限は、失敗より先に出す", () => {
    expect(
      notReadyReason({
        ...base,
        status: "stopped",
        deviceLimit: true,
        startFailure: { source: "server", status: 409, code: "E2EE_DEVICE_LIMIT" },
      }),
    ).toEqual({ kind: "deviceLimit" });
  });

  it("起動に失敗して試し直している", () => {
    const failure = { source: "server", status: 403, code: "E2EE_DISABLED" } as const;
    expect(notReadyReason({ ...base, status: "stopped", startFailure: failure })).toEqual({
      kind: "failed",
      failure,
    });
  });

  it("ほかに理由がなければ、開いているところ", () => {
    expect(notReadyReason({ ...base, status: "idle", lockState: "idle" })).toEqual({
      kind: "starting",
    });
    // ready でも Worker がまだなければ、開いているところとして扱う。
    expect(notReadyReason({ ...base, status: "ready", ready: false })).toEqual({
      kind: "starting",
    });
  });
});

describe("describeStartFailure", () => {
  it("サーバーのエラーは、状態コードと本文の code を残す", () => {
    expect(
      describeStartFailure(new ApiError(403, "forbidden", { code: "E2EE_FORBIDDEN" })),
    ).toEqual({ source: "server", status: 403, code: "E2EE_FORBIDDEN" });
    expect(describeStartFailure(new ApiError(503, "unavailable", "busy"))).toEqual({
      source: "server",
      status: 503,
      code: null,
    });
  });

  it("fetch の失敗は、つながらなかったとして扱う", () => {
    expect(describeStartFailure(new TypeError("Failed to fetch"))).toEqual({ source: "network" });
  });

  it("それ以外は、この端末での失敗として、エラーの名前だけを残す", () => {
    const error = new Error("secret detail");
    error.name = "NoModificationAllowedError";
    expect(describeStartFailure(error)).toEqual({
      source: "local",
      name: "NoModificationAllowedError",
    });
    expect(describeStartFailure(new Error("plain"))).toEqual({ source: "local", name: null });
    expect(describeStartFailure("oops")).toEqual({ source: "local", name: null });
  });
});
