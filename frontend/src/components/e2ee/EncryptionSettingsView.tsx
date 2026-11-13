import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CheckIcon,
  CopyIcon,
  DownloadIcon,
  FingerprintIcon,
  KeyRoundIcon,
  LockIcon,
  ShieldCheckIcon,
  Trash2Icon,
} from "lucide-react";

import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { SettingsSection } from "../ui/control-panel";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { Spinner } from "../ui/spinner";

/** 画面に出す誤り。文言は `e2ee.errors.<code>`。 */
export type EncryptionErrorCode =
  | "loadFailed"
  | "saveFailed"
  | "popupBlocked"
  | "reauthFailed"
  | "reauthTimeout"
  | "setupConflict"
  | "mismatch"
  | "invalidRecoveryKey"
  | "wrongRecoveryKey"
  | "passkeyCancelled"
  | "passkeyNoPrf"
  | "passkeyOriginNotAllowed"
  | "passkeyNoSlot"
  | "lastSlot";

export interface KeySlotView {
  id: string;
  kind: "passkey" | "recovery";
  label: string | null;
  createdAt: string | null;
  lastUsedAt: string | null;
}

export type EncryptionViewState =
  | { step: "loading" }
  /** 暗号ストアがまだ開いていない（端末の登録の前など）。 */
  | { step: "unavailable" }
  | { step: "loadFailed" }
  | { step: "notSetUp"; busy: boolean }
  | {
      step: "recoveryKey";
      recoveryKey: string;
      /** E2EE のリセットか。 */
      reset: boolean;
      passkeyAvailable: boolean;
      passkeyAdded: boolean;
      busy: "passkey" | "saving" | null;
      error: EncryptionErrorCode | null;
    }
  | {
      step: "locked";
      /**
       * `addPasskey` は、解錠済みの端末で、パスキーを足すために SSSS を開け直すとき
       * （ページを読み込み直すと、SSSS 鍵はメモリーから消える）。
       */
      reason: "new" | "resume" | "mismatch" | "addPasskey";
      passkeyAvailable: boolean;
      hasPasskeySlots: boolean;
      busy: "passkey" | "recovery" | null;
      error: EncryptionErrorCode | null;
    }
  /** 解錠したが、続きの保存（初回設定の再開）に再認証が要る。 */
  | { step: "needsReauth"; busy: boolean; error: EncryptionErrorCode | null }
  | {
      step: "unlocked";
      slots: KeySlotView[];
      passkeyAvailable: boolean;
      /** SSSS 鍵がメモリーにあり、スロットを足せる。 */
      canAddPasskey: boolean;
      busy: "passkey" | "remove" | null;
      error: EncryptionErrorCode | null;
    };

export interface EncryptionSettingsHandlers {
  onRetry: () => void;
  onStart: () => void;
  onAddSetupPasskey: () => void;
  /** クリックの中で同期的に呼ぶ（再認証の別ウィンドウを開くため）。 */
  onSave: () => void;
  onUnlockWithPasskey: () => void;
  onUnlockWithRecoveryKey: (recoveryKey: string) => void;
  onContinue: () => void;
  onAddPasskey: () => void;
  /** 解錠済みの端末で、パスキーを足すために SSSS を開け直す画面に移る。 */
  onReunlock: () => void;
  /** SSSS を開け直すのをやめて、解錠済みの画面に戻る。 */
  onCancelReunlock: () => void;
  onRemoveSlot: (slotId: string) => void;
  onReset: () => void;
}

function formatDate(value: string | null, locale: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(date);
}

function ErrorMessage({ code }: { code: EncryptionErrorCode | null }) {
  const { t } = useTranslation();
  if (!code) return null;
  return (
    <p role="alert" className="text-sm text-destructive">
      {t(`e2ee.errors.${code}`)}
    </p>
  );
}

