import { useShell } from "./shell-context";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { cn } from "cn";
import {
  ActivityIcon,
  HashIcon,
  HomeIcon,
  MessageCircleMoreIcon,
  type LucideIcon,
  SettingsIcon,
} from "lucide-react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { useCollections } from "../../lib/electric/collections-context";
import { isDesktopSession } from "../../lib/desktop";
import { accountProfileUrl } from "../../lib/oidc/accountUrl";
import { AppAvatar } from "../chat/AppIdentity";
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
  const usersQuery = useUsers();
  const me = (usersQuery.data ?? []).find((user) => user.id === currentUserId);
  // プロフィールは AuthZ で編集する。保存後にこの画面へ戻れるよう、現在の origin を渡す。
  // デスクトップ版では AuthZ が既定のブラウザーで開き、戻り先もブラウザーの Web 版になるため付けない。
  const profileUrl = config.oidc
    ? accountProfileUrl(config.oidc.issuer, isDesktopSession() ? null : window.location.origin)
    : null;
  const meLabel = me ? me.display_name || me.user_name : (currentUserId ?? "?");

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
        {profileUrl ? (
          <a
            href={profileUrl}
            aria-label={`${meLabel} - ${t("auth.profile")}`}
            title={`${meLabel} - ${t("auth.profile")}`}
            className="rounded-full outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            <AppAvatar label={meLabel} src={me?.avatar_url} size="sm" />
          </a>
        ) : (
          <AppAvatar label={meLabel} src={me?.avatar_url} size="sm" title={meLabel} />
        )}
      </div>
    </nav>
  );
}
