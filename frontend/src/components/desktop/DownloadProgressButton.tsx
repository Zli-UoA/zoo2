import { DownloadIcon, LoaderCircleIcon, SquareIcon } from "lucide-react";
import type { CSSProperties } from "react";

import type { DesktopDownloadState } from "../../lib/desktopDownloads";
import { Button } from "../ui/button";
import { cn } from "cn";

/** 初期表示から DOM に置き、進捗の変化だけを控えめに読み上げる領域。 */
function ProgressAnnouncement({ text }: { text: string }) {
  return (
    <span role="status" aria-live="polite" aria-atomic="true" className="sr-only">
      {text}
    </span>
  );
}

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
  finishingLabel,
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
  /** 名前への置き換え中。もう中止できないので、ボタンを無効にして伝える。 */
  finishingLabel: string;
  progressLabel: string;
  onDownload: () => void;
  onCancel: () => void;
}) {
  // ラベルを出す場合は xs の gap・文字サイズが元の指定と同じになる。
  const size = showLabel ? "xs" : "icon-sm";
  const className = "shrink-0";

  if (!state) {
    return (
      <>
        <Button
          type="button"
          variant="ghost"
          size={size}
          className={className}
          aria-label={downloadLabel}
          title={downloadLabel}
          disabled={pending}
          onClick={(event) => {
            // 中止のダブルクリックの 2 回目は、開始ボタンに戻った後に届く。ここで開始すると、
            // 中止した直後に保存が再び始まってしまう。
            if (event.detail > 1) return;
            onDownload();
          }}
        >
          {pending ? (
            <LoaderCircleIcon
              className="size-4 animate-spin text-muted-foreground motion-reduce:animate-none"
              aria-hidden="true"
            />
          ) : (
            <DownloadIcon className="size-4 text-muted-foreground" aria-hidden="true" />
          )}
          {showLabel ? downloadShortLabel : null}
        </Button>
        <ProgressAnnouncement text="" />
      </>
    );
  }

  const finishing = state.status === "finishing";
  const fraction =
    state.status === "downloading" ? state.fraction : state.status === "finishing" ? 1 : null;
  const percent = fraction === null ? null : Math.round(fraction * 100);
  const indeterminate = percent === null;
  // 円は `<button>` の中にあり、支援技術にはボタンの名前だけが伝わる。進み具合は
  // ボタンの `aria-label` に入れる（`role="progressbar"` は読み上げられない）。
  const label = finishing
    ? finishingLabel
    : percent === null
      ? cancelLabel
      : `${cancelLabel} ${percent}%`;
  // ライブリージョンは 10% 刻みで更新する。1% ごとに読み上げると、保存中ずっと話し続ける。
  const milestone = percent === null ? null : Math.floor(percent / 10) * 10;
  const announcement = finishing
    ? finishingLabel
    : milestone === null
      ? progressLabel
      : `${progressLabel} ${milestone}%`;

  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size={size}
        className={className}
        aria-label={label}
        title={label}
        disabled={finishing}
        onClick={(event) => {
          // 開始ボタンのダブルクリックの 2 回目は、開始直後の円に当たる。ここで中止すると、
          // 保存ダイアログで保存先を選んだ直後に、通知なく取り消してしまう。
          if (event.detail > 1) return;
          onCancel();
        }}
      >
        <span
          role="progressbar"
          aria-hidden="true"
          aria-label={progressLabel}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent ?? undefined}
          aria-valuetext={percent === null ? undefined : `${percent}%`}
          className="relative grid size-[18px] shrink-0 place-items-center text-muted-foreground"
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
                  ? "animate-spin motion-reduce:animate-none"
                  : "transition-all duration-150 motion-reduce:transition-none",
              )}
              style={
                indeterminate
                  ? ({ "--animate-spin": "spin 1.4s linear infinite" } as CSSProperties)
                  : undefined
              }
            />
          </svg>
          <SquareIcon
            className="absolute size-2 fill-current opacity-0 transition-opacity group-hover/button:opacity-100 group-focus-visible/button:opacity-100 motion-reduce:transition-none"
            aria-hidden="true"
          />
        </span>
        {showLabel ? (
          <span className="tabular-nums">{finishing ? finishingLabel : cancelShortLabel}</span>
        ) : null}
      </Button>
      <ProgressAnnouncement text={announcement} />
    </>
  );
}
