import { DownloadIcon, LoaderCircleIcon, SquareIcon } from "lucide-react";

import type { DesktopDownloadState } from "../../lib/desktopDownloads";
import { Button } from "../ui/button";
import { cn } from "cn";

/**
 * 添付の保存ボタン。デスクトップ版で保存している間は、外周の円で進み具合を出す。
 *
 * 円を押すと中止する。マウスを乗せたときとキーボードで選んだときは、円の中心に停止の四角を出す
 * （円だけでは押せることが分かりにくいため）。100% になると円が閉じ、そのまま名前に置き換わる。
 */
export function DownloadProgressButton({
  state,
  pending = false,
  showLabel = false,
  downloadLabel,
  downloadShortLabel,
  cancelLabel,
  cancelShortLabel,
  progressLabel,
  onDownload,
  onCancel,
}: {
  /** デスクトップ版の保存の状態。保存していなければ `undefined`。 */
  state: DesktopDownloadState | undefined;
  /** デスクトップ版ではないときの、保存の準備中（ブラウザーの保存）。 */
  pending?: boolean;
  showLabel?: boolean;
  downloadLabel: string;
  downloadShortLabel: string;
  cancelLabel: string;
  cancelShortLabel: string;
  progressLabel: string;
  onDownload: () => void;
  onCancel: () => void;
}) {
  const size = showLabel ? "sm" : "icon-sm";
  const className = showLabel ? "shrink-0 gap-1 text-xs" : "text-muted-foreground";

  if (!state) {
    return (
      <Button
        type="button"
        variant="ghost"
        size={size}
        className={className}
        aria-label={downloadLabel}
        title={downloadLabel}
        disabled={pending}
        onClick={onDownload}
      >
        {pending ? (
          <LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
        ) : (
          <DownloadIcon className="size-4" aria-hidden="true" />
        )}
        {showLabel ? downloadShortLabel : null}
      </Button>
    );
  }

  const fraction =
    state.status === "downloading" ? state.fraction : state.status === "finishing" ? 1 : null;
  const percent = fraction === null ? null : Math.round(fraction * 100);
  const indeterminate = percent === null;

  return (
    <Button
      type="button"
      variant="ghost"
      size={size}
      className={cn(className, "tabular-nums")}
      aria-label={cancelLabel}
      title={cancelLabel}
      onClick={onCancel}
    >
      <span
        role="progressbar"
        aria-label={progressLabel}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={percent === null ? undefined : `${percent}%`}
        className="relative grid size-[18px] shrink-0 place-items-center"
      >
        <svg viewBox="0 0 20 20" className="size-[18px]" aria-hidden="true">
          <circle
            cx="10"
            cy="10"
            r="8"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            className="opacity-20"
          />
          <circle
            cx="10"
            cy="10"
            r="8"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            pathLength={100}
            strokeDasharray={indeterminate ? "22 78" : "100 100"}
            strokeDashoffset={indeterminate ? 0 : 100 - (percent ?? 0)}
            className={cn(
              "origin-center -rotate-90 text-primary",
              indeterminate
                ? "animate-spin [animation-duration:1.4s]"
                : "transition-[stroke-dashoffset] duration-150",
            )}
          />
        </svg>
        <SquareIcon
          className="absolute size-2 fill-current opacity-0 transition-opacity group-hover/button:opacity-100 group-focus-visible/button:opacity-100"
          aria-hidden="true"
        />
      </span>
      {showLabel ? cancelShortLabel : null}
    </Button>
  );
}
