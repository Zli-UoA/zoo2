import { useMemo } from "react";

import { useActiveChannels, useChannelAccess } from "../../hooks/useRealtime";
import { Skeleton } from "../ui/skeleton";
import { ChannelHeader } from "./ChannelHeader";
import { MessagePane } from "./messages/MessagePane";

const LOADING_ROWS = [0, 1, 2, 3];

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
  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];
  const channel = useMemo(
    () => channels.find((candidate) => candidate.id === channelId) ?? null,
    [channels, channelId],
  );
  // 権限判定（閲覧可否・参加状態・保留中）は `useChannelAccess` に集約する。
  const access = useChannelAccess(channel);

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
          {channelsQuery.isError
            ? "チャンネル取得に失敗しました。"
            : "チャンネルが見つかりません。"}
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
            <p className="text-sm text-muted-foreground">読み込み中…</p>
          </div>
        ) : access.canRead ? (
          <MessagePane channelId={channelId} />
        ) : (
          <div className="flex flex-1 items-center justify-center p-6">
            <p className="max-w-sm text-center text-sm text-muted-foreground">
              このチャンネルは非公開です。招待されるとメッセージを表示できます。
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
