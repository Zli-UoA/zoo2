import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { useLiveQuery } from "@tanstack/react-db";
import { cn } from "cn";
import { ArchiveIcon, HashIcon, LockIcon, PlusIcon, SearchIcon } from "lucide-react";
import { useMemo, useState } from "react";

import { useUnreadChannels } from "../../hooks/useUnreadChannels";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelRow } from "../../lib/electric/types";
import { SyncStatus } from "../SyncStatus";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { ScrollArea } from "../ui/scroll-area";
import { Skeleton } from "../ui/skeleton";
import { ChannelBrowserDialog } from "./ChannelBrowserDialog";
import { filterJoinedChannels } from "./channelSearch";
import { CreateChannelDialog } from "./CreateChannelDialog";
import { CreateDmDialog } from "./CreateDmDialog";
import { DmSection } from "./DmSection";

const SKELETON_ROWS = [0, 1, 2, 3, 4];

/** 左サイドバー。参加中のチャンネル一覧・作成・検索の入口を出す。 */
export function ChannelSidebar({
  channels,
  loading,
  error,
  className,
}: {
  channels: ChannelRow[];
  loading: boolean;
  error: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const { channelUsers } = useCollections();
  const usersQuery = useUsers();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const unreadChannelIds = useUnreadChannels(currentUserId);
  const [createOpen, setCreateOpen] = useState(false);
  const [browserOpen, setBrowserOpen] = useState(false);
  const [dmOpen, setDmOpen] = useState(false);

  // 一覧の各チャンネルについて現在ユーザーの参加状態を出す。`channel_user` の
  // 購読は `getCollections()` が 1 本だけ持つため、ここで直接 live query を張っても
  // 購読は増えない（`hooks/useRealtime.ts` に一覧用フックが無いための利用）。
  const membershipsQuery = useLiveQuery((q) => q.from({ channelUsers }));
  const joinedChannelIds = useMemo(() => {
    const joined = new Set<string>();
    for (const membership of membershipsQuery.data ?? []) {
      if (membership.user_id === currentUserId) {
        joined.add(membership.channel_id);
      }
    }
    return joined;
  }, [membershipsQuery.data, currentUserId]);
  // サイドバーは参加中だけを出すため、参加状態が解決するまでは一覧を確定しない
  // （解決前に空の一覧を一瞬出さない）。
  const sidebarLoading = loading || membershipsQuery.isLoading || currentUserLoading;
  const sidebarError = error || membershipsQuery.isError;
  const regularChannels = useMemo(() => channels.filter((channel) => !channel.is_dm), [channels]);
  const sidebarChannels = useMemo(
    () => filterJoinedChannels(regularChannels, joinedChannelIds),
    [regularChannels, joinedChannelIds],
  );
  const dmChannels = useMemo(
    () =>
      channels.filter(
        (channel) => channel.is_dm && !channel.deleted_at && joinedChannelIds.has(channel.id),
      ),
    [channels, joinedChannelIds],
  );

  return (
    <aside
      className={cn(
        "min-w-0 flex-1 flex-col border-r bg-sidebar text-sidebar-foreground md:w-72 md:flex-none",
        className,
      )}
    >
      <div className="flex items-center justify-between gap-2 px-3 py-3">
        <div className="flex items-baseline gap-2">
          <h1 className="font-heading text-sm font-semibold tracking-tight">
            {t("channels.title")}
          </h1>
          <span className="text-xs tabular-nums text-muted-foreground">
            {sidebarLoading ? "…" : sidebarChannels.length}
          </span>
        </div>
        <div className="flex items-center gap-0.5">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("channels.browseTitle")}
            title={t("channels.browseTitle")}
            onClick={() => setBrowserOpen(true)}
            disabled={sidebarLoading || regularChannels.length === 0}
          >
            <SearchIcon />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("channels.createTitle")}
            onClick={() => setCreateOpen(true)}
          >
            <PlusIcon />
          </Button>
        </div>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-0.5 px-2 pb-2">
          {sidebarLoading ? (
            SKELETON_ROWS.map((row) => <Skeleton key={row} className="h-9 w-full rounded-lg" />)
          ) : sidebarError ? (
            <p className="px-2 py-1 text-xs text-destructive">{t("channels.loadError")}</p>
          ) : regularChannels.length === 0 ? (
            <div className="flex flex-col items-start gap-2 px-2 py-4">
              <p className="text-sm text-muted-foreground">{t("channels.empty")}</p>
              <Button variant="outline" size="sm" onClick={() => setCreateOpen(true)}>
                <PlusIcon />
                {t("channels.createAction")}
              </Button>
            </div>
          ) : sidebarChannels.length === 0 ? (
            <div className="flex flex-col items-start gap-2 px-2 py-4">
              <p className="text-sm text-muted-foreground">{t("channels.noJoined")}</p>
              <Button variant="outline" size="sm" onClick={() => setBrowserOpen(true)}>
                <SearchIcon />
                {t("channels.browseTitle")}
              </Button>
            </div>
          ) : (
            sidebarChannels.map((channel) => {
              const isUnread = unreadChannelIds.has(channel.id);
              const isActive = shell.selectedChannelId === channel.id;
              return (
                <Link
                  key={channel.id}
                  to="/channels/$channelId"
                  params={{ channelId: channel.id }}
                  onClick={() => shell.showChat()}
                  className={cn(
                    "group flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition-colors",
                    "focus-visible:ring-3 focus-visible:ring-ring/50 outline-none",
                    isUnread && "font-semibold text-sidebar-foreground",
                    isActive
                      ? "bg-brand-muted font-medium text-brand-muted-foreground"
                      : isUnread
                        ? "hover:bg-sidebar-accent"
                        : "text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                  )}
                >
                  {channel.is_private ? (
                    <LockIcon className="size-3.5 shrink-0" aria-hidden="true" />
                  ) : (
                    <HashIcon className="size-4 shrink-0 opacity-60" aria-hidden="true" />
                  )}
                  <span
                    className={cn(
                      "min-w-0 flex-1 truncate",
                      channel.archive_at ? "text-muted-foreground" : null,
                    )}
                  >
                    {channel.channel_name}
                  </span>
                  {isUnread ? (
                    <span
                      role="img"
                      aria-label={t("channels.unread")}
                      title={t("channels.unread")}
                      className="size-2 shrink-0 rounded-full bg-brand"
                    />
                  ) : null}
                  {channel.archive_at ? (
                    <span
                      role="img"
                      title={t("moderation.archived")}
                      aria-label={t("moderation.archived")}
                      className="shrink-0 text-muted-foreground/70"
                    >
                      <ArchiveIcon className="size-3" aria-hidden="true" />
                    </span>
                  ) : null}
                </Link>
              );
            })
          )}
        </div>
        <div className="mx-3 border-t" />
        <DmSection
          channels={dmChannels}
          memberships={membershipsQuery.data ?? []}
          users={usersQuery.data ?? []}
          currentUserId={currentUserId}
          loading={sidebarLoading || usersQuery.isLoading}
          error={error || membershipsQuery.isError}
          usersError={usersQuery.isError}
          activeChannelId={shell.selectedChannelId}
          unreadChannelIds={unreadChannelIds}
          onCreate={() => setDmOpen(true)}
          onOpen={() => shell.showChat()}
          onReload={() => window.location.reload()}
        />
      </ScrollArea>

      {/* 接続状態の表示は開発時の確認用。本番ビルドでは出さない。 */}
      {import.meta.env.DEV ? (
        <div className="border-t px-3 py-2">
          <SyncStatus />
        </div>
      ) : null}

      <CreateChannelDialog open={createOpen} onOpenChange={setCreateOpen} />
      <CreateDmDialog open={dmOpen} onOpenChange={setDmOpen} />
      <ChannelBrowserDialog
        open={browserOpen}
        onOpenChange={setBrowserOpen}
        channels={regularChannels}
        joinedChannelIds={joinedChannelIds}
      />
    </aside>
  );
}
