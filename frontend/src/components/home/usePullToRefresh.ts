import { useCallback, useRef, useState, type TouchEvent } from "react";

const PULL_THRESHOLD = 60;
const MAX_PULL = 100;

/**
 * スクロール先頭での引っ張り操作だけを扱う状態。
 * 実際の再取得は `onRefresh` に任せ、完了時に表示を戻す。
 */
export function usePullToRefresh(onRefresh: () => Promise<unknown>, getScrollTop: () => number) {
  const [pull, setPull] = useState(0);
  const [pulling, setPulling] = useState(false);
  const touchStart = useRef<number | null>(null);

  const onTouchStart = useCallback(
    (event: TouchEvent<HTMLElement>) => {
      touchStart.current =
        event.touches.length === 1 && getScrollTop() <= 0 ? event.touches[0].clientY : null;
    },
    [getScrollTop],
  );

  const onTouchMove = useCallback(
    (event: TouchEvent<HTMLElement>) => {
      if (touchStart.current === null || pulling) return;
      if (getScrollTop() > 0 || event.touches.length !== 1) {
        touchStart.current = null;
        setPull(0);
        return;
      }
      setPull(Math.min(MAX_PULL, Math.max(0, (event.touches[0].clientY - touchStart.current) / 2)));
    },
    [getScrollTop, pulling],
  );

  const onTouchCancel = useCallback(() => {
    touchStart.current = null;
    setPull(0);
  }, []);

  const onTouchEnd = useCallback(() => {
    touchStart.current = null;
    if (pull >= PULL_THRESHOLD && !pulling) {
      setPulling(true);
      void onRefresh().finally(() => {
        setPulling(false);
        setPull(0);
      });
    } else setPull(0);
  }, [pull, pulling, onRefresh]);

  return { pull, pulling, onTouchStart, onTouchMove, onTouchCancel, onTouchEnd };
}
