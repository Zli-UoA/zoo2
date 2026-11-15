import { PhoneIcon, PhoneOffIcon } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";

import { AppAvatar } from "../chat/AppIdentity";
import { Button } from "../ui/button";

export interface IncomingCallCardProps {
  /** 呼んだ人の表示名。 */
  inviterName: string;
  inviterAvatarUrl?: string | null;
  /** チャンネルの表示名。DM では相手の名前（呼んだ人と同じ）。 */
  channelLabel: string;
  isDm: boolean;
  /** 応答・拒否の処理中。 */
  busy?: boolean;
  /** 応答できなかったときの文言。 */
  error?: string | null;
  onAccept: () => void;
  onDecline: () => void;
}

/** 着信の表示（応答と拒否）。操作と着信音は `IncomingCallDialog` が受け持つ。 */
export function IncomingCallCard({
  inviterName,
  inviterAvatarUrl,
  channelLabel,
  isDm,
  busy = false,
  error = null,
  onAccept,
  onDecline,
}: IncomingCallCardProps) {
  const { t } = useTranslation();
  const titleId = useId();
  return (
    <section
      role="alertdialog"
      aria-labelledby={titleId}
      aria-live="assertive"
      className="fixed top-4 left-4 right-4 z-[70] mx-auto flex max-w-sm items-center gap-3 rounded-2xl border bg-card px-4 py-3 shadow-2xl md:left-auto md:right-4 md:mx-0 md:w-96"
    >
      <span className="relative flex shrink-0">
        <span
          aria-hidden="true"
          className="absolute inset-0 animate-ping rounded-full bg-emerald-400/40"
        />
        <AppAvatar label={inviterName} src={inviterAvatarUrl} className="relative size-10" />
      </span>
      <div className="min-w-0 flex-1">
        <p id={titleId} className="truncate text-sm font-semibold">
          {isDm
            ? t("call.incomingDm", { name: inviterName })
            : t("call.incomingChannel", { name: inviterName })}
        </p>
        <p className="truncate text-xs text-muted-foreground" title={channelLabel}>
          {isDm ? t("call.incomingDmHint") : channelLabel}
        </p>
        {error ? (
          <p role="alert" className="mt-1 text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <Button
          type="button"
          variant="destructive"
          size="icon-sm"
          className="size-9 rounded-full"
          aria-label={t("call.decline")}
          title={t("call.decline")}
          disabled={busy}
          onClick={onDecline}
        >
          <PhoneOffIcon className="size-4" />
        </Button>
        <Button
          type="button"
          size="icon-sm"
          className="size-9 rounded-full bg-emerald-600 text-white hover:bg-emerald-700"
          aria-label={t("call.accept")}
          title={t("call.accept")}
          disabled={busy}
          onClick={onAccept}
        >
          <PhoneIcon className="size-4" />
        </Button>
      </div>
    </section>
  );
}
