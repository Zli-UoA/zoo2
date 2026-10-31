import { toast } from "sonner";
import { useShell } from "./shell-context";
import { disablePush } from "../../lib/pwa/push";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ActivityIcon,
  HashIcon,
  LogOutIcon,
  MessageCircleMoreIcon,
  type LucideIcon,
  SettingsIcon,
} from "lucide-react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { useCollections } from "../../lib/electric/collections-context";
import { getOidcClient } from "../../lib/oidc/session";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import { Separator } from "../ui/separator";
import { AppDialogs } from "../chat/AppsDialog";
import { NotificationsButton } from "../chat/NotificationsButton";
import { initials } from "../chat/format";

function RailLink({
  to,
  icon: Icon,
  label,
  exact = false,
}: {
  to: "/" | "/bubble" | "/realtime";
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
  const usersQuery = useUsers();
  const me = (usersQuery.data ?? []).find((user) => user.id === currentUserId);
  const meLabel = me ? me.display_name || me.user_name : (currentUserId ?? "?");

  // OIDC セッションの破棄（トークンのクリアとログアウト画面への遷移）。
  // dev（OIDC 未設定）ではログアウト UI を出さない。
  const handleLogout = async () => {
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
        "flex w-14 shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-3",
        "md:w-16",
      )}
    >
      <img
        src="/icons/zoo.svg"
        alt="Zoo"
        width={36}
        height={36}
        className="mb-1 size-9 rounded-xl"
      />
      <Separator className="my-1 w-6" />
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
        <Avatar size="sm" title={meLabel}>
          <AvatarFallback>{initials(meLabel)}</AvatarFallback>
        </Avatar>
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
      </div>
    </nav>
  );
}
