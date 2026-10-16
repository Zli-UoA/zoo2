import { Link, createFileRoute } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import { ChannelMemberPanel } from "../../components/ChannelMemberPanel";
import { MessagePane } from "../../components/MessagePane";
import { SyncStatus } from "../../components/SyncStatus";
import { useActiveChannels, useChannelMembers, useUsers } from "../../hooks/useRealtime";
import { getCollections } from "../../lib/electric/collections";

export const Route = createFileRoute("/channels/$channelId")({ component: ChannelRoute });

function ChannelRoute() {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
  }, []);
  const { channelId } = Route.useParams();

  return (
    <main className="mx-auto max-w-4xl p-8">
      {mounted ? (
        <ChannelScreen channelId={channelId} />
      ) : (
        <p className="text-sm text-muted-foreground">ブラウザで読み込み中…</p>
      )}
    </main>
  );
}

function ChannelScreen({ channelId }: { channelId: string }) {
  const { config } = getCollections();
  const channelsQuery = useActiveChannels();
  const membersQuery = useChannelMembers(channelId);
  const usersQuery = useUsers();
  const channels = channelsQuery.data ?? [];
  const channel = useMemo(
    () => channels.find((candidate) => candidate.id === channelId) ?? null,
    [channels, channelId],
  );

  const members = membersQuery.data ?? [];
  const currentUserId = config.devUserId;
  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;
  const userNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const user of usersQuery.data ?? []) {
      map.set(user.id, user.display_name || user.user_name);
    }
    return map;
  }, [usersQuery.data]);

  // 非公開チャンネルは参加者だけに説明・メンバー・メッセージを表示する。
  // direct モードの Shape は認可せず全件配信するため、UI 側でも隠す（proxy 切替後も同様）。
  const canViewMembers = channel ? !channel.is_private || isMember : false;
  const membershipPending = !!channel?.is_private && membersQuery.isLoading;
  const canRead = channel ? !channel.is_private || isMember : false;

  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-col gap-2">
        <Link to="/" className="text-sm text-muted-foreground underline-offset-4 hover:underline">
          ← チャンネル一覧
        </Link>
        <h1 className="text-2xl font-bold">
          {channel ? (
            <>
              {channel.is_private ? "🔒 " : "# "}
              {channel.channel_name}
            </>
          ) : (
            "チャンネル"
          )}
        </h1>
        {channel?.description && canViewMembers ? (
          <p className="text-sm text-muted-foreground">{channel.description}</p>
        ) : null}
        <SyncStatus />
      </header>

      {!channel && channelsQuery.isLoading ? (
        <p className="text-sm text-muted-foreground">読み込み中…</p>
      ) : null}
      {!channel && channelsQuery.isError ? (
        <p className="text-sm text-destructive">チャンネル取得に失敗しました。</p>
      ) : null}
      {!channel && !channelsQuery.isLoading && !channelsQuery.isError ? (
        <p className="text-sm text-destructive">チャンネルが見つかりません。</p>
      ) : null}

      {channel ? (
        <>
          <ChannelMemberPanel
            channelId={channelId}
            isPrivate={channel.is_private}
            createdUserId={channel.created_user_id}
            members={members}
            isMember={isMember}
            userNames={userNames}
            isLoading={membersQuery.isLoading}
          />
          {membershipPending ? (
            <p className="text-sm text-muted-foreground">読み込み中…</p>
          ) : canRead ? (
            <MessagePane channelId={channelId} />
          ) : (
            <p className="text-sm text-muted-foreground">
              このチャンネルは非公開です。招待されるとメッセージを表示できます。
            </p>
          )}
        </>
      ) : null}
    </section>
  );
}
