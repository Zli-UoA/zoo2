import { useMemo, useState } from "react";

import {
  useActiveChannels,
  useChannelMembers,
  useMessageFeed,
  useUsers,
} from "../hooks/useRealtime";
import { normalizeUtcTimestamp } from "../lib/timestamps";
import { Button } from "./ui/button";

function formatTimestamp(value: string): string {
  return new Date(normalizeUtcTimestamp(value)).toLocaleString("ja-JP");
}

interface AuthorLike {
  user_id: string;
  user_name?: string | null;
  display_name?: string | null;
}

function authorLabel(row: AuthorLike, fallbackNames: Map<string, string>): string {
  return row.display_name || row.user_name || fallbackNames.get(row.user_id) || row.user_id;
}

export function RealtimeReadView() {
  const channelsQuery = useActiveChannels();
  const usersQuery = useUsers();
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const channels = channelsQuery.data ?? [];
  const activeId = useMemo(() => {
    if (selectedId && channels.some((channel) => channel.id === selectedId)) {
      return selectedId;
    }
    return channels[0]?.id ?? null;
  }, [selectedId, channels]);

  const messagesQuery = useMessageFeed(activeId);
  const membersQuery = useChannelMembers(activeId);

  const userNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const user of usersQuery.data ?? []) {
      map.set(user.id, user.display_name || user.user_name);
    }
    return map;
  }, [usersQuery.data]);

  const messages = messagesQuery.data ?? [];
  const members = membersQuery.data ?? [];

  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-col gap-1">
        <h1 className="text-2xl font-bold">リアルタイム読み取り（Phase 2）</h1>
        <p className="text-sm text-muted-foreground">
          Electric Shape を購読し、messages と users を live query で結合して表示します。
        </p>
      </header>

      <div className="flex flex-wrap items-center gap-3 text-sm">
        <span className="rounded bg-muted px-2 py-1">
          チャンネル: {channelsQuery.isLoading ? "読み込み中…" : `${channels.length} 件`}
        </span>
        {channelsQuery.isError ? (
          <span className="text-destructive">チャンネル取得に失敗: {channelsQuery.status}</span>
        ) : null}
      </div>

      <div className="grid gap-6 md:grid-cols-[220px_1fr]">
        <nav className="flex flex-col gap-1">
          <h2 className="text-sm font-semibold text-muted-foreground">チャンネル</h2>
          {channels.length === 0 && !channelsQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">チャンネルがありません。</p>
          ) : null}
          {channels.map((channel) => (
            <Button
              key={channel.id}
              variant={channel.id === activeId ? "secondary" : "ghost"}
              className="justify-start"
              onClick={() => setSelectedId(channel.id)}
            >
              {channel.is_private ? "🔒 " : "# "}
              {channel.channel_name}
            </Button>
          ))}
        </nav>

        <div className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <span className="rounded bg-muted px-2 py-1">
              メンバー: {activeId ? members.length : "-"} 名
            </span>
            <span className="rounded bg-muted px-2 py-1">
              メッセージ: {messagesQuery.isLoading ? "…" : `${messages.length} 件`}
            </span>
            {messagesQuery.isError ? (
              <span className="text-destructive">メッセージ取得に失敗: {messagesQuery.status}</span>
            ) : null}
          </div>

          {members.length > 0 ? (
            <p className="text-xs text-muted-foreground">
              参加者:{" "}
              {members.map((member) => userNames.get(member.user_id) ?? member.user_id).join(", ")}
            </p>
          ) : null}

          <ul className="flex flex-col gap-3">
            {messages.map((message) => (
              <li key={message.id} className="rounded-lg border p-3">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="font-medium">{authorLabel(message, userNames)}</span>
                  <time className="text-xs text-muted-foreground" dateTime={message.created_at}>
                    {formatTimestamp(message.created_at)}
                  </time>
                </div>
                <p className="mt-1 whitespace-pre-wrap">{message.content}</p>
              </li>
            ))}
          </ul>

          {activeId && messages.length === 0 && !messagesQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">メッセージがありません。</p>
          ) : null}
        </div>
      </div>
    </section>
  );
}
