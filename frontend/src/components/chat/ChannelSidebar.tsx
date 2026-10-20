import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { useLiveQuery } from "@tanstack/react-db";
import { cn } from "cn";
import { HashIcon, LockIcon, PlusIcon } from "lucide-react";
import { useMemo, useState } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { getCollections } from "../../lib/electric/collections";
import type { ChannelRow } from "../../lib/electric/types";
import { SyncStatus } from "../SyncStatus";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { ScrollArea } from "../ui/scroll-area";
import { Skeleton } from "../ui/skeleton";
import { CreateChannelDialog } from "./CreateChannelDialog";

const SKELETON_ROWS = [0, 1, 2, 3, 4];

/** 左サイドバー。チャンネル一覧・作成と、現在ユーザーの参加状態を出す。 */
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
  const { channelUsers } = getCollections();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const [createOpen, setCreateOpen] = useState(false);

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
  // 解決前は参加状態を出さない（OIDC では /api/users/me の応答待ちの間、
  // 一瞬「未参加」に見えるのを避ける）。
  const hasCurrentUser = !currentUserLoading && currentUserId !== null;

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
            {loading ? "…" : channels.length}
          </span>
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("channels.createTitle")}
          onClick={() => setCreateOpen(true)}
        >
          <PlusIcon />
        </Button>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-0.5 px-2 pb-2">
          {loading ? (
            SKELETON_ROWS.map((row) => <Skeleton key={row} className="h-9 w-full rounded-lg" />)
          ) : error ? (
            <p className="px-2 py-1 text-xs text-destructive">{t("channels.loadError")}</p>
          ) : channels.length === 0 ? (
            <div className="flex flex-col items-start gap-2 px-2 py-4">
              <p className="text-sm text-muted-foreground">{t("channels.empty")}</p>
              <Button variant="outline" size="sm" onClick={() => setCreateOpen(true)}>
                <PlusIcon />
                {t("channels.createAction")}
              </Button>
            </div>
          ) : (
            channels.map((channel) => {
              const isActive = shell.selectedChannelId === channel.id;
              const isJoined = joinedChannelIds.has(channel.id);
              return (
                <Link
                  key={channel.id}
                  to="/channels/$channelId"
                  params={{ channelId: channel.id }}
                  onClick={() => shell.showChat()}
                  className={cn(
                    "group flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition-colors",
                    "focus-visible:ring-3 focus-visible:ring-ring/50 outline-none",
                    isActive
                      ? "bg-brand-muted font-medium text-brand-muted-foreground"
                      : "text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                  )}
                >
                  {channel.is_private ? (
                    <LockIcon className="size-3.5 shrink-0" aria-hidden="true" />
                  ) : (
                    <HashIcon className="size-4 shrink-0 opacity-60" aria-hidden="true" />
                  )}
                  <span className="min-w-0 flex-1 truncate">{channel.channel_name}</span>
                  {hasCurrentUser ? (
                    isJoined ? (
                      <span
                        role="img"
                        title={t("members.joined")}
                        aria-label={t("members.joined")}
                        className="size-1.5 shrink-0 rounded-full bg-brand"
                      />
                    ) : (
                      <span className="shrink-0 text-[0.65rem] text-muted-foreground/70">
                        {t("members.notJoined")}
                      </span>
                    )
                  ) : null}
                </Link>
              );
            })
          )}
        </div>
      </ScrollArea>

      {/* 接続状態の表示は開発時の確認用。本番ビルドでは出さない。 */}
      {import.meta.env.DEV ? (
        <div className="border-t px-3 py-2">
          <SyncStatus />
        </div>
      ) : null}

      <CreateChannelDialog open={createOpen} onOpenChange={setCreateOpen} />
    </aside>
  );
}
