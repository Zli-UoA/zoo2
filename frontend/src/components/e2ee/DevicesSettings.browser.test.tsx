import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";

import type { CryptoWorker } from "../../lib/e2ee/cryptoWorker";
import { E2eeContext, type E2eeContextValue } from "../../lib/e2ee/e2ee-context";
import * as controller from "../../lib/e2ee/revocationController";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { DevicesSettings } from "./DevicesSettings";

// Electric の行。`users` を差し替えて描画し直すと、`nameOf` が作り直されて一覧を読み直す。
const live = vi.hoisted(() => ({
  users: [] as { id: string; display_name: string }[],
  /** チャンネルとメンバーの行（同じ配列を返し続け、`peers` を作り直さない）。 */
  empty: [] as never[],
  /**
   * `useCollections` の値。描画のたびに同じ参照を返す（新しい config を返すと、API クライアントと
   * `load` が作り直され、描画のたびに読み直してしまい、意図した競合を確かめられない）。
   */
  collections: {
    config: { oidc: null, apiBaseUrl: "", devUserId: null, telemetryEnabled: false },
    channels: {},
    channelUsers: {},
  },
}));

vi.mock("@tanstack/react-db", () => ({ useLiveQuery: () => ({ data: live.empty }) }));
vi.mock("../../hooks/useRealtime", () => ({ useUsers: () => ({ data: live.users }) }));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => live.collections,
}));
vi.mock("../../lib/e2ee/revocationApi", async (original) => ({
  ...(await original<typeof import("../../lib/e2ee/revocationApi")>()),
  renameDevice: vi.fn(async () => {}),
}));
vi.mock("../../lib/e2ee/passkey", async (original) => ({
  ...(await original<typeof import("../../lib/e2ee/passkey")>()),
  passkeyPrfSupported: async () => false,
  passkeyRpId: () => null,
}));
vi.mock("../../lib/oidc/session", () => ({ getOidcClient: () => null }));
vi.mock("../../lib/e2ee/revocationController", () => ({
  RevocationConflictError: class RevocationConflictError extends Error {},
  RevocationChangedError: class RevocationChangedError extends Error {},
  RevocationPendingError: class RevocationPendingError extends Error {},
  loadDevices: vi.fn(),
  revocationStatus: vi.fn(),
  loadVerifications: vi.fn(),
  openSecretStorage: vi.fn(),
  deleteSelectedDevices: vi.fn(),
  beginRevocation: vi.fn(),
  newRevocationRecoveryKey: vi.fn(),
  continueRevocation: vi.fn(),
  resumeRevocation: vi.fn(),
  abandonRevocation: vi.fn(),
  addRevocationPasskey: vi.fn(),
  reverifyUser: vi.fn(),
}));

const mocked = vi.mocked(controller);
const DEVICE = "00000000-0000-4000-8000-0000000000d1";
const NEW_RECOVERY_KEY = "EsTc 1234 5678 abcd";

/** 読み直しの結果が描画に反映されるまで待つ。 */
const settle = () => new Promise((resolve) => setTimeout(resolve, 50));

function summary(scope: "device_only" | "full", stage = "prepared") {
  return {
    scope,
    stage,
    deleted_devices: [],
    resigned_users: [],
    resigned_master_keys: 0,
    notes: {},
    versions_without_key: [],
    key_id: "key",
    new_backup_version: null,
  } as never;
}

beforeEach(() => {
  vi.clearAllMocks();
  live.users = [{ id: "u1", display_name: "Alice" }];
  mocked.loadDevices.mockResolvedValue([
    {
      deviceId: DEVICE,
      displayName: "この端末",
      createdAt: "2026-11-01T00:00:00Z",
      lastSeenAt: "2026-11-14T00:00:00Z",
      current: true,
      signed: true,
    },
  ]);
  mocked.revocationStatus.mockResolvedValue(null);
  mocked.loadVerifications.mockResolvedValue({ users: [], warnings: [] });
  mocked.openSecretStorage.mockResolvedValue(undefined as never);
  mocked.deleteSelectedDevices.mockResolvedValue(undefined);
  mocked.beginRevocation.mockImplementation(async (_deps, scope) => summary(scope));
  mocked.newRevocationRecoveryKey.mockResolvedValue(NEW_RECOVERY_KEY);
});

function renderPanel() {
  const worker = {
    request: async ({ op }: { op: string }) => (op === "hasOwnKeys" ? true : null),
    terminate: () => {},
  } as unknown as CryptoWorker;
  const value: E2eeContextValue = {
    status: "ready" as E2eeContextValue["status"],
    deviceId: DEVICE,
    worker,
    userId: "u0",
    pendingBackupCount: async () => 0,
    revocationPending: async () => false,
    deviceLimit: null,
    cancelLogout: vi.fn(),
    requestKeyBackup: vi.fn(),
    clearForLogout: async () => {},
  };
  const tree = () => (
    <LocaleProvider>
      <E2eeContext.Provider value={value}>
        <DevicesSettings />
      </E2eeContext.Provider>
    </LocaleProvider>
  );
  const view = render(tree());
  return {
    /** Electric の `users` の行が変わったときと同じく、一覧を読み直させる。 */
    async changeRows() {
      const calls = mocked.loadDevices.mock.calls.length;
      live.users = [...live.users, { id: `u${calls + 2}`, display_name: "Bob" }];
      view.rerender(tree());
      await waitFor(() => expect(mocked.loadDevices.mock.calls.length).toBeGreaterThan(calls));
    },
  };
}

