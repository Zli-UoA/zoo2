import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, within } from "storybook/test";

import { playForImplementation } from "../../../.storybook/play-for-implementation";
import {
  EncryptionSettingsView,
  type EncryptionSettingsHandlers,
  type EncryptionViewState,
} from "./EncryptionSettingsView";

const RECOVERY_KEY = "EsTc 5rJ7 kQ2m 8NvP x3Lw 9aHd 4ZfY 6bGs 2CtR 7uWe 1MnK pXqV";

const handlers = (): EncryptionSettingsHandlers => ({
  onRetry: fn(),
  onStart: fn(),
  onAddSetupPasskey: fn(),
  onSave: fn(),
  onUnlockWithPasskey: fn(),
  onUnlockWithRecoveryKey: fn(),
  onContinue: fn(),
  onAddPasskey: fn(),
  onReunlock: fn(),
  onCancelReunlock: fn(),
  onRemoveSlot: fn(),
  onReset: fn(),
  onRegenerateRecoveryKey: fn(),
  onSaveRecoveryKey: fn(),
  onCancelRecoveryKey: fn(),
});

const recoveryKey: EncryptionViewState = {
  step: "recoveryKey",
  recoveryKey: RECOVERY_KEY,
  reset: false,
  passkeyAvailable: true,
  passkeyAdded: false,
  busy: null,
  error: null,
};

const locked: EncryptionViewState = {
  step: "locked",
  reason: "new",
  passkeyAvailable: true,
  hasPasskeySlots: true,
  busy: null,
  error: null,
};

const unlocked: EncryptionViewState = {
  step: "unlocked",
  passkeyAvailable: true,
  canAddPasskey: true,
  busy: null,
  error: null,
  slots: [
    {
      id: "slot-recovery",
      kind: "recovery",
      label: null,
      createdAt: "2026-11-13T09:00:00Z",
      lastUsedAt: null,
    },
    {
      id: "slot-passkey",
      kind: "passkey",
      label: "MacBook の iCloud キーチェーン",
      createdAt: "2026-11-13T09:00:00Z",
      lastUsedAt: "2026-11-14T08:30:00Z",
    },
  ],
};

const meta = {
  title: "Features/E2EE/EncryptionSettings",
  component: EncryptionSettingsView,
  parameters: { a11y: { test: "error" } },
  args: { state: { step: "loading" }, handlers: handlers() },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof EncryptionSettingsView>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loading: Story = {};

export const Unavailable: Story = { args: { state: { step: "unavailable" } } };

export const LoadFailed: Story = {
  args: { state: { step: "loadFailed" }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "もう一度試す" }));
    await expect(args.handlers.onRetry).toHaveBeenCalled();
  }),
};

export const NotSetUp: Story = {
  args: { state: { step: "notSetUp", busy: false }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "設定を始める" }));
    await expect(args.handlers.onStart).toHaveBeenCalled();
  }),
};

/** リカバリーキーを控えたと確かめるまで、保存できない。 */
export const RecoveryKey: Story = {
  args: { state: recoveryKey, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByLabelText("リカバリーキー")).toHaveTextContent(RECOVERY_KEY);
    const save = canvas.getByRole("button", { name: "保存する" });
    await expect(save).toBeDisabled();
    await userEvent.click(canvas.getByRole("checkbox", { name: "リカバリーキーを控えました" }));
    await expect(save).toBeEnabled();
    await userEvent.click(save);
    await expect(args.handlers.onSave).toHaveBeenCalled();
  }),
};

export const RecoveryKeyPasskeyAdded: Story = {
  args: { state: { ...recoveryKey, passkeyAdded: true } },
};

export const RecoveryKeyWithoutPasskey: Story = {
  args: { state: { ...recoveryKey, passkeyAvailable: false } },
};

export const Saving: Story = { args: { state: { ...recoveryKey, busy: "saving" } } };

export const PopupBlocked: Story = {
  args: { state: { ...recoveryKey, error: "popupBlocked" } },
};

export const SetupConflict: Story = { args: { state: { ...locked, error: "setupConflict" } } };

export const ResetRecoveryKey: Story = { args: { state: { ...recoveryKey, reset: true } } };

/**
 * リセットでは、見覚えのない端末を選んで削除してもらう。選択を確かめるまで保存できない（#362）。
 */
