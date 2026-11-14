import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";

import { resolveConfig } from "../../lib/config";
import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { cryptoLockName } from "../../lib/e2ee/identity";
import { E2eeProvider } from "./E2eeProvider";

type OnStatus = (status: string, error?: unknown) => void;

const mocks = vi.hoisted(() => ({
  /** 作られた manager の `onStatus`（新しい順に後ろへ足す）。 */
  managers: [] as OnStatus[],
  startCryptoWorker: vi.fn(),
}));

// 元のモジュールの書き出しを引き継ぎ、差し替える物だけを上書きする（Provider が新しい書き出しを使っても
// 読み込みで落ちないように）。
vi.mock("../../lib/e2ee/cryptoWorker", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  startCryptoWorker: () => mocks.startCryptoWorker(),
}));

vi.mock("../../lib/e2ee/deviceManager", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  E2eeDeviceManager: class {
    currentDeviceId = "00000000-0000-4000-8000-0000000000d1";
    private readonly onStatus: OnStatus;
    constructor(options: { onStatus?: OnStatus }) {
      this.onStatus = options.onStatus ?? (() => {});
      mocks.managers.push(this.onStatus);
    }
    async start() {
      this.onStatus("starting");
      this.onStatus("ready");
    }
    async stop() {
      this.onStatus("stopped");
    }
    async suspend() {}
    resume() {}
  },
}));

vi.mock("../../lib/e2ee/sasController", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  SasController: class {
    async handleMessages() {}
    dispose() {}
  },
}));

vi.mock("../../lib/e2ee/setupController", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  runStartupCheck: async () => {},
  uploadKeyBackup: async () => {},
}));

vi.mock("../../lib/e2ee/push/recipient", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  rememberPushRecipient: async () => {},
  forgetPushRecipient: async () => {},
}));

const USER = "00000000-0000-4000-8000-00000000000a";
const userState = { currentUserId: USER, isAdmin: false, error: null };
vi.mock("../../lib/currentUser", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  getCurrentUserStore: () => ({ subscribe: () => () => {}, getState: () => userState }),
}));

const config = resolveConfig({}, { devUserId: USER });

function Probe() {
  const { notReadyReason } = useE2ee();
  return <output data-testid="reason">{JSON.stringify(notReadyReason)}</output>;
}

function reason() {
  return JSON.parse(screen.getByTestId("reason").textContent ?? "null");
}

beforeEach(() => {
  mocks.managers.length = 0;
  mocks.startCryptoWorker.mockReset();
  mocks.startCryptoWorker.mockImplementation(() => ({
    request: async () => null,
    terminate: () => {},
  }));
});

afterEach(() => {
  cleanup();
});

it("準備ができたあとの登録し直しの失敗も、理由に出し、成功したら消す", async () => {
  render(
    <E2eeProvider config={config}>
      <Probe />
    </E2eeProvider>,
  );
  await waitFor(() => expect(reason()).toBeNull());

  // 別の端末から消されて登録し直し（`reregister`）、通信に失敗した。
  const onStatus = mocks.managers.at(-1)!;
  act(() => onStatus("error", new TypeError("Failed to fetch")));
  expect(reason()).toEqual({ kind: "failed", failure: { source: "network" } });

  act(() => onStatus("ready"));
  expect(reason()).toBeNull();
});

it("Worker を作れなかったときは、ほかのタブではなく、この端末での失敗として出す", async () => {
  mocks.startCryptoWorker.mockImplementation(() => {
    const error = new Error("blocked");
    error.name = "SecurityError";
    throw error;
  });
  render(
    <E2eeProvider config={config}>
      <Probe />
    </E2eeProvider>,
  );
  await waitFor(() =>
    expect(reason()).toEqual({
      kind: "failed",
      failure: { source: "local", name: "SecurityError" },
    }),
  );
});

it("Worker を作るときの TypeError は、通信の失敗ではなく、この端末での失敗として出す", async () => {
  // CSP の Trusted Types で既定のポリシーがないと、`Worker()` は TypeError を投げる。
  mocks.startCryptoWorker.mockImplementation(() => {
    throw new TypeError("This document requires 'TrustedScriptURL' assignment.");
  });
  render(
    <E2eeProvider config={config}>
      <Probe />
    </E2eeProvider>,
  );
  await waitFor(() =>
    expect(reason()).toEqual({ kind: "failed", failure: { source: "local", name: "TypeError" } }),
  );
});

it("ほかのタブがロックを持っているあいだはそう出し、返されたら引き継ぐ", async () => {
  let releaseOther: () => void = () => {};
  const otherHolds = new Promise<void>((resolve) => {
    releaseOther = resolve;
  });
  const holding = new Promise<void>((resolve) => {
    void navigator.locks.request(cryptoLockName("default"), async () => {
      resolve();
      await otherHolds;
    });
  });
  await holding;

  render(
    <E2eeProvider config={config}>
      <Probe />
    </E2eeProvider>,
  );
  await waitFor(() => expect(reason()).toEqual({ kind: "otherTab" }));

  releaseOther();
  await waitFor(() => expect(reason()).toBeNull());
});
