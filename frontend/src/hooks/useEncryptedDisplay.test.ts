// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { noteEncryptedChannel, resetSeenEncryptedChannels } from "../lib/e2ee/encryptionGuard";
import { useEncryptedDisplay } from "./useEncryptedDisplay";

const mocks = vi.hoisted(() => {
  const request = vi.fn();
  const worker = { request };
  /** `useE2ee` が返す値。テストごとに差し替える。 */
  const context: { current: Record<string, unknown> } = { current: { worker } };
  return { request, worker, context };
});

vi.mock("../lib/e2ee/e2ee-context", () => ({
  useE2ee: () => mocks.context.current,
}));

afterEach(() => {
  mocks.request.mockReset();
  window.localStorage.clear();
  resetSeenEncryptedChannels();
  mocks.context.current = { worker: mocks.worker };
});

it("treats a previously recorded channel as encrypted even after the worker answers false", async () => {
  noteEncryptedChannel("c1");
  let resolveWorker!: (value: boolean) => void;
  mocks.request.mockImplementation(() => new Promise((resolve) => (resolveWorker = resolve)));
  const { result } = renderHook(() =>
    useEncryptedDisplay({ channelId: "c1", encrypted: false, hasCiphertext: false }),
  );
  await act(async () => {
    resolveWorker(false);
  });
  expect(result.current.treatAsEncrypted).toBe(true);
  expect(result.current.downgraded).toBe(true);
});

it("still treats an edm channel as encrypted", async () => {
  mocks.request.mockResolvedValue(false);
  const { result } = renderHook(() =>
    useEncryptedDisplay({
      channelId: "c2",
      channelName: "edm:alice:bob",
      encrypted: false,
      hasCiphertext: false,
    }),
  );
  expect(result.current.treatAsEncrypted).toBe(true);
});

it("treats a channel the crypto store remembers as encrypted, and checks until the store answers", async () => {
  let resolveWorker!: (value: boolean) => void;
  mocks.request.mockImplementation(() => new Promise((resolve) => (resolveWorker = resolve)));
  const { result } = renderHook(() =>
    useEncryptedDisplay({ channelId: "c3", encrypted: false, hasCiphertext: false }),
  );
  // 問い合わせの結果が出るまでは、確認中。まだ暗号化とは決めない。
  expect(result.current.checkingEncryption).toBe(true);
  expect(result.current.treatAsEncrypted).toBe(false);
  expect(mocks.request).toHaveBeenCalledWith({ op: "megolmIsKnownEncrypted", channelId: "c3" });
  await act(async () => {
    resolveWorker(true);
  });
  expect(result.current.checkingEncryption).toBe(false);
  expect(result.current.treatAsEncrypted).toBe(true);
});

it("keeps checking when the crypto store query fails", async () => {
  mocks.request.mockRejectedValue(new Error("store unavailable"));
  const { result } = renderHook(() =>
    useEncryptedDisplay({ channelId: "c4", encrypted: false, hasCiphertext: false }),
  );
  await act(async () => {});
  expect(result.current.checkingEncryption).toBe(true);
  expect(result.current.treatAsEncrypted).toBe(false);
});

it("does nothing while there is no channel to judge", async () => {
  mocks.request.mockResolvedValue(true);
  const { result } = renderHook(() =>
    useEncryptedDisplay({ channelId: null, encrypted: true, hasCiphertext: true }),
  );
  await act(async () => {});
  expect(mocks.request).not.toHaveBeenCalled();
  expect(result.current).toEqual({
    downgraded: false,
    checkingEncryption: false,
    treatAsEncrypted: false,
  });
  // 対象のないまま、暗号化チャンネルを覚えることもない。
  expect(Object.keys(window.localStorage)).toEqual([]);
});

const notReady = (kind: string) => ({ worker: null, notReadyReason: { kind } });
const judge = (holdWhileOpening?: boolean) =>
  renderHook(() =>
    useEncryptedDisplay({
      channelId: "c5",
      encrypted: false,
      hasCiphertext: false,
      holdWhileOpening,
    }),
  ).result;

it("holds while the crypto store is opening, only for callers that ask for it", () => {
  mocks.context.current = notReady("starting");
  expect(judge(true).current).toMatchObject({
    checkingEncryption: true,
    treatAsEncrypted: false,
  });
  // MessagePane とキャッチアップは指定しないので、今までどおり。
  expect(judge().current.checkingEncryption).toBe(false);
  expect(mocks.request).not.toHaveBeenCalled();
});

it.each([
  "unsupported",
  "otherTab",
  "offline",
  "failed",
  "deviceLimit",
  "signedOut",
  "userUnavailable",
])("does not hold when this tab cannot open the crypto store (%s)", (kind) => {
  mocks.context.current = notReady(kind);
  expect(judge(true).current.checkingEncryption).toBe(false);
});

it("does not hold outside the provider, where the store never opens", () => {
  mocks.context.current = { ...notReady("starting"), unprovided: true };
  expect(judge(true).current.checkingEncryption).toBe(false);
});

it("still treats a recorded channel as encrypted while the store is opening", () => {
  noteEncryptedChannel("c5");
  mocks.context.current = notReady("starting");
  expect(judge(true).current).toMatchObject({
    checkingEncryption: false,
    treatAsEncrypted: true,
  });
});