export const ResetChooseDevices: Story = {
  args: {
    state: {
      ...recoveryKey,
      reset: true,
      accountUrls: {
        security: "https://account.zli.works/settings/security",
        services: "https://account.zli.works/settings/services",
      },
      resetDevices: [
        {
          deviceId: "5b7d1a8a-1c2e-4e9f-a1b2-c3d4e5f60718",
          displayName: "Chrome（macOS）",
          createdAt: "2026-11-01T09:00:00Z",
          lastSeenAt: "2026-11-12T22:10:00Z",
          current: false,
          signed: true,
        },
        {
          deviceId: "9c3e2b1a-7d6f-4a5b-8c9d-0e1f2a3b4c5d",
          displayName: "Chrome（macOS）",
          createdAt: "2026-11-13T03:00:00Z",
          lastSeenAt: "2026-11-13T03:05:00Z",
          current: false,
          signed: false,
        },
      ],
    },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("checkbox", { name: "リカバリーキーを控えました" }));
    const save = canvas.getByRole("button", { name: "リセットして保存する" });
    await expect(save).toBeDisabled();
    const devices = canvas.getAllByRole("checkbox", { name: /Chrome/ });
    await userEvent.click(devices[1]);
    await userEvent.click(canvas.getByRole("checkbox", { name: /見覚えのない端末を選んだ/ }));
    await userEvent.click(save);
    await expect(args.handlers.onSave).toHaveBeenCalledWith([
      "9c3e2b1a-7d6f-4a5b-8c9d-0e1f2a3b4c5d",
    ]);
  }),
};

/** 解錠済みの端末で、リカバリーキーを作り直す。控えたと確かめるまで保存できない。 */
export const NewRecoveryKey: Story = {
  args: {
    state: { step: "newRecoveryKey", recoveryKey: RECOVERY_KEY, busy: false, error: null },
    handlers: handlers(),
  },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    const save = canvas.getByRole("button", { name: "保存する" });
    await expect(save).toBeDisabled();
    await userEvent.click(canvas.getByRole("checkbox", { name: "リカバリーキーを控えました" }));
    await userEvent.click(save);
    await expect(args.handlers.onSaveRecoveryKey).toHaveBeenCalled();
  }),
};

export const Locked: Story = {
  args: { state: locked, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.type(canvas.getByLabelText("リカバリーキー"), RECOVERY_KEY);
    await userEvent.click(canvas.getByRole("button", { name: "リカバリーキーで解錠" }));
    await expect(args.handlers.onUnlockWithRecoveryKey).toHaveBeenCalledWith(RECOVERY_KEY);
    await userEvent.click(canvas.getByRole("button", { name: "パスキーで解錠" }));
    await expect(args.handlers.onUnlockWithPasskey).toHaveBeenCalled();
  }),
};

/** Preview や TUI など、パスキーを使えない環境ではリカバリーキーだけを案内する。 */
export const LockedRecoveryOnly: Story = {
  args: { state: { ...locked, passkeyAvailable: false } },
};

export const LockedResume: Story = { args: { state: { ...locked, reason: "resume" } } };

export const LockedMismatch: Story = {
  args: { state: { ...locked, reason: "mismatch", error: "mismatch" } },
};

export const WrongRecoveryKey: Story = {
  args: { state: { ...locked, error: "invalidRecoveryKey" } },
};

export const PasskeyWithoutPrf: Story = { args: { state: { ...locked, error: "passkeyNoPrf" } } };

export const Unlocking: Story = { args: { state: { ...locked, busy: "recovery" } } };

export const NeedsReauth: Story = {
  args: { state: { step: "needsReauth", busy: false, error: null }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "ログインして続ける" }));
    await expect(args.handlers.onContinue).toHaveBeenCalled();
  }),
};

export const Unlocked: Story = {
  args: { state: unlocked, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      canvas.getByRole("button", { name: "MacBook の iCloud キーチェーン を削除" }),
    );
    await userEvent.click(await screen.findByRole("button", { name: "削除する" }));
    await expect(args.handlers.onRemoveSlot).toHaveBeenCalledWith("slot-passkey");
  }),
};

/** リカバリーキーのスロットも作り直しと削除ができる（#362）。 */
export const UnlockedRegenerateRecoveryKey: Story = {
  args: { state: unlocked, handlers: handlers() },
  play: playForImplementation(async ({ args, canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "リカバリーキーを作り直す" }));
    await expect(args.handlers.onRegenerateRecoveryKey).toHaveBeenCalled();
  }),
};

/** 読み込み直したあと：SSSS 鍵がメモリーにないので、開け直してから足す。 */
export const UnlockedNeedsUnlockToAdd: Story = {
  args: { state: { ...unlocked, canAddPasskey: false }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await userEvent.click(screen.getByRole("button", { name: "解錠し直してパスキーを追加" }));
    await expect(args.handlers.onReunlock).toHaveBeenCalled();
  }),
};

/** パスキーを足すために SSSS を開け直す。リセットは出さず、取り消せる。 */
export const ReunlockToAddPasskey: Story = {
  args: { state: { ...locked, reason: "addPasskey" }, handlers: handlers() },
  play: playForImplementation(async ({ args }) => {
    await expect(screen.queryByRole("button", { name: "リセットする" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "キャンセル" }));
    await expect(args.handlers.onCancelReunlock).toHaveBeenCalled();
  }),
};

export const LastSlot: Story = { args: { state: { ...unlocked, error: "lastSlot" } } };

export const Mobile: Story = {
  args: { state: recoveryKey },
  globals: { viewport: { value: "mobile" } },
};
