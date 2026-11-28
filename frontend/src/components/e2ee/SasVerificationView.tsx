import { lazy, Suspense, useState, type ReactNode } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { CameraIcon, QrCodeIcon, ShieldAlertIcon, ShieldCheckIcon, SmileIcon } from "lucide-react";

import type { QrScanResult, SasEntry } from "../../lib/e2ee/sasController";
import { SAS_EMOJI } from "../../lib/e2ee/sasEmoji";
import type { QrCodeScannerProps } from "./QrCodeScanner";

// QR の生成・読み取りは検証ダイアログを開いたときだけ要る。jsqr / qrcode-generator を
// 初期チャンクから外すため、ダイアログ内の部品だけ遅延読み込みする。
const QrCodeImage = lazy(() =>
  import("./QrCodeImage").then((module) => ({ default: module.QrCodeImage })),
);
const QrCodeScanner = lazy(() =>
  import("./QrCodeScanner").then((module) => ({ default: module.QrCodeScanner })),
);
import { Button } from "../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";
import { Spinner } from "../ui/spinner";

export interface SasVerificationHandlers {
  onAccept: () => void;
  /** 要求を断る、または途中で取りやめる。 */
  onCancel: () => void;
  onMatch: () => void;
  onMismatch: () => void;
  onRetrySave: () => void;
  /** 画面を閉じる（終わっていなければ取りやめる）。 */
  onClose: () => void;
  /** QR を読み取らずに、絵文字で検証する。 */
  onStartSas: () => void;
  /** 相手の QR を読み取った。 */
  onScanQr: (bytes: Uint8Array) => Promise<QrScanResult>;
  /** 相手がこの端末の QR を読み取ったと、利用者が答えた。 */
  onQrConfirm: () => void;
}

/** `m.key.verification.cancel` の `code` から、出す文言のキーを選ぶ。 */
function cancelMessage(entry: SasEntry, t: TFunction): string {
  const cancel = entry.view.cancel;
  switch (cancel?.code) {
    case "m.user":
      return t(cancel.by_us ? "e2ee.sas.cancelled.user" : "e2ee.sas.cancelled.userByThem");
    case "m.timeout":
      return t("e2ee.sas.cancelled.timeout");
    case "m.mismatched_sas":
      return t("e2ee.sas.cancelled.mismatchedSas");
    case "m.key_mismatch":
      return t(
        entry.view.method === "qr" ? "e2ee.qr.keyMismatch" : "e2ee.sas.cancelled.keyMismatch",
      );
    case "m.mismatched_commitment":
      return t("e2ee.sas.cancelled.keyMismatch");
    case "m.accepted":
      return t("e2ee.sas.cancelled.accepted");
    case "m.unknown_method":
      return t("e2ee.sas.cancelled.unknownMethod");
    default:
      return t("e2ee.sas.cancelled.other");
  }
}

function Busy({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-2 text-sm text-muted-foreground">
      <Spinner label={label} className="[&>svg]:size-4" />
      {label}
    </div>
  );
}

/** 7 つの絵文字と名前。相手の画面と同じ順に並ぶ。 */
export function SasEmojiList({ emoji }: { emoji: number[] }) {
  const { t } = useTranslation();
  return (
    <ol aria-label={t("e2ee.sas.emojiList")} className="grid grid-cols-4 gap-1.5 sm:grid-cols-7">
      {emoji.map((index, position) => (
        <li
          // 同じ絵文字が 2 回出ることがあるので、位置をキーにする。
          key={position}
          className="flex min-w-0 flex-col items-center gap-1 rounded-md border px-1 py-2 text-center"
        >
          <span aria-hidden="true" className="text-3xl leading-none">
            {SAS_EMOJI[index]}
          </span>
          <span className="w-full break-words text-micro leading-tight text-muted-foreground">
            {
              // 翻訳のキーは 0〜63 の文字列（型では 1 つに代表させる）。
              t(`e2ee.sas.emoji.${String(index) as "0"}`)
            }
          </span>
        </li>
      ))}
    </ol>
  );
}

/**
 * 方式を選ぶ段階の本文：この端末の QR（相手に読み取ってもらう）か、相手の QR を読み取るカメラと、
 * その下の「相手の QR を読み取る」「絵文字で検証する」。
 */