function Busy({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-2 text-sm text-muted-foreground">
      <Spinner label={label} className="[&>svg]:size-4" />
      <span aria-hidden="true">{label}</span>
    </div>
  );
}

/** 設定の「暗号化」タブ（#360）。状態は `EncryptionSettings` が持つ。 */
export function EncryptionSettingsView({
  state,
  handlers,
}: {
  state: EncryptionViewState;
  handlers: EncryptionSettingsHandlers;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <div className="rounded-lg border p-4">
        <p className="flex items-center gap-2 text-sm font-semibold">
          <ShieldCheckIcon aria-hidden="true" className="size-5 text-muted-foreground" />
          {t("e2ee.title")}
        </p>
        <p className="mt-1 text-xs text-muted-foreground">{t("e2ee.description")}</p>
      </div>
      <StepContent state={state} handlers={handlers} />
    </div>
  );
}

function StepContent({
  state,
  handlers,
}: {
  state: EncryptionViewState;
  handlers: EncryptionSettingsHandlers;
}) {
  const { t } = useTranslation();
  switch (state.step) {
    case "loading":
      return <Busy label={t("e2ee.loading")} />;
    case "unavailable":
      return <p className="text-sm text-muted-foreground">{t("e2ee.unavailable")}</p>;
    case "loadFailed":
      return (
        <div className="space-y-2">
          <ErrorMessage code="loadFailed" />
          <Button variant="outline" onClick={handlers.onRetry}>
            {t("e2ee.retry")}
          </Button>
        </div>
      );
    case "notSetUp":
      return (
        <SettingsSection
          title={t("e2ee.notSetUpTitle")}
          description={t("e2ee.notSetUpDescription")}
        >
          <Button disabled={state.busy} onClick={handlers.onStart}>
            {t("e2ee.start")}
          </Button>
        </SettingsSection>
      );
    case "recoveryKey":
      return <RecoveryKeyStep state={state} handlers={handlers} />;
    case "locked":
      return <LockedStep state={state} handlers={handlers} />;
    case "needsReauth":
      return (
        <SettingsSection
          title={t("e2ee.continueTitle")}
          description={t("e2ee.continueDescription")}
        >
          <ErrorMessage code={state.error} />
          {state.busy ? (
            <Busy label={t("e2ee.saving")} />
          ) : (
            <Button onClick={handlers.onContinue}>{t("e2ee.continue")}</Button>
          )}
        </SettingsSection>
      );
    case "unlocked":
      return <UnlockedStep state={state} handlers={handlers} />;
  }
}

