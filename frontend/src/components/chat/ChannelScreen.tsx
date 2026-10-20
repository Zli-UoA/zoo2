import { useTranslation } from "react-i18next";
import { ArchiveIcon } from "lucide-react";
import { useMemo } from "react";

import { useActiveChannels, useChannelAccess, useMessageFeed } from "../../hooks/useRealtime";
import { Skeleton } from "../ui/skeleton";
import { ChannelHeader } from "./ChannelHeader";
import { MessageList } from "./messages/MessageList";
import { MessagePane } from "./messages/MessagePane";

const LOADING_ROWS = [0, 1, 2, 3];

function noop() {}

function ChannelLoading() {
  return (
    <>
      <div className="flex items-center gap-3 border-b px-4 py-2.5">
        <Skeleton className="size-8 rounded-lg" />
        <div className="flex flex-1 flex-col gap-1.5">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-3 w-24" />
        </div>
      </div>
      <div className="flex flex-1 flex-col gap-3 p-4">
        {LOADING_ROWS.map((row) => (
          <Skeleton key={row} className="h-14 w-full rounded-lg" />
        ))}
      </div>
    </>
  );
}

/** チャンネルルート。ヘッダとメッセージ（非公開は参加者だけ）を出す。 */
export function ChannelScreen({ channelId }: { channelId: string }) {
  const { t } = useTranslation();
  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];
  const channel = useMemo(
    () => channels.find((candidate) => candidate.id === channelId) ?? null,
    [channels, channelId],
  );
  // 権限判定（閲覧可否・参加状態・保留中）は `useChannelAccess` に集約する。
  const access = useChannelAccess(channel);
  // アーカイブ済みは Composer を出さず、MessageList を読み取り専用で描く。
  const archivedFeedQuery = useMessageFeed(
    channel && access.isArchived && access.canRead ? channelId : null,
  );

  if (!channel) {
    if (channelsQuery.isLoading) {
      return (
        <div className="flex h-full min-h-0 flex-col">
          <ChannelLoading />
        </div>
      );
    }
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p
          className={
            channelsQuery.isError ? "text-sm text-destructive" : "text-sm text-muted-foreground"
          }
        >
          {channelsQuery.isError ? t("channels.loadError") : t("channels.notFound")}
        </p>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ChannelHeader
        channel={channel}
        memberCount={access.canRead ? (access.isLoading ? null : access.members.length) : undefined}
      />
      <div className="flex min-h-0 flex-1 flex-col">
        {access.isMembershipPending ? (
          <div className="flex flex-1 items-center justify-center">
            <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
          </div>
        ) : access.isArchived ? (
          access.canRead ? (
            <div className="flex min-h-0 flex-1 flex-col">
              <div className="flex items-start gap-2 border-b bg-muted/40 px-4 py-2 text-xs text-muted-foreground">
                <ArchiveIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
                <span>{t("moderation.archivedReadOnly")}</span>
              </div>
              <MessageList
                key={`archived-${channelId}`}
                messages={archivedFeedQuery.data ?? []}
                isLoading={archivedFeedQuery.isLoading}
                isError={archivedFeedQuery.isError}
                currentUserId={null}
                editingId={null}
                editingText=""
                onEditingTextChange={noop}
                onStartEdit={noop}
                onCancelEdit={noop}
                onSaveEdit={noop}
                onRequestDelete={noop}
              />
            </div>
          ) : (
            <div className="flex flex-1 items-center justify-center p-6">
              <p className="max-w-sm text-center text-sm text-muted-foreground">
                {t("channels.privateMessages")}
              </p>
            </div>
          )
        ) : access.canRead ? (
          <MessagePane channelId={channelId} />
        ) : (
          <div className="flex flex-1 items-center justify-center p-6">
            <p className="max-w-sm text-center text-sm text-muted-foreground">
              {t("channels.privateMessages")}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
