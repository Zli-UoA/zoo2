import { act, render } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";
import type { AppConfig } from "../../lib/config";
import { useE2ee, type E2eeContextValue } from "../../lib/e2ee/e2ee-context";
import { E2eeProvider } from "./E2eeProvider";

const mocks = vi.hoisted(() => ({
  attachGate: null as null | Promise<void>,
  managers: [] as Array<{
    suspend: ReturnType<typeof vi.fn>;
    resume: ReturnType<typeof vi.fn>;
    pendingBackupCount: ReturnType<typeof vi.fn>;
  }>,
}));

vi.mock("../../lib/e2ee/support", () => ({ supportsCryptoStore: () => true }));
vi.mock("../../lib/e2ee/cryptoWorker", () => ({
  startCryptoWorker: () => ({ terminate: () => {}, request: async () => ({}) }),
}));
vi.mock("../../lib/offline/offline-context", () => ({
  // オフライン扱いにして、通常の起動（リーダーの選出）は動かさない。ログアウトの経路だけを見る。
  useOfflineStatus: () => ({ namespace: "ns-test", isOnline: false }),
}));
vi.mock("../../lib/offline/identity", async (original) => ({
  ...(await original<typeof import("../../lib/offline/identity")>()),
  readLastOfflineUserId: () => null,
}));
vi.mock("../../lib/e2ee/deviceManager", () => ({
  E2eeDeviceManager: class {
    suspend = vi.fn(async () => {});
    resume = vi.fn();
    pendingBackupCount = vi.fn(async () => 1);
    stop = vi.fn(async () => {});
    constructor() {
      mocks.managers.push(this);
    }
    async attach() {
      await mocks.attachGate;
    }
  },
}));

const config = { devUserId: "user-a" } as unknown as AppConfig;

function mount() {
  let ctx!: E2eeContextValue;
  function Probe() {
    ctx = useE2ee();
    return null;
  }
  render(
    <E2eeProvider config={config}>
      <Probe />
    </E2eeProvider>,
  );
  return { get: () => ctx };
}

it("does not suspend the manager when the logout is cancelled while the store is still opening", async () => {
  mocks.managers.length = 0;
  let openStore!: () => void;
  mocks.attachGate = new Promise<void>((resolve) => (openStore = resolve));
  const { get } = mount();
  let outcome: unknown = "pending";
  const count = get()
    .pendingBackupCount()
    .then(
      (value) => (outcome = value),
      (error: unknown) => (outcome = error),
    );
  // ストアを開いている間に画面を離れて、ログアウトを取りやめる。
  await act(async () => get().cancelLogout());
  // 遅れてストアが開く。
  await act(async () => {
    openStore();
    await count;
  });
  expect(outcome).toBeInstanceOf(Error);
  expect(mocks.managers).toHaveLength(1);
  expect(mocks.managers[0].suspend).not.toHaveBeenCalled();
  expect(mocks.managers[0].pendingBackupCount).not.toHaveBeenCalled();
});

it("counts normally when the logout is not cancelled", async () => {
  mocks.managers.length = 0;
  mocks.attachGate = Promise.resolve();
  const { get } = mount();
  let result = -1;
  await act(async () => {
    result = await get().pendingBackupCount();
  });
  expect(result).toBe(1);
  expect(mocks.managers[0].suspend).toHaveBeenCalledTimes(1);
  await act(async () => get().cancelLogout());
});