function RecoveryKeyStep({
  state,
  handlers,
}: {
  state: Extract<EncryptionViewState, { step: "recoveryKey" }>;
  handlers: EncryptionSettingsHandlers;
}) {
  const { t } = useTranslation();
  const [confirmed, setConfirmed] = useState(false);
  const [copied, setCopied] = useState(false);
  const confirmId = useId();
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(state.recoveryKey);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };
  const download = () => {
    const blob = new Blob([`${state.recoveryKey}\n`], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "zoo-recovery-key.txt";
    anchor.click();
    URL.revokeObjectURL(url);
  };
  return (
    <div className="space-y-4">
      <SettingsSection
        title={t("e2ee.recoveryKeyTitle")}
        description={t("e2ee.recoveryKeyDescription")}
      >
        <p
          aria-label={t("e2ee.recoveryKeyTitle")}
          className="select-all break-words rounded-md bg-muted p-3 font-mono text-sm tracking-wide"
          translate="no"
        >
          {state.recoveryKey}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={() => void copy()}>
            {copied ? <CheckIcon /> : <CopyIcon />}
            {copied ? t("e2ee.copied") : t("e2ee.copy")}
          </Button>
          <Button variant="outline" size="sm" onClick={download}>
            <DownloadIcon />
            {t("e2ee.download")}
          </Button>
        </div>
        <label htmlFor={confirmId} className="flex items-center gap-2 text-sm">
          <input
            id={confirmId}
            type="checkbox"
            className="size-4"
            checked={confirmed}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          {t("e2ee.recoveryKeyConfirm")}
        </label>
      </SettingsSection>
      <SettingsSection title={t("e2ee.passkeyTitle")} description={t("e2ee.passkeyDescription")}>
        {!state.passkeyAvailable ? (
          <p className="text-sm text-muted-foreground">{t("e2ee.passkeyUnavailable")}</p>
        ) : state.passkeyAdded ? (
          <p className="flex items-center gap-2 text-sm">
            <CheckIcon aria-hidden="true" className="size-4" />
            {t("e2ee.passkeyAdded")}
          </p>
        ) : (
          <Button
            variant="outline"
            disabled={state.busy !== null}
            onClick={handlers.onAddSetupPasskey}
          >
            <FingerprintIcon />
            {t("e2ee.addPasskey")}
          </Button>
        )}
      </SettingsSection>
      <SettingsSection>
        <p className="text-xs text-muted-foreground">{t("e2ee.saveDescription")}</p>
        <ErrorMessage code={state.error} />
        {state.busy === "saving" ? (
          <Busy label={t("e2ee.saving")} />
        ) : (
          <Button disabled={!confirmed || state.busy !== null} onClick={handlers.onSave}>
            {state.reset ? t("e2ee.saveReset") : t("e2ee.save")}
          </Button>
        )}
      </SettingsSection>
    </div>
  );
}

function LockedStep({
  state,
  handlers,
}: {
  state: Extract<EncryptionViewState, { step: "locked" }>;
  handlers: EncryptionSettingsHandlers;
}) {
  const { t } = useTranslation();
  const [recoveryKey, setRecoveryKey] = useState("");
  const inputId = useId();
  const busy = state.busy !== null;
  const description =
    state.reason === "addPasskey"
      ? t("e2ee.reunlockToAddDescription")
      : state.reason === "resume"
        ? t("e2ee.resumeDescription")
        : state.reason === "mismatch"
          ? t("e2ee.reunlockDescription")
          : t("e2ee.lockedDescription");
  return (
    <div className="space-y-4">
      <SettingsSection title={t("e2ee.lockedTitle")} description={description}>
        <ErrorMessage code={state.error} />
        {state.passkeyAvailable && state.hasPasskeySlots ? (
          <Button disabled={busy} onClick={handlers.onUnlockWithPasskey}>
            <FingerprintIcon />
            {t("e2ee.unlockWithPasskey")}
          </Button>
        ) : (
          <p className="text-xs text-muted-foreground">{t("e2ee.passkeyUnavailable")}</p>
        )}
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (recoveryKey.trim()) handlers.onUnlockWithRecoveryKey(recoveryKey);
          }}
        >
          <label htmlFor={inputId} className="text-sm font-medium">
            {t("e2ee.recoveryKeyInput")}
          </label>
          <Input
            id={inputId}
            value={recoveryKey}
            autoComplete="off"
            spellCheck={false}
            className="font-mono"
            placeholder={t("e2ee.recoveryKeyPlaceholder")}
            onChange={(event) => setRecoveryKey(event.target.value)}
          />
          <Button type="submit" variant="outline" disabled={busy || !recoveryKey.trim()}>
            <KeyRoundIcon />
            {t("e2ee.unlockWithRecoveryKey")}
          </Button>
        </form>
        {busy ? <Busy label={t("e2ee.unlocking")} /> : null}
      </SettingsSection>
      {state.reason === "addPasskey" ? (
        <Button variant="ghost" disabled={busy} onClick={handlers.onCancelReunlock}>
          {t("common.cancel")}
        </Button>
      ) : (
        <ResetSection onReset={handlers.onReset} disabled={busy} />
      )}
    </div>
  );
}

