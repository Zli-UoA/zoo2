import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, within } from "storybook/test";

import { playForImplementation } from "../../../.storybook/play-for-implementation";
import {
  DeviceLimitView,
  DevicesSettingsView,
  type DeviceRowView,
  type DevicesSettingsHandlers,
  type DevicesViewState,
} from "./DevicesSettingsView";

const RECOVERY_KEY = "EsTc 5rJ7 kQ2m 8NvP x3Lw 9aHd 4ZfY 6bGs 2CtR 7uWe 1MnK pXqV";
const LOST = "9c3e2b1a-7d6f-4a5b-8c9d-0e1f2a3b4c5d";

const handlers = (): DevicesSettingsHandlers => ({
  onRetry: fn(),
  onRename: fn(),
  onDelete: fn(),
  onStartRevocation: fn(),
  onSelectionChange: fn(),
  onScopeChange: fn(),
  onConfirmSelection: fn(),
  onResumeRevocation: fn(),
  onUnlockWithPasskey: fn(),
  onUnlockWithRecoveryKey: fn(),
  onAddRevocationPasskey: fn(),
  onExecute: fn(),
  onRestart: fn(),
  onCancelRevocation: fn(),
  onFinish: fn(),
  onReverify: fn(),
});

const devices: DeviceRowView[] = [
  {
    deviceId: "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718",
    displayName: "Firefox（Linux）",
    createdAt: "2026-10-20T09:00:00Z",
    lastSeenAt: "2026-11-13T08:00:00Z",
    current: true,
    signed: true,
  },
  {
    deviceId: "5b7d1a8a-1c2e-4e9f-a1b2-c3d4e5f60718",
    displayName: "Pixel 9",
    createdAt: "2026-11-01T09:00:00Z",
    lastSeenAt: "2026-11-12T22:10:00Z",
    current: false,
    signed: true,
  },
  {
    deviceId: LOST,
    displayName: "Pixel 9",
    createdAt: "2026-11-13T03:00:00Z",
    lastSeenAt: "2026-11-13T03:05:00Z",
    current: false,
    signed: false,
  },
];

const ready: Extract<DevicesViewState, { step: "ready" }> = {
  step: "ready",
  devices,
  verifications: [
    {
      userId: "00000000-0000-4000-8000-0000000000b1",
      name: "伊藤さくら",
      state: { state: "needs_reverification", reason: "master_key_changed" },
    },
    {
      userId: "00000000-0000-4000-8000-0000000000b2",
      name: "佐藤ひろし",
      state: { state: "trusted", basis: "user_signing_signature" },
    },
    {
      userId: "00000000-0000-4000-8000-0000000000b3",
      name: "山田はな",
      state: { state: "trusted", basis: "record" },
    },
    {
      userId: "00000000-0000-4000-8000-0000000000b4",
      name: "鈴木けん",
      state: { state: "no_cross_signing_keys" },
    },
  ],
  warnings: [],
  unlocked: true,
  pendingRevocation: null,
  accountUrls: {
    security: "https://account.zli.works/settings/security",
    services: "https://account.zli.works/settings/services",
  },
  revocation: null,
  passkeyAvailable: true,
  busy: null,
  error: null,
};

const notes = [
  {
    userId: "00000000-0000-4000-8000-0000000000b4",
    name: "鈴木けん",
    notes: ["no_record" as const],
  },
  {
    userId: "00000000-0000-4000-8000-0000000000b1",
    name: "伊藤さくら",
    notes: ["pin_replaced" as const, "signature_unverified" as const],
  },
];

const meta = {
  title: "Features/E2EE/DevicesSettings",
  component: DevicesSettingsView,
  parameters: { a11y: { test: "error" } },
  args: { state: { step: "loading" }, handlers: handlers() },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DevicesSettingsView>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loading: Story = {};

export const LoadFailed: Story = {
  args: { state: { step: "loadFailed" }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "もう一度試す" }));
    await expect(args.handlers.onRetry).toHaveBeenCalled();
  }),
};

/** 端末一覧と、他のユーザーの検証状態。削除は失効ではないと表示する。 */
export const Ready: Story = {
  args: { state: ready, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("未署名")).toBeInTheDocument();
    await userEvent.click(canvas.getAllByRole("button", { name: "端末を削除" })[1]);
    await userEvent.click(await screen.findByRole("button", { name: "削除だけ行う" }));
    await expect(args.handlers.onDelete).toHaveBeenCalledWith(LOST);
    await userEvent.click(canvas.getByRole("button", { name: "失効を始める" }));
    await expect(args.handlers.onStartRevocation).toHaveBeenCalledWith([]);
  }),
};

/** 鍵が変わった相手を、再検証する（確認のダイアログを経る）。 */
export const Reverify: Story = {
  args: { state: ready, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "再検証する" }));
    await userEvent.click(await screen.findByRole("button", { name: "再検証する" }));
    await expect(args.handlers.onReverify).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-0000000000b1",
    );
  }),
};

export const Rename: Story = {
  args: { state: ready, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getAllByRole("button", { name: "端末名を変える" })[0]);
    const input = canvas.getByRole("textbox", { name: "端末名" });
    await userEvent.clear(input);
    await userEvent.type(input, "仕事用");
    await userEvent.click(canvas.getByRole("button", { name: "保存" }));
    await expect(args.handlers.onRename).toHaveBeenCalledWith(devices[0].deviceId, "仕事用");
  }),
};

