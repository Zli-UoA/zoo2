import { describe, expect, it } from "vite-plus/test";

import { ApiError } from "../api/client";
import {
  describeLocalFailure,
  describeStartFailure,
  notReadyReason,
  type NotReadyInput,
} from "./notReady";

const base: NotReadyInput = {
  supported: true,
  userId: "00000000-0000-4000-8000-00000000000a",
  userLoading: false,
  userError: false,
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

  it("ユーザーの取得が終わり、失敗もしていないのにユーザーがいない", () => {
    expect(notReadyReason({ ...base, userId: null })).toEqual({ kind: "signedOut" });
  });

  it("ユーザーを取得しているあいだは、ログインし直しではなく準備中と出す", () => {
    expect(notReadyReason({ ...base, userId: null, userLoading: true })).toEqual({
      kind: "starting",
    });
  });

  it("ユーザーの取得が一時的に失敗したときは、ログインし直しを案内しない", () => {
    expect(notReadyReason({ ...base, userId: null, userError: true })).toEqual({
      kind: "userUnavailable",
    });
  });

  it("オフラインで起動してユーザーが分からないときは、ログインし直しではなくオフラインと出す", () => {
    expect(notReadyReason({ ...base, userId: null, isOnline: false, status: "idle" })).toEqual({
      kind: "offline",
    });
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

describe("describeLocalFailure", () => {
  it("TypeError も、この端末での失敗として扱う", () => {
    expect(describeLocalFailure(new TypeError("Trusted Types"))).toEqual({
      source: "local",
      name: "TypeError",
    });
  });
});
