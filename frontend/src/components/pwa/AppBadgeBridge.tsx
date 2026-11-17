import { useEffect } from "react";

import { useUnifiedNotifications } from "../../hooks/useUnifiedNotifications";
import { isDesktopSession } from "../../lib/desktop";
import { setAppBadge } from "../../lib/pwa/badge";

/**
 * インストール済み PWA のアイコンに、未読数のバッジを出す。
 * デスクトップ版（Tauri）は DesktopBridge が OS のバッジを出すので、何もしない。
 */
export function AppBadgeBridge() {
  if (isDesktopSession()) return null;
  return <AppBadgeBridgeInner />;
}

function AppBadgeBridgeInner() {
  const { unreadCount } = useUnifiedNotifications();
  useEffect(() => {
    setAppBadge(unreadCount);
    // 別のタブや、閉じている間の push で変わった値を、戻ってきたときに合わせ直す。
    const resync = () => {
      if (document.visibilityState === "visible") setAppBadge(unreadCount);
    };
    document.addEventListener("visibilitychange", resync);
    return () => document.removeEventListener("visibilitychange", resync);
  }, [unreadCount]);
  return null;
}
