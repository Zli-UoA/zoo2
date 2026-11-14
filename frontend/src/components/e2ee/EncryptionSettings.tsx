import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { PasskeyError, passkeyPrfSupported, passkeyRpId } from "../../lib/e2ee/passkey";
import { ReauthError, startReauth } from "../../lib/e2ee/reauth";
import type { SecretStorageResponse } from "../../lib/e2ee/secretStorageApi";
import {
  type E2eeSetupDeps,
  ReauthRequiredError,
  ServerKeyMismatchError,
  SetupConflictError,
  addPasskeySlot,
  addSetupPasskey,
  beginSetup,
  completeSetup,
  hasUnlockedSecrets,
  loadE2eeStatus,
  newRecoveryKey,
  removeKeySlot,
  saveRecoverySlot,
  saveRemaining,
  unlock,
} from "../../lib/e2ee/setupController";
import { accountRevocationUrls } from "../../lib/oidc/accountUrl";
import { deleteSelectedDevices, loadDevices } from "../../lib/e2ee/revocationController";
import type { KeySlot } from "../../lib/e2ee/protocol";
import {
  type EncryptionErrorCode,
  type EncryptionSettingsHandlers,
  type EncryptionViewState,
  EncryptionSettingsView,
  type KeySlotView,
} from "./EncryptionSettingsView";

/** 失敗を、画面に出す誤りに変える。 */
export function toEncryptionError(
  error: unknown,
  fallback: EncryptionErrorCode,
): EncryptionErrorCode {
  if (error instanceof SetupConflictError) return "setupConflict";
  if (error instanceof ServerKeyMismatchError) return "mismatch";
  if (error instanceof ReauthError) {
    if (error.code === "popupBlocked") return "popupBlocked";
    if (error.code === "timeout") return "reauthTimeout";
    return "reauthFailed";
  }
  if (error instanceof ReauthRequiredError) return "reauthFailed";
  if (error instanceof PasskeyError) {
    switch (error.code) {
      case "cancelled":
        return "passkeyCancelled";
      case "prfUnsupported":
        return "passkeyNoPrf";
      case "originNotAllowed":
        return "passkeyOriginNotAllowed";
      case "noSlot":
        return "passkeyNoSlot";
      default:
        return fallback;
    }
  }
  const code = (error as { code?: unknown } | null)?.code;
  if (code === "InvalidRecoveryKey") return "invalidRecoveryKey";
  if (code === "KeySlotUnlock" || code === "NoKeySlot" || code === "SsssKeyMismatch") {
    return "wrongRecoveryKey";
  }
  if (code === "ServerKeyMismatch") return "mismatch";
  if ((error as { status?: unknown } | null)?.status === 400) {
    const body = (error as { body?: { code?: unknown } }).body;
    if (body?.code === "E2EE_SLOTS_REQUIRED") return "lastSlot";
  }
  return fallback;
}

function slotViews(storage: SecretStorageResponse): KeySlotView[] {
  return storage.slots.map((slot) => ({
    id: slot.id,
    kind: slot.kind,
    label: slot.label ?? null,
    createdAt: slot.created_at ?? null,
    lastUsedAt: slot.last_used_at ?? null,
  }));
}