async function startRevocation(scope: "device_only" | "full") {
  await userEvent.click(await screen.findByRole("button", { name: "失効を始める" }));
  if (scope === "full") {
    await userEvent.click(screen.getByRole("radio", { name: /パスキー、リカバリーキー/ }));
  }
  await userEvent.click(screen.getByRole("checkbox", { name: /AuthZ でセッション/ }));
  await userEvent.click(screen.getByRole("button", { name: "次へ" }));
  await userEvent.type(screen.getByLabelText("リカバリーキー"), "old key");
  await userEvent.click(screen.getByRole("button", { name: "リカバリーキーで解錠" }));
}

it("keeps the new recovery key on screen when Electric rows change", async () => {
  const panel = renderPanel();
  await startRevocation("full");
  expect(await screen.findByText(NEW_RECOVERY_KEY)).toBeVisible();

  await panel.changeRows();
  await settle();
  expect(screen.getByText(NEW_RECOVERY_KEY)).toBeVisible();
});

it("keeps the recovery key of a resumed full revocation when Electric rows change", async () => {
  // 1〜5 を SSSS の保存の前に止めた失効を、記録した範囲で最初からやり直す。
  mocked.revocationStatus.mockResolvedValue(summary("full"));
  // 本物の `resumeRevocation` と同じく、やり直すときは記録を消す。
  mocked.resumeRevocation.mockImplementation(async () => {
    mocked.revocationStatus.mockResolvedValue(null);
    return { kind: "restart", scope: "full", deletedDevices: ["lost"] };
  });
  const panel = renderPanel();
  await userEvent.click(await screen.findByRole("button", { name: "失効を続ける" }));
  await userEvent.type(screen.getByLabelText("リカバリーキー"), "old key");
  await userEvent.click(screen.getByRole("button", { name: "リカバリーキーで解錠" }));
  expect(await screen.findByText(NEW_RECOVERY_KEY)).toBeVisible();
  expect(mocked.beginRevocation).toHaveBeenCalledWith(expect.anything(), "full", ["lost"]);

  // useUsers の行を新しい配列に替え、背景の読み直しが終わるのを待つ。
  await panel.changeRows();
  await settle();
  expect(screen.getByText(NEW_RECOVERY_KEY)).toBeVisible();
});

it("does not offer to resume a revocation that is still running when rows change", async () => {
  let finish!: (result: Awaited<ReturnType<typeof controller.continueRevocation>>) => void;
  mocked.continueRevocation.mockImplementation(() => new Promise((resolve) => (finish = resolve)));
  const panel = renderPanel();
  await startRevocation("device_only");
  await userEvent.click(await screen.findByRole("button", { name: "失効を実行" }));
  await waitFor(() => expect(mocked.continueRevocation).toHaveBeenCalledTimes(1));

  // 実行中は暗号ストアに進み具合があるので、読み直すと「終わっていない失効」が返る。
  mocked.revocationStatus.mockResolvedValue(summary("device_only", "secret_storage_saved"));
  await panel.changeRows();
  await settle();
  expect(screen.queryByRole("button", { name: "失効を続ける" })).toBeNull();
  expect(mocked.resumeRevocation).not.toHaveBeenCalled();

  mocked.revocationStatus.mockResolvedValue(null);
  finish({ summary: summary("device_only", "verified"), unsignedDevices: [], droppedSessions: 0 });
  expect(await screen.findByRole("button", { name: "閉じる" })).toBeVisible();
});

it("keeps the selection screen when a reload started earlier returns late or fails", async () => {
  const panel = renderPanel();
  await screen.findByRole("button", { name: "失効を始める" });

  // 読み直しを始めてから、返る前に失効を始める。
  let resolveDevices!: (rows: Awaited<ReturnType<typeof controller.loadDevices>>) => void;
  mocked.loadDevices.mockImplementationOnce(
    () => new Promise((resolve) => (resolveDevices = resolve)),
  );
  await panel.changeRows();
  await userEvent.click(screen.getByRole("button", { name: "失効を始める" }));
  await userEvent.click(screen.getByRole("radio", { name: /パスキー、リカバリーキー/ }));
  resolveDevices([]);
  await settle();
  expect(screen.getByRole("radio", { name: /パスキー、リカバリーキー/ })).toBeChecked();
  expect(screen.getByRole("button", { name: "次へ" })).toBeVisible();

  // 読み直しに失敗しても、読み込みの失敗の画面に変えない。
  mocked.loadDevices.mockRejectedValueOnce(new TypeError("network error"));
  await panel.changeRows();
  await settle();
  expect(screen.getByRole("button", { name: "次へ" })).toBeVisible();
});

it("keeps the list and offers a retry when reloading after a rename fails", async () => {
  renderPanel();
  await userEvent.click(await screen.findByRole("button", { name: "端末名を変える" }));
  const loads = mocked.loadDevices.mock.calls.length;
  mocked.loadDevices.mockRejectedValueOnce(new TypeError("network error"));
  await userEvent.clear(screen.getByLabelText("端末名"));
  await userEvent.type(screen.getByLabelText("端末名"), "新しい名前");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByText(/端末の一覧を読み直せませんでした/)).toBeVisible();
  // 一覧は残している。
  expect(screen.getByRole("button", { name: "失効を始める" })).toBeVisible();
  expect(mocked.loadDevices.mock.calls.length).toBe(loads + 1);

  await userEvent.click(screen.getByRole("button", { name: "もう一度試す" }));
  await waitFor(() => expect(screen.queryByText(/端末の一覧を読み直せませんでした/)).toBeNull());
  expect(mocked.loadDevices.mock.calls.length).toBe(loads + 2);
});
