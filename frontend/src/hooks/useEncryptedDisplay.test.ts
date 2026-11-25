// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { noteEncryptedChannel, resetSeenEncryptedChannels } from "../lib/e2ee/encryptionGuard";
import { useEncryptedDisplay } from "./useEncryptedDisplay";

const mocks = vi.hoisted(() => {
  const request = vi.fn();
  return { request, worker: { request } };
});

vi.mock("../lib/e2ee/e2ee-context", () => ({
  useE2ee: () => ({ worker: mocks.worker }),
}));

afterEach(() => {
  mocks.request.mockReset();
  window.localStorage.clear();
  resetSeenEncryptedChannels();
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
