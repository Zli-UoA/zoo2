import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { E2eeContext, type E2eeContextValue } from "../../lib/e2ee/e2ee-context";
import { LogoutButton } from "./LogoutSetting";

vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: { oidc: { issuer: "https://id.example", clientId: "zoo" } } }),
}));
vi.mock("../../lib/offline/offline-context", () => ({
  useOfflineStatus: () => ({ clearUserData: async () => {} }),
}));
vi.mock("../../lib/oidc/session", () => ({ getOidcClient: () => null }));

function e2ee(
  pendingBackupCount: E2eeContextValue["pendingBackupCount"],
  revocationPending: E2eeContextValue["revocationPending"] = async () => false,
) {
  const cancelLogout = vi.fn();
  const clearForLogout = vi.fn(async () => {});
  const value: E2eeContextValue = {
    status: "idle",
    deviceId: null,
    worker: null,
    notReadyReason: { kind: "starting" },
    roomKeysVersion: 0,
    userId: null,
    pendingBackupCount,
    revocationPending,
    deviceLimit: null,
    cancelLogout,
    requestKeyBackup: vi.fn(),
    requestKeyBackupImport: vi.fn(),
    clearForLogout,
    sas: null,
  };
  return { value, cancelLogout, clearForLogout };
}

function renderButton(value: E2eeContextValue) {
  return render(
    <LocaleProvider>
      <E2eeContext.Provider value={value}>
        <LogoutButton />
      </E2eeContext.Provider>
    </LocaleProvider>,
  );
}

it("cancels the suspended logout when the settings screen is left during the key count", async () => {
  let resolveCount!: (count: number) => void;
  const { value, cancelLogout, clearForLogout } = e2ee(
    () => new Promise<number>((resolve) => (resolveCount = resolve)),
  );
  const view = renderButton(value);
  await userEvent.click(screen.getByRole("button", { name: "ログアウト" }));
  view.unmount();
  expect(cancelLogout).toHaveBeenCalledTimes(1);
  resolveCount(1);
  await Promise.resolve();
  // 遅れて返った結果では、ログアウトを続けず、二重に取りやめもしない。
  expect(clearForLogout).not.toHaveBeenCalled();
  expect(cancelLogout).toHaveBeenCalledTimes(1);
});

it("cancels the suspended logout when the screen is left while the confirmation is open", async () => {
  const { value, cancelLogout } = e2ee(async () => 2);
  const view = renderButton(value);
  await userEvent.click(screen.getByRole("button", { name: "ログアウト" }));
  await screen.findByRole("alertdialog");
  view.unmount();
  expect(cancelLogout).toHaveBeenCalledTimes(1);
});

it("does not cancel when nothing was started", () => {
  const { value, cancelLogout } = e2ee(async () => 0);
  renderButton(value).unmount();
  expect(cancelLogout).not.toHaveBeenCalled();
});

it("cancels the suspended logout when an unfinished revocation warning is dismissed", async () => {
  const pendingBackupCount = vi.fn(async () => 0);
  const { value, cancelLogout, clearForLogout } = e2ee(pendingBackupCount, async () => true);
  renderButton(value);
  await userEvent.click(screen.getByRole("button", { name: "ログアウト" }));
  await screen.findByText("失効が終わっていません");
  await userEvent.keyboard("{Escape}");
  expect(cancelLogout).toHaveBeenCalledTimes(1);
  expect(pendingBackupCount).not.toHaveBeenCalled();
  expect(clearForLogout).not.toHaveBeenCalled();
});

it("continues to the key count after confirming the unfinished revocation warning", async () => {
  const pendingBackupCount = vi.fn(async () => 2);
  const { value, cancelLogout } = e2ee(pendingBackupCount, async () => true);
  renderButton(value);
  await userEvent.click(screen.getByRole("button", { name: "ログアウト" }));
  await screen.findByText("失効が終わっていません");
  await userEvent.click(screen.getByRole("button", { name: "ログアウトする" }));
  expect(pendingBackupCount).toHaveBeenCalledTimes(1);
  // 確かめてダイアログが閉じても、取りやめとは扱わない。
  expect(cancelLogout).not.toHaveBeenCalled();
});

it("warns before logging out when the unfinished revocation check fails", async () => {
  const pendingBackupCount = vi.fn(async () => 0);
  const { value, cancelLogout, clearForLogout } = e2ee(pendingBackupCount, async () => {
    throw new Error("worker unavailable");
  });
  renderButton(value);
  await userEvent.click(screen.getByRole("button", { name: "ログアウト" }));
  await screen.findByText("失効が終わっていません");
  expect(pendingBackupCount).not.toHaveBeenCalled();
  expect(clearForLogout).not.toHaveBeenCalled();
  await userEvent.keyboard("{Escape}");
  expect(cancelLogout).toHaveBeenCalledTimes(1);
});
