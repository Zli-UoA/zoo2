// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { noteEncryptedChannel } from "../lib/e2ee/encryptionGuard";
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
