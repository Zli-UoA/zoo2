import { Link, createFileRoute } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import { ChannelMemberPanel } from "../../components/ChannelMemberPanel";
import { MessagePane } from "../../components/MessagePane";
import { SyncStatus } from "../../components/SyncStatus";
import { useActiveChannels } from "../../hooks/useRealtime";

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
  const channelsQuery = useActiveChannels();
  const channels = channelsQuery.data ?? [];
  const channel = useMemo(
    () => channels.find((candidate) => candidate.id === channelId) ?? null,
    [channels, channelId],
  );

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
        {channel?.description ? (
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
          <ChannelMemberPanel channelId={channelId} isPrivate={channel.is_private} />
          <MessagePane channelId={channelId} />
        </>
      ) : null}
    </section>
  );
}
