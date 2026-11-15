import { useId, useState } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import {
  CheckIcon,
  CopyIcon,
  ExternalLinkIcon,
  FingerprintIcon,
  KeyRoundIcon,
  LoaderCircleIcon,
  MonitorSmartphoneIcon,
  PencilIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
  ShieldIcon,
  Trash2Icon,
} from "lucide-react";
import type {
  ResignNote,
  RevocationScope,
  RevocationStage,
  TrustWarning,
  UserState,
} from "@zoo/crypto-wasm";

import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { SettingsSection } from "../ui/control-panel";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "../ui/alert-dialog";
import { formatKeyFingerprint } from "../../lib/e2ee/masterKey";
import { Spinner } from "../ui/spinner";
import type { E2eeNotReadyReason } from "../../lib/e2ee/notReady";
import type { EncryptionErrorCode } from "./EncryptionSettingsView";

/** 端末一覧の 1 行。 */
export interface DeviceRowView {
  deviceId: string;
  displayName: string | null;
  createdAt: string;
  lastSeenAt: string | null;
  current: boolean;
  signed: boolean;
}

/**
 * 再検証の確認の画面に出す鍵（`previewReverify`）。署名するのは `masterKey` だけで、確認のあとに
 * サーバーの鍵が変わっていたら署名しない。
 */
export interface ReverifyPreviewView {
  /** 相手の今のマスター鍵の公開鍵。 */
  masterKey: string;
  /** この端末が前に信頼した鍵（記録）。記録がなければ `null`。 */
  previousMasterKey: string | null;
}

/** 他のユーザーの検証状態の 1 行。 */
export interface VerificationRowView {
  userId: string;
  name: string;
  state: UserState;
}

/** 失効の画面に出す、署名し直さなかった相手。 */
export interface ResignNoteView {
  userId: string;
  name: string;
  notes: ResignNote[];
}

/** 失効の結果。 */
export interface RevocationResultView {
  scope: RevocationScope;
  notes: ResignNoteView[];
  versionsWithoutKey: string[];
  unsignedDevices: DeviceRowView[];
  droppedSessions: number;
}

/** 失効の画面の段。 */
export type RevocationPhase =
  /** 削除する端末と範囲を選ぶ。AuthZ で行う手順を案内する。 */
  | { phase: "select"; scope: RevocationScope; selected: string[] }
  /** 始める前に SSSS を開け直す。`resume` は止まった失効の続き。 */
  | { phase: "unlock"; resume: boolean }
  /** 2〜5：新しいリカバリーキーを控え、パスキーのスロットを作る。 */
  | { phase: "recoveryKey"; recoveryKey: string; passkeyAdded: boolean }
  /** 実行の前の確認（署名し直さない相手の一覧）。 */
  | { phase: "confirm"; notes: ResignNoteView[]; versionsWithoutKey: string[] }
  /** 保存中。 */
  | { phase: "running"; stage: RevocationStage }
  /** 終わった。AuthZ で行う残りの手順を案内する。 */
  | { phase: "done"; result: RevocationResultView }
  /** 別の端末の失効と入り混じった。SSSS を開け直してやり直す。 */
  | { phase: "conflict" };

export type DevicesErrorCode =
  | EncryptionErrorCode
  | "deleteFailed"
  /** 操作のあとで一覧を読み直せなかった（表示は残し、読み直しを出す）。 */
  | "refreshFailed"
  /** 終わっていない失効があるので、新しい失効を始めなかった。 */
  | "revocationPending"
  /** 別の端末が SSSS の鍵を作り直したので、端末だけの失効は行えない（範囲を全体に切り替えた）。 */
  | "deviceOnlyUnavailable"
  | "renameFailed"
  | "revocationFailed"
  | "verifyFailed"
  /** 確認の画面に出した鍵から、相手の鍵が変わった（再検証を止めた）。 */
  | "reverifyKeyChanged"
  /** 絵文字 SAS の検証を始められなかった（#368）。 */
  | "sasFailed";

