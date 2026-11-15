import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, within } from "storybook/test";

import {
  E2EE_RECOVERY_KEY as RECOVERY_KEY,
  LOST_DEVICE_ID as LOST,
  REVERIFY_NEW_MASTER_KEY,
  deviceRows as devices,
  devicesHandlers as handlers,
  devicesReady as ready,
} from "../../../.storybook/e2ee-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { DeviceLimitView, DevicesSettingsView } from "./DevicesSettingsView";

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

/**
 * 鍵が変わった相手を、再検証する。確認のダイアログに、新しい鍵の指紋と、前に信頼した鍵から変わった
 * ことを出し、絵文字での検証を勧める。受け入れると、見せた鍵にだけ署名する。
 */
export const Reverify: Story = {
  args: { state: ready, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "再検証する" }));
    await expect(args.handlers.onPrepareReverify).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-0000000000b1",
    );
    const dialog = await screen.findByRole("alertdialog");
    await expect(within(dialog).getByText("nE7w")).toBeInTheDocument();
    await expect(within(dialog).getByText("BnQ")).toBeInTheDocument();
    await expect(within(dialog).getByRole("alert")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "この鍵を信頼する" }));
    await expect(args.handlers.onReverify).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-0000000000b1",
      REVERIFY_NEW_MASTER_KEY,
    );
  }),
};

/** 再検証の確認から、絵文字での検証に切り替える。 */
export const ReverifyWithSas: Story = {
  args: { state: ready, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "再検証する" }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "絵文字で検証" }));
    await expect(args.handlers.onSasVerify).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-0000000000b1",
    );
    await expect(args.handlers.onReverify).not.toHaveBeenCalled();
  }),
};

/** 検証済みでない相手を、絵文字 SAS で検証する（#368。画面はシェルのダイアログに出る）。 */
export const SasVerify: Story = {
  args: { state: ready, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    // 鍵が変わった相手と、最初に見た鍵を信頼している相手に出す。検証済みの相手には出さない。
    const buttons = canvas.getAllByRole("button", { name: "絵文字で検証" });
    await expect(buttons).toHaveLength(2);
    await userEvent.click(buttons[1]);
    await expect(args.handlers.onSasVerify).toHaveBeenCalledWith(
      "00000000-0000-4000-8000-0000000000b3",
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
    // 控えたら、実行の前の確認（署名し直さない相手など）に進む。ここでは実行しない。
    const next = canvas.getByRole("button", { name: "次へ" });
    await expect(next).toBeDisabled();
    await userEvent.click(canvas.getByRole("checkbox", { name: "リカバリーキーを控えました" }));
    await userEvent.click(next);
    await expect(args.handlers.onRecoveryKeySaved).toHaveBeenCalled();
    await expect(args.handlers.onExecute).not.toHaveBeenCalled();
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
