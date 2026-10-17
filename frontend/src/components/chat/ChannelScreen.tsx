import { useMemo } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useActiveChannels, useChannelMembers } from "../../hooks/useRealtime";
import { MessagePane } from "../MessagePane";
import { Skeleton } from "../ui/skeleton";
import { ChannelHeader } from "./ChannelHeader";

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
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const channelsQuery = useActiveChannels();
  const membersQuery = useChannelMembers(channelId);
  const channels = channelsQuery.data ?? [];
  const channel = useMemo(
    () => channels.find((candidate) => candidate.id === channelId) ?? null,
    [channels, channelId],
  );

  const members = membersQuery.data ?? [];
  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;
  // 非公開チャンネルは参加者だけにメッセージを表示する。direct モードの Shape は
  // 認可せず全件配信するため、UI 側でも隠す（proxy 切替後も同じ）。
  const canRead = channel ? !channel.is_private || isMember : false;
  // 非公開チャンネルの参加判定は、メンバー一覧と現在ユーザーの両方が
  // 解決するまで保留する（解決前に「非公開です」を出すのを避ける）。
  const membershipPending = !!channel?.is_private && (membersQuery.isLoading || currentUserLoading);

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
        memberCount={canRead ? (membersQuery.isLoading ? null : members.length) : undefined}
      />
      <div className="flex min-h-0 flex-1 flex-col">
        {membershipPending ? (
          <div className="flex flex-1 items-center justify-center">
            <p className="text-sm text-muted-foreground">読み込み中…</p>
          </div>
        ) : canRead ? (
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