export type DevicesViewState =
  | { step: "loading" }
  | { step: "loadFailed" }
  | {
      step: "ready";
      devices: DeviceRowView[];
      verifications: VerificationRowView[];
      /**
       * 配布先の判定の警告（自分のユーザー署名鍵の巻き戻し、自分のマスター鍵の変更など）。
       * 一度置き換わったユーザー署名鍵に戻ったときは、他のユーザーに鍵を配らない（#362）。
       */
      warnings: TrustWarning[];
      /** この端末が解錠済みか（失効と再検証は解錠済みの端末だけ）。 */
      unlocked: boolean;
      /**
       * 自分のマスター鍵の公開鍵（解錠して信頼したもの）。相手が再検証で比べる指紋として出す。
       * 未解錠なら `null`。
       */
      ownMasterKey?: string | null;
      /** 終わっていない失効（あれば、続けるよう表示する）。 */
      pendingRevocation: { scope: RevocationScope; stage: RevocationStage } | null;
      /** AuthZ のセッションとサービス接続の画面。OIDC を設定していなければ `null`。 */
      accountUrls: { security: string; services: string } | null;
      revocation: RevocationPhase | null;
      passkeyAvailable: boolean;
      busy: "rename" | "delete" | "revocation" | "verify" | null;
      error: DevicesErrorCode | null;
    };

export interface DevicesSettingsHandlers {
  onRetry: () => void;
  onRename: (deviceId: string, displayName: string) => void;
  onDelete: (deviceId: string) => void;
  onStartRevocation: (selected: string[]) => void;
  onSelectionChange: (selected: string[]) => void;
  onScopeChange: (scope: RevocationScope) => void;
  onConfirmSelection: () => void;
  onResumeRevocation: () => void;
  onUnlockWithPasskey: () => void;
  onUnlockWithRecoveryKey: (recoveryKey: string) => void;
  onAddRevocationPasskey: () => void;
  /** 1〜5 の新しいリカバリーキーを控えた。実行の前の確認（署名し直さない相手など）に進む。 */
  onRecoveryKeySaved: () => void;
  onExecute: () => void;
  onRestart: () => void;
  onCancelRevocation: () => void;
  onFinish: () => void;
  /**
   * 再検証の確認の画面に出す鍵を読む（相手の今のマスター鍵と、前に信頼した鍵）。読めなければ `null`
   * （エラーは画面の状態に出す）。
   */
  onPrepareReverify: (userId: string) => Promise<ReverifyPreviewView | null>;
  /** 確認の画面で見せた鍵（`masterKey`）を受け入れる。サーバーの鍵が変わっていたら署名しない。 */
  onReverify: (userId: string, masterKey: string) => void;
  /**
   * 絵文字 SAS で検証する（#368）。この端末で検証を進められないとき（暗号ストアを開いていない
   * タブなど）は渡さない（ボタンを出さない）。
   */
  onSasVerify?: (userId: string) => void;
}

/** 絵文字 SAS で検証できる相手か（クロス署名鍵があり、まだ検証済みでない他のユーザー）。 */
function canSasVerify(state: UserState): boolean {
  if (state.state === "needs_reverification") return true;
  return (
    state.state === "trusted" && state.basis !== "own" && state.basis !== "user_signing_signature"
  );
}

function formatDate(value: string | null, locale: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString(locale, { dateStyle: "medium", timeStyle: "short" });
}

function ErrorMessage({ code }: { code: DevicesErrorCode | null }) {
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
      {label}
    </div>
  );
}

