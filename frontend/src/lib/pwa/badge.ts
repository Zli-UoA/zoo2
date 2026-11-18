type BadgeNavigator = Navigator & {
  setAppBadge?: (count?: number) => Promise<void>;
  clearAppBadge?: () => Promise<void>;
};

/**
 * インストール済み PWA のアイコンに未読数を出す（App Badging API）。
 * 未対応の環境や、インストールしていないブラウザーでは何もしない。`count <= 0` で消す。
 */
export function setAppBadge(count: number): void {
  if (typeof navigator === "undefined") return;
  const nav = navigator as BadgeNavigator;
  try {
    const result =
      count > 0 ? nav.setAppBadge?.(Math.floor(count)) : (nav.clearAppBadge?.() ?? undefined);
    void result?.catch(() => {});
  } catch {
    // 表示できないだけなので無視する。
  }
}
