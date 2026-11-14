import { fn } from "storybook/test";

import type {
  DeviceRowView,
  DevicesSettingsHandlers,
  DevicesViewState,
} from "../src/components/e2ee/DevicesSettingsView";
import type {
  EncryptionSettingsHandlers,
  EncryptionViewState,
} from "../src/components/e2ee/EncryptionSettingsView";

/** E2EE の設定（暗号化タブ）のストーリーで共有する値。 */
export const E2EE_RECOVERY_KEY = "EsTc 5rJ7 kQ2m 8NvP x3Lw 9aHd 4ZfY 6bGs 2CtR 7uWe 1MnK pXqV";
export const LOST_DEVICE_ID = "9c3e2b1a-7d6f-4a5b-8c9d-0e1f2a3b4c5d";

export const devicesHandlers = (): DevicesSettingsHandlers => ({
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
  onRecoveryKeySaved: fn(),
  onExecute: fn(),
  onRestart: fn(),
  onCancelRevocation: fn(),
  onFinish: fn(),
  onReverify: fn(),
  onSasVerify: fn(),
});

export const deviceRows: DeviceRowView[] = [
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
    deviceId: LOST_DEVICE_ID,
    displayName: "Pixel 9",
    createdAt: "2026-11-13T03:00:00Z",
    lastSeenAt: "2026-11-13T03:05:00Z",
    current: false,
    signed: false,
  },
];

export const devicesReady: Extract<DevicesViewState, { step: "ready" }> = {
  step: "ready",
  devices: deviceRows,
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

export const encryptionHandlers = (): EncryptionSettingsHandlers => ({
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

export const encryptionRecoveryKey: Extract<EncryptionViewState, { step: "recoveryKey" }> = {
  step: "recoveryKey",
  recoveryKey: E2EE_RECOVERY_KEY,
  reset: false,
  passkeyAvailable: true,
  passkeyAdded: false,
  busy: null,
  error: null,
};

export const encryptionLocked: Extract<EncryptionViewState, { step: "locked" }> = {
  step: "locked",
  reason: "new",
  passkeyAvailable: true,
  hasPasskeySlots: true,
  busy: null,
  error: null,
};

export const encryptionUnlocked: Extract<EncryptionViewState, { step: "unlocked" }> = {
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
