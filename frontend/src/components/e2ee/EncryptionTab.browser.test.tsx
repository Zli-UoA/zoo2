import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vite-plus/test";

import type { CryptoWorker } from "../../lib/e2ee/cryptoWorker";
import { E2eeContext, type E2eeContextValue } from "../../lib/e2ee/e2ee-context";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { EncryptionTab } from "./EncryptionTab";

// 2 つのセクションは、`EncryptionTab` から渡された値と知らせ先だけを出す部品に置き換える
// （各セクションの中身は、それぞれのストーリーとテストで確かめる）。
vi.mock("./EncryptionSettings", () => ({
  EncryptionSettings: (props: {
    refreshKey: number;
    onUnlocked: () => void;
    onSettled: (settled: boolean) => void;
  }) => (
    <div>
      <output aria-label="encryption-refresh">{props.refreshKey}</output>
      <button onClick={props.onUnlocked}>unlocked</button>
      <button onClick={() => props.onSettled(true)}>encryption-settled</button>
    </div>
  ),
}));
vi.mock("./DevicesSettings", () => ({
  DevicesSettings: (props: {
    refreshKey: number;
    onKeysChanged: () => void;
    onSettled: (settled: boolean) => void;
  }) => (
    <div>
      <output aria-label="devices-refresh">{props.refreshKey}</output>
      <button onClick={props.onKeysChanged}>revoked</button>
      <button onClick={() => props.onSettled(true)}>devices-settled</button>
    </div>
  ),
  DeviceLimit: () => <p>device-limit</p>,
}));

function context(patch: Partial<E2eeContextValue>): E2eeContextValue {
  return {
    status: "ready",
    deviceId: "d1",
    worker: {} as CryptoWorker,
    notReadyReason: null,
    userId: "u1",
    roomKeysVersion: 0,
    pendingBackupCount: async () => 0,
    revocationPending: async () => false,
    deviceLimit: null,
    cancelLogout: () => {},
    requestKeyBackup: () => {},
    requestKeyBackupImport: () => {},
    requestSenderReverify: () => {},
    clearForLogout: async () => {},
    sas: null,
    ...patch,
  };
}

function renderTab(value: E2eeContextValue, props: Parameters<typeof EncryptionTab>[0] = {}) {
  return render(
    <LocaleProvider>
      <E2eeContext.Provider value={value}>
        <EncryptionTab {...props} />
      </E2eeContext.Provider>
    </LocaleProvider>,
  );
}

it("暗号ストアを開けていないときは、理由を 1 回だけ出し、各セクションは出さない", async () => {
  renderTab(
    context({
      worker: null,
      notReadyReason: { kind: "deviceLimit" },
      deviceLimit: [{ device_id: "d9" } as never],
    }),
  );
  await expect.element(screen.getByText(/下の端末の一覧で/)).toBeVisible();
  expect(screen.getByText("device-limit")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "unlocked" })).toBeNull();
  expect(screen.queryByRole("button", { name: "revoked" })).toBeNull();
});

it("上のセクションで解錠したら端末を、失効を終えたら鍵スロットを読み直させる", async () => {
  renderTab(context({}));
  expect(screen.getByLabelText("devices-refresh").textContent).toBe("0");
  await userEvent.click(screen.getByRole("button", { name: "unlocked" }));
  expect(screen.getByLabelText("devices-refresh").textContent).toBe("1");
  expect(screen.getByLabelText("encryption-refresh").textContent).toBe("0");
  await userEvent.click(screen.getByRole("button", { name: "revoked" }));
  expect(screen.getByLabelText("encryption-refresh").textContent).toBe("1");
});

it("端末のセクションへは、両方のセクションを読み込み終えてから移る", async () => {
  const onDevicesFocused = vi.fn();
  renderTab(context({}), { focusDevices: true, onDevicesFocused });
  const frame = () => act(() => new Promise((resolve) => requestAnimationFrame(resolve)));
  await frame();
  expect(onDevicesFocused).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "devices-settled" }));
  await frame();
  expect(onDevicesFocused).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "encryption-settled" }));
  await frame();
  expect(onDevicesFocused).toHaveBeenCalledTimes(1);
});
