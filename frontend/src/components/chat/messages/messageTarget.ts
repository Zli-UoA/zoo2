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
  /** 再生を始めた時刻（`Date.now()`）。終わったあとの行に、古い強調を渡さないために使う。 */
  startedAt: number;
}

/**
 * 対象の行へスクロールしたときに、その行の強調を再生する。`play` を呼ぶたびに最初から再生し直す。
 *
 * 再生が終わったあとに、行が一覧から外れて再び描かれても、古い強調を再生し直さない。`runFor` が
 * 経過時間を直接見て、終わった強調には `undefined` を返す。状態を消すタイマーは、メモリを空けて
 * 再描画するためだけのもので、タイマーが遅れても、終わったあとの行には強調が渡らない。
 * （再生の途中で行が外れて戻ったときは、最初から再生し直す。）
 */
export function useMessageTargetFlash() {
  const [flash, setFlash] = useState<MessageTargetFlash | null>(null);
  const play = useCallback((id: string) => {
    setFlash((previous) => ({ id, run: (previous?.run ?? 0) + 1, startedAt: Date.now() }));
  }, []);
  useEffect(() => {
    if (!flash) return;
    const timer = setTimeout(() => setFlash(null), MESSAGE_TARGET_FLASH_MS);
    return () => clearTimeout(timer);
  }, [flash]);
  /** その行に渡す `highlightRun`。強調しない行と、再生が終わった行は `undefined`。 */
  const runFor = useCallback(
    (id: string): number | undefined =>
      flash?.id === id && Date.now() - flash.startedAt < MESSAGE_TARGET_FLASH_MS
        ? flash.run
        : undefined,
    [flash],
  );
  return { play, runFor };
}
