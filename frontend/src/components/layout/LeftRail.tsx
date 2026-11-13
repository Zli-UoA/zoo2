import { toast } from "sonner";
import { useShell } from "./shell-context";
import { disablePush } from "../../lib/pwa/push";
import { useTranslation } from "react-i18next";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ActivityIcon,
  HashIcon,
  HomeIcon,
  LogOutIcon,
  MessageCircleMoreIcon,
  type LucideIcon,
  SettingsIcon,
} from "lucide-react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { useCollections } from "../../lib/electric/collections-context";
import { desktopLogout, isDesktopSession } from "../../lib/desktop";
import { getOidcClient } from "../../lib/oidc/session";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { UnbackedKeysLogoutDialog } from "../e2ee/UnbackedKeysLogoutDialog";
import { AppAvatar } from "../chat/AppIdentity";
import { Button } from "../ui/button";
import { Separator } from "../ui/separator";
import { AppDialogs } from "../chat/AppsDialog";
import { NotificationsButton } from "../chat/NotificationsButton";
import { PaneHeader } from "./PaneHeader";

function RailLink({
  to,
  icon: Icon,
  label,
  exact = false,
}: {
  to: "/home" | "/" | "/bubble" | "/realtime";
  icon: LucideIcon;
  label: string;
  exact?: boolean;
}) {
  return (
    <Link
      to={to}
      activeOptions={{ exact }}
      aria-label={label}
      title={label}
      className="relative flex size-10 items-center justify-center rounded-xl text-muted-foreground transition-colors outline-none hover:bg-accent hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[status=active]:bg-brand-muted data-[status=active]:text-brand-muted-foreground"
    >
      <Icon className="size-5" aria-hidden="true" />
    </Link>
  );
}

/**
 * 最左の固定レール。アプリの印と主要ナビ、テーマ切替、現在ユーザーを置く。
 * 画面幅に関わらず表示し、狭幅でも操作の入口を保つ。
 */
export function LeftRail() {
  const { t } = useTranslation();
  const shell = useShell();
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const { clearUserData } = useOfflineStatus();
  const e2ee = useE2ee();
  // 鍵バックアップに送れていない鍵の数。0 より大きければ、ログアウトの前に確かめる。
  const [unbackedKeys, setUnbackedKeys] = useState(0);
  const [loggingOut, setLoggingOut] = useState(false);
  const usersQuery = useUsers();
  const me = (usersQuery.data ?? []).find((user) => user.id === currentUserId);
  const meLabel = me ? me.display_name || me.user_name : (currentUserId ?? "?");

  // OIDC セッションの破棄（トークンのクリアとログアウト画面への遷移）。
  // dev（OIDC 未設定）ではログアウト UI を出さない。
  const handleLogout = async () => {
    // 暗号ストアを消すと、この端末にだけある鍵を失う。送れていない鍵があれば先に確かめる
    // （docs/e2ee.md の「ログアウトと端末の削除」）。
    const pending = await e2ee.pendingBackupCount().catch(() => 0);
    if (pending > 0) {
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
      return;
    }
    try {
      await disablePush(config);
    } catch {
      toast.error(t("pwa.logoutError"));
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
    <nav
      aria-label={t("nav.main")}
      className={cn(
        "flex w-14 shrink-0 flex-col items-center gap-1 border-r bg-sidebar pb-3",
        "md:w-16",
      )}
    >
      <PaneHeader className="mb-1 w-full justify-center px-0">
        <img src="/icons/zoo.svg" alt="Zoo" width={36} height={36} className="size-9 rounded-xl" />
      </PaneHeader>
      <RailLink to="/home" icon={HomeIcon} label={t("home.title")} exact />
      <RailLink to="/" icon={HashIcon} label={t("channels.title")} exact />
      <NotificationsButton />
      <RailLink to="/bubble" icon={MessageCircleMoreIcon} label={t("bubbleChat.title")} exact />
      <AppDialogs />
      {/* 同期の状態（/realtime）は開発時の確認用。本番ビルドでは出さない。 */}
      {import.meta.env.DEV ? (
        <RailLink to="/realtime" icon={ActivityIcon} label={t("sync.title")} exact />
      ) : null}

      <div className="mt-auto flex flex-col items-center gap-2">
        <Link
          to="/settings"
          search={{ channel: shell.selectedChannelId ?? undefined }}
          aria-label={t("settings.zooTitle")}
          title={t("settings.zooTitle")}
          className="flex size-10 items-center justify-center rounded-xl text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[status=active]:bg-brand-muted data-[status=active]:text-brand-muted-foreground"
        >
          <SettingsIcon className="size-5" aria-hidden="true" />
        </Link>
        <Separator className="w-6" />
        <AppAvatar label={meLabel} src={me?.avatar_url} size="sm" title={meLabel} />
        {config.oidc ? (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("auth.logout")}
            title={t("auth.logout")}
            onClick={handleLogout}
          >
            <LogOutIcon />
          </Button>
        ) : null}
        <UnbackedKeysLogoutDialog
          count={unbackedKeys}
          pending={loggingOut}
          onCancel={() => setUnbackedKeys(0)}
          onConfirm={async () => {
            setLoggingOut(true);
            try {
              await performLogout();
            } finally {
              setLoggingOut(false);
              setUnbackedKeys(0);
            }
          }}
        />
      </div>
    </nav>
  );
}