function ChooseMethod({
  entry,
  name,
  handlers,
  scannerProps,
}: {
  entry: SasEntry;
  name: string;
  handlers: SasVerificationHandlers;
  scannerProps?: Partial<QrCodeScannerProps>;
}) {
  const { t } = useTranslation();
  const { view, busy } = entry;
  const [scanning, setScanning] = useState(false);
  const canScan = view.phase === "choose_method" && view.can_scan_qr_code;
  const showScanner = scanning && canScan;
  return (
    <div className="flex flex-col items-center gap-3">
      {showScanner ? (
        <Suspense fallback={null}>
          <QrCodeScanner onScan={handlers.onScanQr} {...scannerProps} />
        </Suspense>
      ) : view.qr_code ? (
        <>
          <Suspense fallback={null}>
            <QrCodeImage bytes={view.qr_code} label={t("e2ee.qr.codeLabel")} />
          </Suspense>
          <p className="text-center text-sm text-muted-foreground">
            {view.phase === "requested"
              ? t("e2ee.qr.waitingForAccept", { name })
              : t("e2ee.qr.showHint", { name })}
          </p>
        </>
      ) : null}
      <div className="flex flex-wrap justify-center gap-2">
        {canScan ? (
          <Button variant="outline" disabled={busy} onClick={() => setScanning(!scanning)}>
            {showScanner ? <QrCodeIcon aria-hidden="true" /> : <CameraIcon aria-hidden="true" />}
            {showScanner ? t("e2ee.qr.showOwn") : t("e2ee.qr.scan")}
          </Button>
        ) : null}
        {view.can_start_sas ? (
          <Button variant="outline" disabled={busy} onClick={handlers.onStartSas}>
            <SmileIcon aria-hidden="true" />
            {t("e2ee.qr.useEmoji")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

/**
 * 他のユーザーの検証のダイアログ（#368 の絵文字 SAS と、QR コードによる検証）。QR を先に出し、その下に
 * 「絵文字で検証する」を出す。`entry` が `null` なら閉じる。
 */
export function SasVerificationView({
  entry,
  name,
  handlers,
  scannerProps,
}: {
  entry: SasEntry | null;
  /** 相手の表示名。 */
  name: string;
  handlers: SasVerificationHandlers;
  /** Storybook とテスト用：カメラの代わり。 */
  scannerProps?: Partial<QrCodeScannerProps>;
}) {
  const { t } = useTranslation();
  const view = entry?.view;
  const busy = entry?.busy ?? false;
  let title = "";
  let description: string | null = null;
  let body: ReactNode = null;
  let footer: ReactNode = null;

  const closeButton = (
    <Button variant="outline" onClick={handlers.onClose}>
      {t("e2ee.sas.close")}
    </Button>
  );
  const cancelButton = (
    <Button variant="outline" disabled={busy} onClick={handlers.onCancel}>
      {t("e2ee.sas.cancel")}
    </Button>
  );

  switch (view?.phase) {
    case "incoming":
      title = t("e2ee.sas.incomingTitle", { name });
      description = t("e2ee.sas.incomingDescription");
      footer = (
        <>
          <Button variant="outline" disabled={busy} onClick={handlers.onCancel}>
            {t("e2ee.sas.decline")}
          </Button>
          <Button disabled={busy} onClick={handlers.onAccept}>
            {t("e2ee.sas.accept")}
          </Button>
        </>
      );
      break;
    case "requested":
      if (entry && (view.qr_code || view.can_start_sas)) {
        // QR を先に出し、その下に「絵文字で検証する」を出す（相手の受け入れを待つあいだも）。
        title = t("e2ee.qr.title", { name });
        body = <ChooseMethod entry={entry} name={name} handlers={handlers} />;
      } else {
        // 絵文字を選んだあと、または相手が QR を読み取れない。
        title = t("e2ee.sas.requestedTitle", { name });
        description = t("e2ee.sas.requestedDescription");
        body = <Busy label={t("e2ee.sas.requestedTitle", { name })} />;
      }
      footer = cancelButton;
      break;
    case "choose_method":
      title = t("e2ee.qr.title", { name });
      body = entry ? (
        <ChooseMethod entry={entry} name={name} handlers={handlers} scannerProps={scannerProps} />
      ) : null;
      footer = cancelButton;
      break;
    case "qr_scanned":
      title = t("e2ee.qr.scannedTitle", { name });
      body = <Busy label={t("e2ee.qr.waitingForConfirm")} />;
      footer = cancelButton;
      break;
    case "qr_confirm":
      title = t("e2ee.qr.confirmTitle", { name });
      description = t("e2ee.qr.confirmDescription", { name });
      footer = (
        <>
          <Button variant="outline" disabled={busy} onClick={handlers.onCancel}>
            {t("e2ee.qr.confirmNo")}
          </Button>
          <Button disabled={busy} onClick={handlers.onQrConfirm}>
            {t("e2ee.qr.confirmYes")}
          </Button>
        </>
      );
      break;
    case "exchanging":
      title = t("e2ee.sas.compareTitle", { name });
      body = <Busy label={t("e2ee.sas.exchanging")} />;
      footer = cancelButton;
      break;
    case "compare":
      title = t("e2ee.sas.compareTitle", { name });
      description = t("e2ee.sas.compareDescription");
      body = view.emoji ? <SasEmojiList emoji={view.emoji} /> : null;
      footer = (
        <>
          <Button variant="outline" disabled={busy} onClick={handlers.onMismatch}>
            {t("e2ee.sas.mismatch")}
          </Button>
          <Button disabled={busy} onClick={handlers.onMatch}>
            {t("e2ee.sas.match")}
          </Button>
        </>
      );
      break;
    case "confirmed":
      title = t("e2ee.sas.compareTitle", { name });
      body = (
        <>
          {view.emoji ? <SasEmojiList emoji={view.emoji} /> : null}
          <Busy label={t("e2ee.sas.confirmed")} />
        </>
      );
      footer = cancelButton;
      break;
    case "done":
      if (entry?.save === "saved") {
        title = t("e2ee.sas.savedTitle", { name });
        description = t("e2ee.sas.savedDescription", { name });
        body = (
          <p className="flex items-center gap-2 text-sm">
            <ShieldCheckIcon aria-hidden="true" className="size-4 shrink-0" />
            {t("e2ee.verification.trusted.user_signing_signature")}
          </p>
        );
        footer = closeButton;
      } else if (entry?.save === "login") {
        // QR を読み取った側：読み取りはクリックではないので、ここで再認証の別のウィンドウを開く。
        title = t("e2ee.qr.verifiedTitle", { name });
        body = (
          <p className="flex items-start gap-2 text-sm">
            <ShieldCheckIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
            {t("e2ee.qr.needsLogin")}
          </p>
        );
        footer = (
          <>
            {closeButton}
            <Button onClick={handlers.onRetrySave}>{t("e2ee.qr.login")}</Button>
          </>
        );
      } else if (entry?.save === "reauth" || entry?.save === "failed") {
        title =
          view.method === "qr"
            ? t("e2ee.qr.verifiedTitle", { name })
            : t("e2ee.sas.compareTitle", { name });
        body = (
          <p role="alert" className="flex items-start gap-2 text-sm text-destructive">
            <ShieldAlertIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
            {entry.save === "reauth" ? t("e2ee.sas.reauth") : t("e2ee.sas.saveFailed")}
          </p>
        );
        footer = (
          <>
            {closeButton}
            <Button onClick={handlers.onRetrySave}>{t("e2ee.sas.retrySave")}</Button>
          </>
        );
      } else {
        title =
          view.method === "qr"
            ? t("e2ee.qr.verifiedTitle", { name })
            : t("e2ee.sas.compareTitle", { name });
        body = <Busy label={t("e2ee.sas.saving")} />;
      }
      break;
    case "cancelled":
      title = t("e2ee.sas.cancelledTitle");
      body = (
        <p role="alert" className="flex items-start gap-2 text-sm">
          <ShieldAlertIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
          {entry ? cancelMessage(entry, t) : null}
        </p>
      );
      footer = closeButton;
      break;
    default:
      break;
  }

  const error = entry?.error
    ? t(
        entry.error === "needsUnlock"
          ? "e2ee.sas.needsUnlock"
          : entry.error === "sendFailed"
            ? "e2ee.sas.sendFailed"
            : entry.error === "receiveFailed"
              ? "e2ee.sas.receiveFailed"
              : "e2ee.sas.failed",
      )
    : null;

  return (
    <Dialog
      open={entry !== null}
      onOpenChange={(open) => {
        if (!open) handlers.onClose();
      }}
    >
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description ? <DialogDescription>{description}</DialogDescription> : null}
        </DialogHeader>
        {body}
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        {footer ? <DialogFooter>{footer}</DialogFooter> : null}
      </DialogContent>
    </Dialog>
  );
}
