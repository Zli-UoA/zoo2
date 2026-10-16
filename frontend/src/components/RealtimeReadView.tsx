import type { Transaction } from "@tanstack/react-db";
import { useMemo, useState, type FormEvent } from "react";

import {
  useActiveChannels,
  useChannelMembers,
  useMessageFeed,
  useUsers,
} from "../hooks/useRealtime";
import { getCollections } from "../lib/electric/collections";
import type { ChannelUserWriteMetadata } from "../lib/electric/writes";
import { normalizeUtcTimestamp } from "../lib/timestamps";
import { Button } from "./ui/button";

function formatTimestamp(value: string): string {
  return new Date(normalizeUtcTimestamp(value)).toLocaleString("ja-JP");
}

function nowIso(): string {
  return new Date().toISOString();
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function settle(transaction: Transaction): Promise<void> {
  await transaction.when("settled");
}

interface AuthorLike {
  user_id: string;
  user_name?: string | null;
  display_name?: string | null;
}

function authorLabel(row: AuthorLike, fallbackNames: Map<string, string>): string {
  return row.display_name || row.user_name || fallbackNames.get(row.user_id) || row.user_id;
}

function inputClassName(): string {
  return "h-8 rounded-lg border border-border bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50";
}

export function RealtimeReadView() {
  const collections = getCollections();
  const { config } = collections;
  const channelsQuery = useActiveChannels();
  const usersQuery = useUsers();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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
  const currentUserId = config.devUserId;
  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;

  const [newChannelName, setNewChannelName] = useState("");
  const [newChannelPrivate, setNewChannelPrivate] = useState(false);
  const [draft, setDraft] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const [memberId, setMemberId] = useState("");

  async function run(action: () => Transaction | Promise<Transaction>): Promise<void> {
    setError(null);
    try {
      await settle(await action());
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  async function createChannel(event: FormEvent) {
    event.preventDefault();
    const channel_name = newChannelName.trim();
    if (!channel_name) return;
    setNewChannelName("");
    await run(() =>
      collections.channels.insert({
        id: crypto.randomUUID(),
        channel_name,
        description: null,
        created_user_id: currentUserId ?? "",
        is_private: newChannelPrivate,
        created_at: nowIso(),
        updated_at: null,
        archive_at: null,
        deleted_at: null,
      }),
    );
  }

  async function sendMessage(event: FormEvent) {
    event.preventDefault();
    const content = draft.trim();
    if (!content || !activeId) return;
    const channelId = activeId;
    setDraft("");
    await run(() =>
      collections.messages(channelId).insert({
        id: crypto.randomUUID(),
        user_id: currentUserId ?? "",
        channel_id: channelId,
        content,
        created_at: nowIso(),
        updated_at: null,
        deleted_at: null,
      }),
    );
  }

  async function saveEdit(messageId: string) {
    const content = editingText.trim();
    if (!content || !activeId) return;
    const channelId = activeId;
    setEditingId(null);
    await run(() =>
      collections.messages(channelId).update(messageId, (message) => {
        message.content = content;
      }),
    );
  }

  async function deleteMessage(messageId: string) {
    if (!activeId) return;
    const channelId = activeId;
    await run(() =>
      collections.messages(channelId).update(messageId, (message) => {
        message.deleted_at = nowIso();
      }),
    );
  }

  async function joinChannel() {
    if (!activeId || !currentUserId) return;
    const channelId = activeId;
    await run(() =>
      collections.channelUsers.insert(
        { user_id: currentUserId, channel_id: channelId, joined_at: nowIso() },
        { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  async function leaveChannel() {
    if (!activeId || !currentUserId) return;
    const channelId = activeId;
    const userId = currentUserId;
    await run(() =>
      collections.channelUsers.delete(`${userId}:${channelId}`, {
        metadata: { action: "leave" } satisfies ChannelUserWriteMetadata,
      }),
    );
  }

  async function addMember(event: FormEvent) {
    event.preventDefault();
    const user_id = memberId.trim();
    if (!activeId || !user_id) return;
    const channelId = activeId;
    setMemberId("");
    await run(() =>
      collections.channelUsers.insert(
        { user_id, channel_id: channelId, joined_at: nowIso() },
        { metadata: { action: "add", user_id } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-col gap-1">
        <h1 className="text-2xl font-bold">リアルタイム書き込み（Phase 3）</h1>
        <p className="text-sm text-muted-foreground">
          Electric Shape を購読し、REST 書き込みを楽観更新して {config.txidMode} モードで txid
          を突き合わせます。
        </p>
      </header>

      <div className="flex flex-wrap items-center gap-3 text-sm">
        <span className="rounded bg-muted px-2 py-1">
          チャンネル: {channelsQuery.isLoading ? "読み込み中…" : `${channels.length} 件`}
        </span>
        <span className="rounded bg-muted px-2 py-1">txid モード: {config.txidMode}</span>
        <span className="rounded bg-muted px-2 py-1">
          現在のユーザー: {currentUserId ?? "（未設定: X-Dev-User なし）"}
        </span>
        {channelsQuery.isError ? (
          <span className="text-destructive">チャンネル取得に失敗: {channelsQuery.status}</span>
        ) : null}
        {error ? <span className="text-destructive">操作失敗: {error}</span> : null}
      </div>

      <form className="flex flex-wrap items-center gap-2" onSubmit={createChannel}>
        <input
          className={inputClassName()}
          placeholder="新しいチャンネル名"
          value={newChannelName}
          onChange={(event) => setNewChannelName(event.target.value)}
        />
        <label className="flex items-center gap-1 text-sm">
          <input
            type="checkbox"
            checked={newChannelPrivate}
            onChange={(event) => setNewChannelPrivate(event.target.checked)}
          />
          非公開
        </label>
        <Button type="submit" size="sm">
          チャンネル作成
        </Button>
      </form>

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
            {activeId && currentUserId ? (
              isMember ? (
                <Button variant="outline" size="sm" onClick={leaveChannel}>
                  退出
                </Button>
              ) : (
                <Button variant="outline" size="sm" onClick={joinChannel}>
                  参加
                </Button>
              )
            ) : null}
            {messagesQuery.isError ? (
              <span className="text-destructive">メッセージ取得に失敗: {messagesQuery.status}</span>
            ) : null}
          </div>

          {activeId ? (
            <form className="flex flex-wrap items-center gap-2" onSubmit={addMember}>
              <input
                className={inputClassName()}
                placeholder="追加するユーザー ID"
                value={memberId}
                onChange={(event) => setMemberId(event.target.value)}
              />
              <Button type="submit" variant="outline" size="sm">
                メンバー追加
              </Button>
            </form>
          ) : null}

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
                {editingId === message.id ? (
                  <div className="mt-1 flex items-center gap-2">
                    <input
                      className={`${inputClassName()} grow`}
                      value={editingText}
                      onChange={(event) => setEditingText(event.target.value)}
                    />
                    <Button size="sm" onClick={() => saveEdit(message.id)}>
                      保存
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => setEditingId(null)}>
                      取消
                    </Button>
                  </div>
                ) : (
                  <>
                    <p className="mt-1 whitespace-pre-wrap">{message.content}</p>
                    <div className="mt-2 flex gap-2">
                      <Button
                        variant="ghost"
                        size="xs"
                        onClick={() => {
                          setEditingId(message.id);
                          setEditingText(message.content);
                        }}
                      >
                        編集
                      </Button>
                      <Button
                        variant="destructive"
                        size="xs"
                        onClick={() => deleteMessage(message.id)}
                      >
                        削除
                      </Button>
                    </div>
                  </>
                )}
              </li>
            ))}
          </ul>

          {activeId && messages.length === 0 && !messagesQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">メッセージがありません。</p>
          ) : null}

          {activeId ? (
            <form className="flex items-end gap-2" onSubmit={sendMessage}>
              <textarea
                className={`${inputClassName()} min-h-16 grow py-1`}
                placeholder="メッセージを送信"
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
              />
              <Button type="submit">送信</Button>
            </form>
          ) : null}
        </div>
      </div>
    </section>
  );
}