/** 自分のユーザー署名鍵が、一度置き換わった鍵に戻った（サーバーの巻き戻し）。配らずに警告する。 */
export const UserSigningKeyRollback: Story = {
  args: { state: { ...ready, warnings: ["user_signing_key_rollback"] } },
};

/** 解錠していない端末は、失効と再検証を行えない。 */
export const ReadyLocked: Story = {
  args: { state: { ...ready, unlocked: false, verifications: [] } },
};

/** 前に始めた失効が終わっていない。 */
export const PendingRevocation: Story = {
  args: {
    state: { ...ready, pendingRevocation: { scope: "device_only", stage: "cross_signing_saved" } },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "失効を続ける" }));
    await expect(args.handlers.onResumeRevocation).toHaveBeenCalled();
  }),
};

/** 削除する端末と範囲を選ぶ。AuthZ で行う手順を確かめるまで進めない。 */
export const SelectDevices: Story = {
  args: {
    state: {
      ...ready,
      revocation: { phase: "select", scope: "device_only", selected: [] },
    },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    const next = canvas.getByRole("button", { name: "次へ" });
    await expect(next).toBeDisabled();
    await userEvent.click(canvas.getAllByRole("checkbox", { name: /Pixel 9/ })[1]);
    await expect(args.handlers.onSelectionChange).toHaveBeenCalledWith([LOST]);
    await userEvent.click(canvas.getByRole("radio", { name: /SSSS の鍵/ }));
    await expect(args.handlers.onScopeChange).toHaveBeenCalledWith("full");
    await userEvent.click(
      canvas.getByRole("checkbox", { name: "AuthZ でセッションとサービス接続を停止した" }),
    );
    await userEvent.click(next);
    await expect(args.handlers.onConfirmSelection).toHaveBeenCalled();
  }),
};

export const Unlock: Story = {
  args: {
    state: { ...ready, revocation: { phase: "unlock", resume: false } },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.type(canvas.getByLabelText("リカバリーキー"), RECOVERY_KEY);
    await userEvent.click(canvas.getByRole("button", { name: "リカバリーキーで解錠" }));
    await expect(args.handlers.onUnlockWithRecoveryKey).toHaveBeenCalledWith(RECOVERY_KEY);
  }),
};

export const UnlockResume: Story = {
  args: { state: { ...ready, revocation: { phase: "unlock", resume: true } } },
};

export const WrongRecoveryKey: Story = {
  args: {
    state: {
      ...ready,
      revocation: { phase: "unlock", resume: false },
      error: "wrongRecoveryKey",
    },
  },
};

/** 1〜5：新しいリカバリーキーを控えるまで実行できない。 */
export const FullRecoveryKey: Story = {
  args: {
    state: {
      ...ready,
      revocation: { phase: "recoveryKey", recoveryKey: RECOVERY_KEY, passkeyAdded: false },
    },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    const execute = canvas.getByRole("button", { name: "失効を実行" });
    await expect(execute).toBeDisabled();
    await userEvent.click(canvas.getByRole("checkbox", { name: "リカバリーキーを控えました" }));
    await userEvent.click(execute);
    await expect(args.handlers.onExecute).toHaveBeenCalled();
  }),
};

/** 実行の前に、署名し直さない相手と、新しい版に移せないバックアップを出す。 */
export const Confirm: Story = {
  args: {
    state: {
      ...ready,
      revocation: { phase: "confirm", notes, versionsWithoutKey: ["3"] },
    },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("鈴木けん")).toBeInTheDocument();
    await userEvent.click(canvas.getByRole("button", { name: "失効を実行" }));
    await expect(args.handlers.onExecute).toHaveBeenCalled();
  }),
};

export const ConfirmReauthFailed: Story = {
  args: {
    state: {
      ...ready,
      revocation: { phase: "confirm", notes: [], versionsWithoutKey: [] },
      error: "popupBlocked",
    },
  },
};

export const Running: Story = {
  args: {
    state: {
      ...ready,
      busy: "revocation",
      revocation: { phase: "running", stage: "signatures_saved" },
    },
  },
};

/** 別の端末の失効と入り混じった。やり直す。 */
export const Conflict: Story = {
  args: { state: { ...ready, revocation: { phase: "conflict" } }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "やり直す" }));
    await expect(args.handlers.onRestart).toHaveBeenCalled();
  }),
};

/** 終わった。署名されていない端末と、AuthZ で行う残りの手順を出す。 */
export const Done: Story = {
  args: {
    state: {
      ...ready,
      revocation: {
        phase: "done",
        result: {
          scope: "full",
          notes,
          versionsWithoutKey: [],
          unsignedDevices: [devices[1]],
          droppedSessions: 2,
        },
      },
    },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "閉じる" }));
    await expect(args.handlers.onFinish).toHaveBeenCalled();
  }),
};

/** 20 台の上限で登録できなかったとき、削除する端末を選ぶ。 */
export const DeviceLimit: Story = {
  render: () => (
    <DeviceLimitView
      devices={devices.map((device) => ({ ...device, current: false }))}
      staleIds={[devices[1].deviceId]}
      busy={false}
      done={false}
      error={null}
      onDelete={fn()}
    />
  ),
};