/** 設定の「暗号化」タブ（初回設定と解錠。#360）。 */
export function EncryptionSettings() {
  const { worker, userId, deviceId, requestKeyBackupImport } = useE2ee();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const rpId = passkeyRpId(config);
  const [prfSupported, setPrfSupported] = useState(false);
  const [state, setState] = useState<EncryptionViewState>({ step: "loading" });
  const storageRef = useRef<SecretStorageResponse | null>(null);
  const keyIdRef = useRef<string | null>(null);
  const resetRef = useRef(false);
  const revisionRef = useRef<number | null>(null);
  /** 作り直したリカバリーキーのスロット（保存するまでサーバーにない）。 */
  const recoverySlotRef = useRef<KeySlot | null>(null);
  /**
   * 状態の取得の番号。取得を始めるときと、利用者が初回設定を始めたときに進め、古い取得の
   * 結果で今の画面を上書きしないようにする。
   */
  const loadSeq = useRef(0);

  const deps: E2eeSetupDeps | null = useMemo(
    () =>
      worker && userId
        ? { worker, client, userId, rpId, onUnlocked: requestKeyBackupImport }
        : null,
    [worker, userId, client, rpId, requestKeyBackupImport],
  );
  const passkeyAvailable = rpId !== null && prfSupported;

  useEffect(() => {
    void passkeyPrfSupported().then(setPrfSupported);
  }, []);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    const latest = () => seq === loadSeq.current;
    if (!deps) {
      setState({ step: "unavailable" });
      return;
    }
    setState({ step: "loading" });
    try {
      const status = await loadE2eeStatus(deps);
      if (!latest()) return;
      if (status.kind === "notSetUp") {
        storageRef.current = null;
        setState({ step: "notSetUp", busy: false });
      } else if (status.kind === "locked") {
        storageRef.current = status.storage;
        setState({
          step: "locked",
          reason: status.reason,
          passkeyAvailable: false,
          hasPasskeySlots: status.storage.slots.some((slot) => slot.kind === "passkey"),
          busy: null,
          error: null,
        });
      } else {
        storageRef.current = status.storage;
        const canAddPasskey = await hasUnlockedSecrets(deps);
        if (!latest()) return;
        setState({
          step: "unlocked",
          slots: slotViews(status.storage),
          passkeyAvailable: false,
          canAddPasskey,
          busy: null,
          error: null,
        });
      }
    } catch {
      if (latest()) setState({ step: "loadFailed" });
    }
  }, [deps]);

  useEffect(() => {
    void load();
  }, [load]);

  const begin = useCallback(
    async (resetFrom: SecretStorageResponse | null) => {
      if (!deps) return;
      // 進行中の状態の取得は、終わっても画面を変えない。
      loadSeq.current += 1;
      try {
        const { recoveryKey, keyId } = await beginSetup(deps, resetFrom);
        keyIdRef.current = keyId;
        resetRef.current = resetFrom !== null;
        // リセットでは、見覚えのない端末を選んで削除してもらう（docs/e2ee.md の【決定】）。
        const resetDevices =
          resetFrom !== null
            ? (await loadDevices(deps, deviceId)).filter((device) => !device.current)
            : null;
        setState({
          step: "recoveryKey",
          recoveryKey,
          reset: resetFrom !== null,
          passkeyAvailable: false,
          passkeyAdded: false,
          busy: null,
          error: null,
          resetDevices,
          accountUrls: config.oidc ? accountRevocationUrls(config.oidc.issuer) : null,
        });
      } catch {
        setState({ step: "loadFailed" });
      }
    },
    [deps, deviceId, config.oidc],
  );

  const update = (patch: (current: EncryptionViewState) => EncryptionViewState) =>
    setState((current) => patch(current));

  const handlers: EncryptionSettingsHandlers = {
    onRetry: () => void load(),
    onStart: () => {
      setState({ step: "notSetUp", busy: true });
      void begin(null);
    },
    onAddSetupPasskey: () => {
      if (!deps) return;
      update((current) =>
        current.step === "recoveryKey" ? { ...current, busy: "passkey", error: null } : current,
      );
      void addSetupPasskey(deps, null).then(
        () =>
          update((current) =>
            current.step === "recoveryKey"
              ? { ...current, busy: null, passkeyAdded: true }
              : current,
          ),
        (error) =>
          update((current) =>
            current.step === "recoveryKey"
              ? { ...current, busy: null, error: toEncryptionError(error, "saveFailed") }
              : current,
          ),
      );
    },
    onSave: (deleteDevices) => {
      if (!deps || !keyIdRef.current) return;
      const keyId = keyIdRef.current;
      const reauth = startReauth(config);
      update((current) =>
        current.step === "recoveryKey" ? { ...current, busy: "saving", error: null } : current,
      );
      void (async () => {
        try {
          const token = await reauth();
          if (resetRef.current) await deleteSelectedDevices(deps, deleteDevices);
          await completeSetup(deps, keyId, token);
          keyIdRef.current = null;
          await load();
        } catch (error) {
          const code = toEncryptionError(error, "saveFailed");
          if (code === "setupConflict" || code === "mismatch") {
            // 手元の鍵は捨てた。表示を読み直し、誤りを残す。
            await load();
            update((current) =>
              current.step === "locked" ? { ...current, error: code } : current,
            );
            return;
          }
          update((current) =>
            current.step === "recoveryKey" ? { ...current, busy: null, error: code } : current,
          );
        }
      })();
    },
    onUnlockWithPasskey: () => runUnlock({ kind: "passkey" }),
    onUnlockWithRecoveryKey: (recoveryKey) => runUnlock({ kind: "recovery", recoveryKey }),
    onContinue: () => {
      if (!deps) return;
      const reauth = startReauth(config);
      setState({ step: "needsReauth", busy: true, error: null });
      void (async () => {
        try {
          const keyId = keyIdRef.current ?? undefined;
          await saveRemaining(deps, revisionRef.current, await reauth(), keyId);
          await load();
        } catch (error) {
          const code = toEncryptionError(error, "saveFailed");
          if (code === "mismatch" || code === "setupConflict") {
            // 手元の鍵は捨てた（サーバーの鍵が変わったか、別の端末が SSSS を作り直した）。読み直して、
            // 今の SSSS を解錠し直すかリセットを選べるようにする。
            await load();
            update((current) =>
              current.step === "locked" ? { ...current, error: code } : current,
            );
            return;
          }
          setState({ step: "needsReauth", busy: false, error: code });
        }
      })();
    },
    onAddPasskey: () => {
      const storage = storageRef.current;
      if (!deps || !storage) return;
      // 同じパスキーのスロットの置き換えには再認証が要るので、先に開いておく。
      const reauth = startReauth(config);
      update((current) =>
        current.step === "unlocked" ? { ...current, busy: "passkey", error: null } : current,
      );
      void (async () => {
        try {
          await addPasskeySlot(deps, storage, null, await reauth());
          await load();
        } catch (error) {
          update((current) =>
            current.step === "unlocked"
              ? { ...current, busy: null, error: toEncryptionError(error, "saveFailed") }
              : current,
          );
        }
      })();
    },
    onRemoveSlot: (slotId) => {
      if (!deps) return;
      const reauth = startReauth(config);
      update((current) =>
        current.step === "unlocked" ? { ...current, busy: "remove", error: null } : current,
      );
      void (async () => {
        try {
          await removeKeySlot(deps, slotId, await reauth());
          await load();
        } catch (error) {
          update((current) =>
            current.step === "unlocked"
              ? { ...current, busy: null, error: toEncryptionError(error, "saveFailed") }
              : current,
          );
        }
      })();
    },
    onReunlock: () => {
      const storage = storageRef.current;
      if (!storage) return;
      loadSeq.current += 1;
      setState({
        step: "locked",
        reason: "addPasskey",
        passkeyAvailable: false,
        hasPasskeySlots: storage.slots.some((slot) => slot.kind === "passkey"),
        busy: null,
        error: null,
      });
    },
    onCancelReunlock: () => void load(),
    onRegenerateRecoveryKey: () => {
      if (!deps) return;
      loadSeq.current += 1;
      void newRecoveryKey(deps).then(
        ({ slot, recoveryKey }) => {
          recoverySlotRef.current = slot;
          setState({ step: "newRecoveryKey", recoveryKey, busy: false, error: null });
        },
        (error) =>
          update((current) =>
            current.step === "unlocked"
              ? { ...current, error: toEncryptionError(error, "saveFailed") }
              : current,
          ),
      );
    },
    onSaveRecoveryKey: () => {
      const storage = storageRef.current;
      const slot = recoverySlotRef.current;
      if (!deps || !storage || !slot) return;
      const reauth = startReauth(config);
      update((current) =>
        current.step === "newRecoveryKey" ? { ...current, busy: true, error: null } : current,
      );
      void (async () => {
        try {
          await saveRecoverySlot(deps, storage, slot, await reauth());
          recoverySlotRef.current = null;
          await load();
        } catch (error) {
          update((current) =>
            current.step === "newRecoveryKey"
              ? { ...current, busy: false, error: toEncryptionError(error, "saveFailed") }
              : current,
          );
        }
      })();
    },
    onCancelRecoveryKey: () => {
      recoverySlotRef.current = null;
      void load();
    },
    onReset: () => {
      const storage = storageRef.current;
      if (!storage) return;
      void begin(storage);
    },
  };

  function runUnlock(method: { kind: "passkey" } | { kind: "recovery"; recoveryKey: string }) {
    const storage = storageRef.current;
    if (!deps || !storage) return;
    update((current) =>
      current.step === "locked"
        ? { ...current, busy: method.kind === "passkey" ? "passkey" : "recovery", error: null }
        : current,
    );
    void (async () => {
      try {
        const result = await unlock(deps, storage, method);
        revisionRef.current = result.secretsRevision;
        // 再認証のあとの続きの保存も、開けた SSSS 鍵を前提にする。
        keyIdRef.current = result.keyId;
        if (result.needsReauth) {
          setState({ step: "needsReauth", busy: false, error: null });
        } else {
          await load();
        }
      } catch (error) {
        update((current) =>
          current.step === "locked"
            ? { ...current, busy: null, error: toEncryptionError(error, "saveFailed") }
            : current,
        );
      }
    })();
  }

  // PRF に対応するかは非同期に分かる。状態を取り直さずに、表示のときに反映する。
  const viewState = "passkeyAvailable" in state ? { ...state, passkeyAvailable } : state;
  return <EncryptionSettingsView state={viewState} handlers={handlers} />;
}
