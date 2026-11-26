import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen } from "storybook/test";

import {
  deviceRows,
  devicesHandlers,
  devicesReady,
  encryptionHandlers,
  encryptionLocked,
  encryptionUnlocked,
} from "../../../.storybook/e2ee-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { DeviceLimitView, DevicesSettingsView } from "./DevicesSettingsView";
import { EncryptionSettingsView } from "./EncryptionSettingsView";
import { EncryptionTabView } from "./EncryptionTabView";

/** 解錠済み：状態と解錠、鍵、端末、他のユーザーの検証が、この順に並ぶ。 */
const unlockedSections = {
  notReady: null,
  encryption: <EncryptionSettingsView state={encryptionUnlocked} handlers={encryptionHandlers()} />,
  devices: <DevicesSettingsView state={devicesReady} handlers={devicesHandlers()} />,
};

const meta = {
  title: "Features/E2EE/EncryptionTab",
  component: EncryptionTabView,
  parameters: { a11y: { test: "error" } },
  args: unlockedSections,
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof EncryptionTabView>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Unlocked: Story = {
  play: playForImplementation(async () => {
    const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    const order = ["解錠の手段", "端末", "他のユーザーの検証"].map((name) =>
      headings.indexOf(name),
    );
    await expect(order.every((index) => index >= 0)).toBe(true);
    await expect([...order].sort((a, b) => a - b)).toEqual(order);
    await expect(screen.getByText("この端末は解錠済みです")).toBeVisible();
  }),
};

export const UnlockedMobile: Story = {
  ...Unlocked,
  globals: { viewport: { value: "mobile" } },
};

/** 解錠していない端末。端末の一覧は出すが、失効と再検証は解錠してから。 */
export const Locked: Story = {
  args: {
    notReady: null,
    encryption: <EncryptionSettingsView state={encryptionLocked} handlers={encryptionHandlers()} />,
    devices: (
      <DevicesSettingsView
        state={{ ...devicesReady, unlocked: false, verifications: [] }}
        handlers={devicesHandlers()}
      />
    ),
  },
};

/** 暗号ストアを開いて、端末を登録しているところ。理由は 1 回だけ出し、各セクションは出さない。 */
export const NotReadyStarting: Story = {
  args: { notReady: { kind: "starting" } },
  play: playForImplementation(async () => {
    await expect(screen.getAllByText(/暗号化の準備をしています/)).toHaveLength(1);
    await expect(screen.queryByRole("heading", { name: "端末" })).not.toBeInTheDocument();
  }),
};

/** ほかのタブが暗号ストアを開いている。そのタブを閉じると引き継ぐ。 */
export const NotReadyOtherTab: Story = {
  args: { notReady: { kind: "otherTab" } },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/ほかの zoo のタブかウィンドウが/)).toBeVisible();
  }),
};

/** 端末の登録をサーバーに拒否され、試し直している。状態コードと code を出す。 */
export const NotReadyRegistrationFailed: Story = {
  args: {
    notReady: {
      kind: "failed",
      failure: { source: "server", status: 403, code: "E2EE_DISABLED" },
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/403 E2EE_DISABLED/)).toBeVisible();
    await expect(screen.getByText(/自動で試し直します/)).toBeVisible();
  }),
};

/** サーバーにつながらず、試し直している。 */
export const NotReadyNetwork: Story = {
  args: { notReady: { kind: "failed", failure: { source: "network" } } },
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/サーバーにつながらず/)).toBeVisible();
  }),
};

/** 暗号ストアを置けないブラウザー。 */
export const NotReadyUnsupported: Story = {
  args: { notReady: { kind: "unsupported" } },
};

/** 端末が 20 台の上限で登録できない。理由のすぐ下に、削除する端末を選ぶ画面を出す。 */
export const DeviceLimit: Story = {
  args: {
    notReady: { kind: "deviceLimit" },
    deviceLimit: (
      <DeviceLimitView
        devices={deviceRows.map((device) => ({ ...device, current: false }))}
        staleIds={[deviceRows[1].deviceId]}
        busy={false}
        done={false}
        error={null}
        onDelete={fn()}
      />
    ),
  },
  play: playForImplementation(async () => {
    await expect(screen.getByText(/下の端末の一覧で/)).toBeVisible();
    await expect(
      screen.getByRole("heading", { name: "端末の上限（20 台）に達しました" }),
    ).toBeVisible();
  }),
};

export const DeviceLimitMobile: Story = {
  ...DeviceLimit,
  globals: { viewport: { value: "mobile" } },
};
