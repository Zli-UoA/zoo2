import { useEffect, useState } from "react";

/** Shape が無言で止まったときにスケルトン無限表示を避けるための上限。 */
const LOADING_TIMEOUT_MS = 8_000;

/** 読み込みが長時間終わらないときだけ true にする。 */
export function useLoadingTimeout(active: boolean, timeoutMs = LOADING_TIMEOUT_MS): boolean {
  const [timedOut, setTimedOut] = useState(false);
  useEffect(() => {
    if (!active) {
      setTimedOut(false);
      return;
    }
    const timer = setTimeout(() => setTimedOut(true), timeoutMs);
    return () => clearTimeout(timer);
  }, [active, timeoutMs]);
  return active && timedOut;
}
