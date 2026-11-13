import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { desktopLogout, isDesktopSession } from "../../lib/desktop";
import { useCollections } from "../../lib/electric/collections-context";
import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { getOidcClient } from "../../lib/oidc/session";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { disablePush } from "../../lib/pwa/push";
import { UnbackedKeysLogoutDialog } from "../e2ee/UnbackedKeysLogoutDialog";
import { Button } from "../ui/button";

/**
 * 設定画面のログアウト。OIDC セッションの破棄（トークンのクリアとログアウト画面への遷移）を行う。
 * 表示するかどうかは呼び出し側が決める（dev の OIDC 未設定では出さない）。
 */
export function LogoutButton() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const { clearUserData } = useOfflineStatus();
  const e2ee = useE2ee();
  // 鍵バックアップに送れていない鍵の数。0 より大きければ、ログアウトの前に確かめる。
  // null は閉じている、-1 は数えられなかった。
  const [unbackedKeys, setUnbackedKeys] = useState<number | null>(null);
  const [loggingOut, setLoggingOut] = useState(false);

  const handleLogout = async () => {
    // 暗号ストアを消すと、この端末にだけある鍵を失う。送れていない鍵があれば先に確かめる
    // （docs/e2ee.md の「ログアウトと端末の削除」）。
    // 数えられなかったら、送れていない鍵がないとは扱わず、分からないまま確かめる。
    const pending = await e2ee.pendingBackupCount().catch(() => -1);
    if (pending !== 0) {
      setUnbackedKeys(pending);
      return;
    }
    await performLogout();
  };

  const clearEncryptionData = () =>
    e2ee.clearForLogout().catch(() => toast.error(t("e2ee.logout.deleteFailed")));

  const performLogout = async () => {
    // デスクトップ版では Rust 側がトークンを持つ。消すとログイン画面へ戻る。
    if (isDesktopSession()) {
      await clearEncryptionData();
      await clearUserData().catch(() => toast.error(t("offline.deleteFailed")));
      await desktopLogout();
      return;
    }
    const client = getOidcClient(config);
    if (!client) {
      e2ee.cancelLogout();
      return;
    }
    try {
      await disablePush(config);
    } catch {
      toast.error(t("pwa.logoutError"));
      // ログアウトを続けないので、件数の確認で奪ったロックを返す。
      e2ee.cancelLogout();
      return;
    }
    await clearEncryptionData();
    try {
      await clearUserData();
    } catch {
      // 端末データを消せなくてもログアウトは続ける。lastUser は消えているため
      // 次のオフライン起動で前のユーザーの DB は選ばれず、削除は再試行される。
      toast.error(t("offline.deleteFailed"));
    }
    await client.logout();
  };

  return (
    <>
      <Button variant="destructive" onClick={() => void handleLogout()}>
        {t("auth.logout")}
      </Button>
      <UnbackedKeysLogoutDialog
        count={unbackedKeys}
        pending={loggingOut}
        onCancel={() => {
          setUnbackedKeys(null);
          e2ee.cancelLogout();
        }}
        onConfirm={async () => {
          setLoggingOut(true);
          try {
            await performLogout();
          } finally {
            setLoggingOut(false);
            setUnbackedKeys(null);
          }
        }}
      />
    </>
  );
}
