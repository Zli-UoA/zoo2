/**
 * ホーム画面や Dock に追加したアプリ（PWA）として開いているか。iOS の Safari は `display-mode` を
 * 返さないことがあるので、`navigator.standalone` も見る。履歴の扱い（`history.ts`）と端末名
 * （`lib/e2ee/displayName.ts`）で、同じ判定を使う。
 */
export function isStandaloneDisplay(
  view: {
    matchMedia?: (query: string) => { matches: boolean };
    navigator?: unknown;
  } = globalThis,
): boolean {
  try {
    if (view.matchMedia?.("(display-mode: standalone)").matches) return true;
  } catch {
    // matchMedia がなければ、下の判定だけを使う。
  }
  return (view.navigator as { standalone?: boolean } | undefined)?.standalone === true;
}
