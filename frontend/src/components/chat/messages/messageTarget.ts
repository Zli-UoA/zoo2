import { useCallback, useEffect, useState, type MutableRefObject } from "react";
import { subscribeMessageJump } from "./messageLink";

/**
 * 対象の行の強調（点滅 → 点灯 → フェードアウト）の長さ。styles.css の `message-target-flash` と同じ値にする。
 * 0〜0.5 秒に 3 回点滅し、1.5 秒点灯を保ち、0.5 秒でフェードアウトする。
 */
export const MESSAGE_TARGET_FLASH_MS = 2500;

/**
 * 表示中の対象メッセージへのリンクをもう一度押したときの再スクロールの依頼を受ける。
 * URL が変わらないので、ルーターは何も変えず、一覧も再スクロールしない。依頼の ID が自分の対象と同じときだけ、
 * スクロール済みの印を消して、数を増やす。スクロールの効果は、この数を依存に入れて走らせ直す。
 */
export function useMessageJumpRequest(
  targetMessageId: string | undefined,
  scrolledTarget: MutableRefObject<string | undefined>,
): number {
  const [jumpRequest, setJumpRequest] = useState(0);
  useEffect(() => {
    if (!targetMessageId) return;
    return subscribeMessageJump((messageId) => {
      if (messageId !== targetMessageId) return;
      scrolledTarget.current = undefined;
      setJumpRequest((count) => count + 1);
    });
  }, [targetMessageId, scrolledTarget]);
  return jumpRequest;
}

/** いま強調を再生している行。`run` は再生のたびに増え、行はこれが変わるたびに最初から再生する。 */
export interface MessageTargetFlash {
  id: string;
  run: number;
}

/**
 * 対象の行へスクロールしたときに、その行の強調を再生する。`play` を呼ぶたびに最初から再生し直し、
 * 再生が終わる頃に自分で消える（行が一覧から外れて、再び描かれたときに、古い強調をまた再生しないため）。
 */
export function useMessageTargetFlash() {
  const [flash, setFlash] = useState<MessageTargetFlash | null>(null);
  const play = useCallback((id: string) => {
    setFlash((previous) => ({ id, run: (previous?.run ?? 0) + 1 }));
  }, []);
  useEffect(() => {
    if (!flash) return;
    const timer = setTimeout(() => setFlash(null), MESSAGE_TARGET_FLASH_MS + 200);
    return () => clearTimeout(timer);
  }, [flash]);
  /** その行に渡す `highlightRun`。強調しない行は `undefined`。 */
  const runFor = useCallback(
    (id: string): number | undefined => (flash?.id === id ? flash.run : undefined),
    [flash],
  );
  return { play, runFor };
}
