import type { ReactNode } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { ShieldAlertIcon, ShieldCheckIcon } from "lucide-react";

import type { SasEntry } from "../../lib/e2ee/sasController";
import { SAS_EMOJI } from "../../lib/e2ee/sasEmoji";
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
          <span className="w-full break-words text-[11px] leading-tight text-muted-foreground">
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
 * 他のユーザーの絵文字 SAS 検証のダイアログ（#368）。`entry` が `null` なら閉じる。
 */
export function SasVerificationView({
  entry,
  name,
  handlers,
}: {
  entry: SasEntry | null;
  /** 相手の表示名。 */
  name: string;
  handlers: SasVerificationHandlers;
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
      title = t("e2ee.sas.requestedTitle", { name });
      description = t("e2ee.sas.requestedDescription");
      body = <Busy label={t("e2ee.sas.requestedTitle", { name })} />;
      footer = cancelButton;
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
      } else if (entry?.save === "reauth" || entry?.save === "failed") {
        title = t("e2ee.sas.compareTitle", { name });
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
        title = t("e2ee.sas.compareTitle", { name });
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
