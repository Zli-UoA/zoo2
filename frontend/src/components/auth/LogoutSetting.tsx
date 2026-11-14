import { useEffect, useRef, useState } from "react";
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
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";

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
  // 終わっていない失効があるときの、ログアウトの確認（#362）。
  const [revocationWarning, setRevocationWarning] = useState(false);
  // 確認でダイアログが閉じたときに、ログアウトを取りやめたと扱わない。
  const revocationConfirmed = useRef(false);
  // 件数の確認で奪った鍵の受信処理の停止を、まだ返していない間は true。
  // 設定画面を離れて unmount されたら、未確定のログアウトを取りやめて停止を戻す。
  const holdsLock = useRef(false);
  const mounted = useRef(false);
  const e2eeRef = useRef(e2ee);
  e2eeRef.current = e2ee;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (holdsLock.current) {
        holdsLock.current = false;
        e2eeRef.current.cancelLogout();
      }
    };
  }, []);

  const handleLogout = async () => {
    holdsLock.current = true;
    // 失効の進み具合は暗号ストアにしかない。消す前に確かめる（docs/e2ee.md の「失効の手順」）。
    const revoking = await e2ee.revocationPending().catch(() => false);
    if (!mounted.current) return;
    if (revoking) {
      setRevocationWarning(true);
      return;
    }
    await continueLogout();
  };

  const continueLogout = async () => {
    // 暗号ストアを消すと、この端末にだけある鍵を失う。送れていない鍵があれば先に確かめる
    // （docs/e2ee.md の「ログアウトと端末の削除」）。
    // 数えられなかったら、送れていない鍵がないとは扱わず、分からないまま確かめる。
    holdsLock.current = true;
    const pending = await e2ee.pendingBackupCount().catch(() => -1);
    // 確認中に画面を離れていたら、遅れて返った結果は捨てる（停止は unmount で戻している）。
    if (!mounted.current) return;
    if (pending !== 0) {
      setUnbackedKeys(pending);
      return;
    }
    await performLogout();
  };

  const clearEncryptionData = () =>
    e2ee.clearForLogout().catch(() => toast.error(t("e2ee.logout.deleteFailed")));

  const performLogout = async () => {
    // ここからは performLogout が停止の扱い（続行か取りやめ）を決める。
    holdsLock.current = false;
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
      <ConfirmActionDialog
        open={revocationWarning}
        onOpenChange={(open) => {
          if (open) return;
          setRevocationWarning(false);
          if (!revocationConfirmed.current) {
            holdsLock.current = false;
            e2ee.cancelLogout();
          }
        }}
        title={t("e2ee.logout.revocationTitle")}
        description={t("e2ee.logout.revocationDescription")}
        confirmLabel={t("e2ee.logout.revocationConfirm")}
        onConfirm={async () => {
          revocationConfirmed.current = true;
          setRevocationWarning(false);
          try {
            await continueLogout();
          } finally {
            revocationConfirmed.current = false;
          }
        }}
      />
      <UnbackedKeysLogoutDialog
        count={unbackedKeys}
        pending={loggingOut}
        onCancel={() => {
          holdsLock.current = false;
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