/** 端末の名前と日時。端末名は攻撃者も付けられるので、登録日時と最後に使った日時を並べる。 */
export function DeviceSummary({ device }: { device: DeviceRowView }) {
  const { t, i18n } = useTranslation();
  const created = formatDate(device.createdAt, i18n.language);
  const used = formatDate(device.lastSeenAt, i18n.language);
  return (
    <div className="min-w-0">
      <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
        <span className="truncate">{device.displayName || t("e2ee.devices.unnamed")}</span>
        {device.current ? (
          <span className="rounded bg-muted px-1.5 py-0.5 text-xs">
            {t("e2ee.devices.current")}
          </span>
        ) : null}
        {!device.signed ? (
          <span className="flex items-center gap-1 rounded bg-destructive/10 px-1.5 py-0.5 text-xs text-destructive">
            <ShieldAlertIcon aria-hidden="true" className="size-3" />
            {t("e2ee.devices.unsigned")}
          </span>
        ) : null}
      </p>
      <p className="text-xs text-muted-foreground">
        {[
          created ? t("e2ee.devices.created", { date: created }) : null,
          used ? t("e2ee.devices.lastSeen", { date: used }) : null,
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>
      <p className="font-mono text-[11px] text-muted-foreground" translate="no">
        {device.deviceId}
      </p>
    </div>
  );
}

function notReadyMessage(t: TFunction, reason: E2eeNotReadyReason): string {
  switch (reason.kind) {
    case "failed": {
      const failure = reason.failure;
      if (failure.source === "network") return t("e2ee.notReady.failedNetwork");
      if (failure.source === "local") {
        return failure.name
          ? t("e2ee.notReady.failedLocalNamed", { name: failure.name })
          : t("e2ee.notReady.failedLocal");
      }
      return failure.code
        ? t("e2ee.notReady.failedServerCode", { status: failure.status, code: failure.code })
        : t("e2ee.notReady.failedServer", { status: failure.status });
    }
    default:
      return t(`e2ee.notReady.${reason.kind}`);
  }
}

/**
 * このタブで暗号ストアをまだ開けていない理由（設定の「暗号化」タブの上に 1 回だけ出す）。
 * ほかのタブが開いている、登録に失敗して試し直している、などを区別して伝える。
 */
export function E2eeNotReadyNotice({ reason }: { reason: E2eeNotReadyReason }) {
  const { t } = useTranslation();
  return (
    <div role="status" className="space-y-1 text-sm text-muted-foreground">
      <p className="flex items-center gap-2">
        {reason.kind === "starting" && (
          <LoaderCircleIcon
            className="size-4 shrink-0 animate-spin motion-reduce:animate-none"
            aria-hidden="true"
          />
        )}
        <span>{notReadyMessage(t, reason)}</span>
      </p>
      {reason.kind === "failed" && <p className="text-xs">{t("e2ee.notReady.retrying")}</p>}
    </div>
  );
}

/**
 * 設定の「暗号化」タブの、端末（一覧、削除、失効）と他のユーザーの検証のセクション（#362）。
 * 状態は `DevicesSettings` が持つ。
 */
export function DevicesSettingsView({
  state,
  handlers,
}: {
  state: DevicesViewState;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  if (state.step === "loading") return <Busy label={t("e2ee.loading")} />;
  if (state.step === "loadFailed") {
    return (
      <div className="space-y-2">
        <ErrorMessage code="loadFailed" />
        <Button variant="outline" onClick={handlers.onRetry}>
          {t("e2ee.retry")}
        </Button>
      </div>
    );
  }
  if (state.revocation) {
    return <RevocationWizard state={state} phase={state.revocation} handlers={handlers} />;
  }
  return (
    <div className="space-y-4">
      {state.pendingRevocation ? (
        <SettingsSection
          title={t("e2ee.revocation.pendingTitle")}
          description={t("e2ee.revocation.pendingDescription")}
        >
          <Button disabled={state.busy !== null} onClick={handlers.onResumeRevocation}>
            {t("e2ee.revocation.resume")}
          </Button>
        </SettingsSection>
      ) : null}
      <DeviceList state={state} handlers={handlers} />
      <VerificationList state={state} handlers={handlers} />
    </div>
  );
}

type Ready = Extract<DevicesViewState, { step: "ready" }>;

function DeviceList({ state, handlers }: { state: Ready; handlers: DevicesSettingsHandlers }) {
  const { t } = useTranslation();
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [deleting, setDeleting] = useState<DeviceRowView | null>(null);
  const busy = state.busy !== null;
  return (
    <SettingsSection title={t("e2ee.devices.title")} description={t("e2ee.devices.description")}>
      <ErrorMessage code={state.error} />
      {state.error === "refreshFailed" ? (
        <Button variant="outline" size="sm" disabled={busy} onClick={handlers.onRetry}>
          {t("e2ee.retry")}
        </Button>
      ) : null}
      <ul className="divide-y rounded-md border">
        {state.devices.map((device) => (
          <li key={device.deviceId} className="flex items-start justify-between gap-3 p-3">
            <div className="flex min-w-0 items-start gap-3">
              <MonitorSmartphoneIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
              {renaming?.id === device.deviceId ? (
                <form
                  className="flex flex-wrap items-center gap-2"
                  onSubmit={(event) => {
                    event.preventDefault();
                    handlers.onRename(device.deviceId, renaming.name.trim());
                    setRenaming(null);
                  }}
                >
                  <Input
                    aria-label={t("e2ee.devices.name")}
                    value={renaming.name}
                    maxLength={100}
                    onChange={(event) => setRenaming({ ...renaming, name: event.target.value })}
                  />
                  <Button type="submit" size="sm" disabled={!renaming.name.trim()}>
                    {t("common.save")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" onClick={() => setRenaming(null)}>
                    {t("common.cancel")}
                  </Button>
                </form>
              ) : (
                <DeviceSummary device={device} />
              )}
            </div>
            <div className="flex shrink-0 gap-1">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("e2ee.devices.rename")}
                disabled={busy}
                onClick={() => setRenaming({ id: device.deviceId, name: device.displayName ?? "" })}
              >
                <PencilIcon />
              </Button>
              {!device.current ? (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("e2ee.devices.delete")}
                  disabled={busy}
                  onClick={() => setDeleting(device)}
                >
                  <Trash2Icon />
                </Button>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">{t("e2ee.devices.deleteIsNotRevocation")}</p>
      {state.unlocked ? (
        // 終わっていない失効があれば、新しい失効は始めない（上の「失効を続ける」から続ける）。
        <Button
          variant="destructive"
          disabled={busy || state.pendingRevocation !== null}
          onClick={() => handlers.onStartRevocation([])}
        >
          <ShieldAlertIcon />
          {t("e2ee.revocation.start")}
        </Button>
      ) : (
        <p className="text-xs text-muted-foreground">{t("e2ee.revocation.needsUnlock")}</p>
      )}
      {state.busy === "rename" || state.busy === "delete" ? (
        <Busy label={t("e2ee.saving")} />
      ) : null}
      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        title={t("e2ee.devices.deleteConfirmTitle")}
        description={t("e2ee.devices.deleteConfirmDescription")}
        confirmLabel={t("e2ee.devices.deleteOnly")}
        onConfirm={() => {
          if (deleting) handlers.onDelete(deleting.deviceId);
          setDeleting(null);
        }}
      />
    </SettingsSection>
  );
}

function verificationLabel(state: UserState, t: TFunction): string {
  switch (state.state) {
    case "trusted":
      return t(`e2ee.verification.trusted.${state.basis}`);
    case "needs_reverification":
      return t(`e2ee.verification.reason.${state.reason}`);
    default:
      return t(`e2ee.verification.state.${state.state}`);
  }
}

function VerificationList({
  state,
  handlers,
}: {
  state: Ready;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState<{
    row: VerificationRowView;
    preview: ReverifyPreviewView;
  } | null>(null);
  const needs = state.verifications.filter((v) => v.state.state === "needs_reverification");
  const others = state.verifications.filter((v) => v.state.state !== "needs_reverification");
  return (
    <SettingsSection
      title={t("e2ee.verification.title")}
      description={t("e2ee.verification.description")}
    >
      {state.ownMasterKey ? (
        <div className="space-y-1">
          <p className="text-sm font-medium">{t("e2ee.verification.ownFingerprint")}</p>
          <KeyFingerprint publicKey={state.ownMasterKey} />
          <p className="text-xs text-muted-foreground">
            {t("e2ee.verification.ownFingerprintHelp")}
          </p>
        </div>
      ) : null}
      {state.warnings.map((warning) => (
        <p key={warning} role="alert" className="flex items-start gap-2 text-sm text-destructive">
          <ShieldAlertIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
          {t(`e2ee.verification.warning.${warning}`)}
        </p>
      ))}
      {state.verifications.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("e2ee.verification.empty")}</p>
      ) : (
        <ul className="divide-y rounded-md border">
          {[...needs, ...others].map((row) => {
            const reverify = row.state.state === "needs_reverification";
            return (
              <li key={row.userId} className="flex items-center justify-between gap-3 p-3">
                <div className="flex min-w-0 items-center gap-3">
                  {reverify ? (
                    <ShieldAlertIcon
                      aria-hidden="true"
                      className="size-4 shrink-0 text-destructive"
                    />
                  ) : row.state.state === "trusted" ? (
                    <ShieldCheckIcon aria-hidden="true" className="size-4 shrink-0" />
                  ) : (
                    <ShieldIcon
                      aria-hidden="true"
                      className="size-4 shrink-0 text-muted-foreground"
                    />
                  )}
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{row.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {verificationLabel(row.state, t)}
                    </p>
                  </div>
                </div>
                <div className="flex shrink-0 flex-wrap justify-end gap-2">
                  {handlers.onSasVerify && state.unlocked && canSasVerify(row.state) ? (
                    <Button
                      variant={reverify ? "default" : "outline"}
                      size="sm"
                      disabled={state.busy !== null}
                      onClick={() => handlers.onSasVerify?.(row.userId)}
                    >
                      {t("e2ee.sas.start")}
                    </Button>
                  ) : null}
                  {reverify && state.unlocked ? (
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={state.busy !== null}
                      onClick={() => {
                        void handlers.onPrepareReverify(row.userId).then((preview) => {
                          if (preview) setConfirming({ row, preview });
                        });
                      }}
                    >
                      {t("e2ee.verification.reverify")}
                    </Button>
                  ) : null}
                </div>
              </li>
            );
          })}
        </ul>
      )}
      {state.busy === "verify" ? <Busy label={t("e2ee.saving")} /> : null}
      <ReverifyDialog
        target={confirming}
        onClose={() => setConfirming(null)}
        onConfirm={(userId, masterKey) => {
          handlers.onReverify(userId, masterKey);
          setConfirming(null);
        }}
        onSasVerify={
          handlers.onSasVerify
            ? (userId) => {
                handlers.onSasVerify?.(userId);
                setConfirming(null);
              }
            : undefined
        }
      />
    </SettingsSection>
  );
}

/** 鍵の指紋（公開鍵を 4 文字ずつに区切ったもの）。 */
export function KeyFingerprint({ publicKey }: { publicKey: string }) {
  return (
    <p className="flex flex-wrap gap-x-2 rounded-md border bg-muted/50 px-3 py-2 font-mono text-sm">
      {/* 4 文字の区切りの途中では折り返さない（読み合わせやすいように）。 */}
      {formatKeyFingerprint(publicKey)
        .split(" ")
        .map((group, index) => (
          <span key={index}>{group}</span>
        ))}
    </p>
  );
}

/**
 * 再検証の確認（セキュリティレビュー M-2）。相手の今のマスター鍵の指紋と、前に信頼した鍵から
 * 変わったかを見せ、チャンネルの外で比べてもらう。絵文字 SAS で検証できるなら、そちらを勧める。
 * 受け入れると、見せた鍵（`preview.masterKey`）にだけ署名する。
 */
export function ReverifyDialog({
  target,
  onClose,
  onConfirm,
  onSasVerify,
}: {
  target: { row: VerificationRowView; preview: ReverifyPreviewView } | null;
  onClose: () => void;
  onConfirm: (userId: string, masterKey: string) => void;
  onSasVerify?: (userId: string) => void;
}) {
  const { t } = useTranslation();
  const preview = target?.preview;
  const previous = preview?.previousMasterKey ?? null;
  const changed = previous !== null && previous !== preview?.masterKey;
  return (
    <AlertDialog
      open={target !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("e2ee.verification.confirmTitle", { name: target?.row.name ?? "" })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("e2ee.verification.confirmDescription", { name: target?.row.name ?? "" })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {preview ? (
          <div className="space-y-3">
            <div className="space-y-1">
              <p className="text-sm font-medium">{t("e2ee.verification.currentFingerprint")}</p>
              <KeyFingerprint publicKey={preview.masterKey} />
            </div>
            {changed ? (
              <>
                <p role="alert" className="flex items-start gap-2 text-sm text-destructive">
                  <ShieldAlertIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
                  {t("e2ee.verification.keyChanged")}
                </p>
                <div className="space-y-1">
                  <p className="text-sm font-medium text-muted-foreground">
                    {t("e2ee.verification.previousFingerprint")}
                  </p>
                  <KeyFingerprint publicKey={previous} />
                </div>
              </>
            ) : previous !== null ? (
              <p className="text-sm text-muted-foreground">{t("e2ee.verification.keyUnchanged")}</p>
            ) : null}
            {onSasVerify ? (
              <p className="text-sm">{t("e2ee.verification.sasRecommended")}</p>
            ) : null}
            <p className="text-xs text-muted-foreground">{t("e2ee.verification.confirmEffect")}</p>
          </div>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          {onSasVerify && target ? (
            <Button variant="outline" onClick={() => onSasVerify(target.row.userId)}>
              {t("e2ee.sas.start")}
            </Button>
          ) : null}
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              if (target) onConfirm(target.row.userId, target.preview.masterKey);
            }}
          >
            {t("e2ee.verification.confirmTrust")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** AuthZ で行う手順（今の AuthZ には 1 回で失効する API がない。docs/e2ee.md の【決定】）。 */
export function AuthzSteps({
  urls,
  after,
}: {
  urls: { security: string; services: string } | null;
  after: boolean;
}) {
  const { t } = useTranslation();
  const steps = after
    ? (["e2ee.authz.disconnectAgain", "e2ee.authz.signOut"] as const)
    : (["e2ee.authz.revokeSessions", "e2ee.authz.disconnect"] as const);
  return (
    <div className="space-y-2 rounded-md border p-3">
      <p className="text-sm font-medium">
        {t(after ? "e2ee.authz.afterTitle" : "e2ee.authz.beforeTitle")}
      </p>
      <ol className="list-decimal space-y-1 pl-5 text-sm">
        {steps.map((key) => (
          <li key={key}>{t(key)}</li>
        ))}
      </ol>
      <p className="text-xs text-muted-foreground">{t("e2ee.authz.note")}</p>
      {urls ? (
        <div className="flex flex-wrap gap-3">
          {(["security", "services"] as const).map((page) => (
            <a
              key={page}
              className="inline-flex items-center gap-1 text-sm underline"
              href={urls[page]}
              target="_blank"
              rel="noreferrer"
            >
              <ExternalLinkIcon aria-hidden="true" className="size-4" />
              {t(`e2ee.authz.open.${page}`)}
            </a>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function NoteList({ notes }: { notes: ResignNoteView[] }) {
  const { t } = useTranslation();
  if (notes.length === 0) return null;
  return (
    <div className="space-y-1">
      <p className="text-sm font-medium">{t("e2ee.revocation.notesTitle")}</p>
      <p className="text-xs text-muted-foreground">{t("e2ee.revocation.notesDescription")}</p>
      <ul className="divide-y rounded-md border">
        {notes.map((note) => (
          <li key={note.userId} className="p-2 text-sm">
            <span className="font-medium">{note.name}</span>
            <span className="text-xs text-muted-foreground">
              {" — "}
              {note.notes.map((n) => t(`e2ee.revocation.note.${n}`)).join("、")}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function RevocationWizard({
  state,
  phase,
  handlers,
}: {
  state: Ready;
  phase: RevocationPhase;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  const busy = state.busy !== null;
  const cancel = (
    <Button variant="ghost" disabled={busy} onClick={handlers.onCancelRevocation}>
      {t("common.cancel")}
    </Button>
  );
  return (
    <div className="space-y-4">
      <SettingsSection
        title={t("e2ee.revocation.title")}
        description={t("e2ee.revocation.description")}
      >
        <ErrorMessage code={state.error} />
        {phase.phase === "select" ? (
          <SelectPhase state={state} phase={phase} handlers={handlers} />
        ) : null}
        {phase.phase === "unlock" ? (
          <UnlockPhase state={state} resume={phase.resume} handlers={handlers} />
        ) : null}
        {phase.phase === "recoveryKey" ? (
          <RecoveryKeyPhase state={state} phase={phase} handlers={handlers} />
        ) : null}
        {phase.phase === "confirm" ? (
          <div className="space-y-3">
            <NoteList notes={phase.notes} />
            {phase.versionsWithoutKey.length > 0 ? (
              <p className="text-sm text-destructive">
                {t("e2ee.revocation.versionsWithoutKey", {
                  versions: phase.versionsWithoutKey.join(", "),
                })}
              </p>
            ) : null}
            <p className="text-xs text-muted-foreground">
              {t("e2ee.revocation.executeDescription")}
            </p>
            <Button variant="destructive" disabled={busy} onClick={handlers.onExecute}>
              {t("e2ee.revocation.execute")}
            </Button>
          </div>
        ) : null}
        {phase.phase === "running" ? (
          <Busy label={t(`e2ee.revocation.stage.${phase.stage}`)} />
        ) : null}
        {phase.phase === "conflict" ? (
          <div className="space-y-2">
            <p className="text-sm">{t("e2ee.revocation.conflict")}</p>
            <Button disabled={busy} onClick={handlers.onRestart}>
              {t("e2ee.revocation.restart")}
            </Button>
          </div>
        ) : null}
        {phase.phase === "done" ? (
          <DonePhase state={state} result={phase.result} handlers={handlers} />
        ) : null}
      </SettingsSection>
      {phase.phase === "select" || phase.phase === "unlock" ? cancel : null}
    </div>
  );
}

function SelectPhase({
  state,
  phase,
  handlers,
}: {
  state: Ready;
  phase: Extract<RevocationPhase, { phase: "select" }>;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  const scopeName = useId();
  const [acknowledged, setAcknowledged] = useState(false);
  const toggle = (deviceId: string, checked: boolean) =>
    handlers.onSelectionChange(
      checked
        ? [...phase.selected, deviceId]
        : phase.selected.filter((selected) => selected !== deviceId),
    );
  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <p className="text-sm font-medium">{t("e2ee.revocation.selectTitle")}</p>
        <p className="text-xs text-muted-foreground">{t("e2ee.revocation.selectDescription")}</p>
        <ul className="divide-y rounded-md border">
          {state.devices
            .filter((device) => !device.current)
            .map((device) => (
              <li key={device.deviceId} className="p-3">
                <label className="flex items-start gap-3">
                  <input
                    type="checkbox"
                    className="mt-1 size-4"
                    checked={phase.selected.includes(device.deviceId)}
                    onChange={(event) => toggle(device.deviceId, event.target.checked)}
                  />
                  <DeviceSummary device={device} />
                </label>
              </li>
            ))}
        </ul>
      </div>
      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">{t("e2ee.revocation.scopeTitle")}</legend>
        {(["device_only", "full"] as const).map((scope) => (
          <label key={scope} className="flex items-start gap-2 text-sm">
            <input
              type="radio"
              name={scopeName}
              className="mt-1"
              checked={phase.scope === scope}
              onChange={() => handlers.onScopeChange(scope)}
            />
            <span>
              <span className="font-medium">{t(`e2ee.revocation.scope.${scope}`)}</span>
              <span className="block text-xs text-muted-foreground">
                {t(`e2ee.revocation.scopeDescription.${scope}`)}
              </span>
            </span>
          </label>
        ))}
      </fieldset>
      <AuthzSteps urls={state.accountUrls} after={false} />
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="size-4"
          checked={acknowledged}
          onChange={(event) => setAcknowledged(event.target.checked)}
        />
        {t("e2ee.authz.acknowledge")}
      </label>
      <p className="text-xs text-muted-foreground">{t("e2ee.revocation.irreversible")}</p>
      <Button disabled={!acknowledged || state.busy !== null} onClick={handlers.onConfirmSelection}>
        {t("e2ee.revocation.next")}
      </Button>
    </div>
  );
}

function UnlockPhase({
  state,
  resume,
  handlers,
}: {
  state: Ready;
  resume: boolean;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  const [recoveryKey, setRecoveryKey] = useState("");
  const inputId = useId();
  const busy = state.busy !== null;
  return (
    <div className="space-y-3">
      <p className="text-sm">
        {t(resume ? "e2ee.revocation.unlockResume" : "e2ee.revocation.unlockDescription")}
      </p>
      {state.passkeyAvailable ? (
        <Button disabled={busy} onClick={handlers.onUnlockWithPasskey}>
          <FingerprintIcon />
          {t("e2ee.unlockWithPasskey")}
        </Button>
      ) : null}
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
    </div>
  );
}

function RecoveryKeyPhase({
  state,
  phase,
  handlers,
}: {
  state: Ready;
  phase: Extract<RevocationPhase, { phase: "recoveryKey" }>;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  const [confirmed, setConfirmed] = useState(false);
  const [copied, setCopied] = useState(false);
  const confirmId = useId();
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(phase.recoveryKey);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="space-y-3">
      <p className="text-sm">{t("e2ee.revocation.newRecoveryKey")}</p>
      <p
        aria-label={t("e2ee.recoveryKeyTitle")}
        className="select-all break-words rounded-md bg-muted p-3 font-mono text-sm tracking-wide"
        translate="no"
      >
        {phase.recoveryKey}
      </p>
      <Button variant="outline" size="sm" onClick={() => void copy()}>
        {copied ? <CheckIcon /> : <CopyIcon />}
        {copied ? t("e2ee.copied") : t("e2ee.copy")}
      </Button>
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
      <p className="text-xs text-muted-foreground">{t("e2ee.revocation.passkeyDescription")}</p>
      {state.passkeyAvailable ? (
        phase.passkeyAdded ? (
          <p className="flex items-center gap-2 text-sm">
            <CheckIcon aria-hidden="true" className="size-4" />
            {t("e2ee.passkeyAdded")}
          </p>
        ) : (
          <Button
            variant="outline"
            disabled={state.busy !== null}
            onClick={handlers.onAddRevocationPasskey}
          >
            <FingerprintIcon />
            {t("e2ee.addPasskey")}
          </Button>
        )
      ) : null}
      <p className="text-xs text-muted-foreground">{t("e2ee.revocation.fullWarning")}</p>
      <Button disabled={!confirmed || state.busy !== null} onClick={handlers.onRecoveryKeySaved}>
        {t("e2ee.revocation.next")}
      </Button>
    </div>
  );
}

function DonePhase({
  state,
  result,
  handlers,
}: {
  state: Ready;
  result: RevocationResultView;
  handlers: DevicesSettingsHandlers;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-3">
      <p className="flex items-center gap-2 text-sm font-medium">
        <ShieldCheckIcon aria-hidden="true" className="size-4" />
        {t("e2ee.revocation.done")}
      </p>
      <NoteList notes={result.notes} />
      {result.unsignedDevices.length > 0 ? (
        <div className="space-y-1">
          <p className="text-sm font-medium">{t("e2ee.revocation.unsignedTitle")}</p>
          <p className="text-xs text-muted-foreground">
            {t("e2ee.revocation.unsignedDescription")}
          </p>
          <ul className="divide-y rounded-md border">
            {result.unsignedDevices.map((device) => (
              <li key={device.deviceId} className="p-2">
                <DeviceSummary device={device} />
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {result.versionsWithoutKey.length > 0 ? (
        <p className="text-sm text-destructive">
          {t("e2ee.revocation.versionsWithoutKey", {
            versions: result.versionsWithoutKey.join(", "),
          })}
        </p>
      ) : null}
      {result.droppedSessions > 0 ? (
        <p className="text-sm text-destructive">
          {t("e2ee.revocation.droppedSessions", { count: result.droppedSessions })}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">{t("e2ee.revocation.irreversible")}</p>
      <AuthzSteps urls={state.accountUrls} after />
      <Button onClick={handlers.onFinish}>{t("e2ee.revocation.finish")}</Button>
    </div>
  );
}

/** 20 台の上限で、この端末を登録できなかったとき（docs/e2ee.md の `stale_devices`）。 */
export function DeviceLimitView({
  devices,
  staleIds,
  busy,
  done,
  error,
  onDelete,
}: {
  devices: DeviceRowView[];
  /** 30 日以上使っていない端末（サーバーの `stale_devices`）。 */
  staleIds: string[];
  busy: boolean;
  done: boolean;
  error: DevicesErrorCode | null;
  onDelete: (deviceIds: string[]) => void;
}) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<string[]>(staleIds);
  const stale = new Set(staleIds);
  return (
    <SettingsSection
      className="border-destructive/50"
      title={t("e2ee.devices.limitTitle")}
      description={t("e2ee.devices.limitDescription")}
    >
      <ErrorMessage code={error} />
      <ul className="divide-y rounded-md border">
        {devices.map((device) => (
          <li key={device.deviceId} className="p-3">
            <label className="flex items-start gap-3">
              <input
                type="checkbox"
                className="mt-1 size-4"
                checked={selected.includes(device.deviceId)}
                onChange={(event) =>
                  setSelected((current) =>
                    event.target.checked
                      ? [...current, device.deviceId]
                      : current.filter((id) => id !== device.deviceId),
                  )
                }
              />
              <div className="min-w-0">
                <DeviceSummary device={device} />
                {stale.has(device.deviceId) ? (
                  <p className="text-xs text-muted-foreground">{t("e2ee.devices.limitStale")}</p>
                ) : null}
              </div>
            </label>
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">{t("e2ee.devices.deleteIsNotRevocation")}</p>
      {busy ? (
        <Busy label={t("e2ee.saving")} />
      ) : done ? (
        <p className="flex items-center gap-2 text-sm">
          <CheckIcon aria-hidden="true" className="size-4" />
          {t("e2ee.devices.limitDeleted")}
        </p>
      ) : (
        <Button
          variant="destructive"
          disabled={selected.length === 0}
          onClick={() => onDelete(selected)}
        >
          <Trash2Icon />
          {t("e2ee.devices.limitDelete")}
        </Button>
      )}
    </SettingsSection>
  );
}