function UnlockedStep({
  state,
  handlers,
}: {
  state: Extract<EncryptionViewState, { step: "unlocked" }>;
  handlers: EncryptionSettingsHandlers;
}) {
  const { t, i18n } = useTranslation();
  const [removing, setRemoving] = useState<KeySlotView | null>(null);
  const busy = state.busy !== null;
  return (
    <div className="space-y-4">
      <SettingsSection>
        <p className="flex items-center gap-2 text-sm font-medium">
          <LockIcon aria-hidden="true" className="size-4" />
          {t("e2ee.unlockedTitle")}
        </p>
        <p className="text-xs text-muted-foreground">{t("e2ee.unlockedDescription")}</p>
      </SettingsSection>
      <SettingsSection title={t("e2ee.slotsTitle")} description={t("e2ee.slotsDescription")}>
        <ErrorMessage code={state.error} />
        <ul className="divide-y rounded-md border">
          {state.slots.map((slot) => {
            const created = formatDate(slot.createdAt, i18n.language);
            const used = formatDate(slot.lastUsedAt, i18n.language);
            const name =
              slot.kind === "recovery"
                ? t("e2ee.slotRecovery")
                : (slot.label ?? t("e2ee.slotPasskey"));
            return (
              <li key={slot.id} className="flex items-center justify-between gap-3 p-3">
                <div className="flex min-w-0 items-center gap-3">
                  {slot.kind === "recovery" ? (
                    <KeyRoundIcon aria-hidden="true" className="size-4 shrink-0" />
                  ) : (
                    <FingerprintIcon aria-hidden="true" className="size-4 shrink-0" />
                  )}
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{name}</p>
                    <p className="text-xs text-muted-foreground">
                      {[
                        created ? t("e2ee.slotCreated", { date: created }) : null,
                        used ? t("e2ee.slotLastUsed", { date: used }) : t("e2ee.slotNeverUsed"),
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </p>
                  </div>
                </div>
                {slot.kind === "passkey" ? (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("e2ee.removeSlot", { name })}
                    disabled={busy}
                    onClick={() => setRemoving(slot)}
                  >
                    <Trash2Icon />
                  </Button>
                ) : null}
              </li>
            );
          })}
        </ul>
        {state.passkeyAvailable ? (
          state.canAddPasskey ? (
            <Button variant="outline" disabled={busy} onClick={handlers.onAddPasskey}>
              <FingerprintIcon />
              {t("e2ee.addPasskey")}
            </Button>
          ) : (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">{t("e2ee.addPasskeyNeedsUnlock")}</p>
              <Button variant="outline" disabled={busy} onClick={handlers.onReunlock}>
                <FingerprintIcon />
                {t("e2ee.reunlockToAdd")}
              </Button>
            </div>
          )
        ) : null}
        {state.busy === "passkey" || state.busy === "remove" ? (
          <Busy label={t("e2ee.saving")} />
        ) : null}
      </SettingsSection>
      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null);
        }}
        title={t("e2ee.removeSlotConfirmTitle")}
        description={t("e2ee.removeSlotConfirmDescription")}
        confirmLabel={t("e2ee.removeSlotConfirm")}
        onConfirm={() => {
          if (removing) handlers.onRemoveSlot(removing.id);
          setRemoving(null);
        }}
      />
    </div>
  );
}

function ResetSection({ onReset, disabled }: { onReset: () => void; disabled: boolean }) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  return (
    <SettingsSection title={t("e2ee.resetTitle")} description={t("e2ee.resetDescription")}>
      <Button variant="destructive" disabled={disabled} onClick={() => setConfirming(true)}>
        {t("e2ee.reset")}
      </Button>
      <ConfirmActionDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("e2ee.resetConfirmTitle")}
        description={t("e2ee.resetConfirmDescription")}
        confirmLabel={t("e2ee.reset")}
        onConfirm={() => {
          setConfirming(false);
          onReset();
        }}
      />
    </SettingsSection>
  );
}
