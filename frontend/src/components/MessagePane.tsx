import { useState, type FormEvent } from "react";

import { useMessageFeed } from "../hooks/useRealtime";
import { useWriteRunner } from "../hooks/useWriteRunner";
import { getCollections } from "../lib/electric/collections";
import { normalizeUtcTimestamp } from "../lib/timestamps";
import { Button } from "./ui/button";
import { inputClassName } from "./ui/input-styles";

interface MessageFeedRow {
  id: string;
  user_id: string;
  content: string;
  created_at: string;
  user_name?: string | null;
  display_name?: string | null;
}

function formatTimestamp(value: string): string {
  return new Date(normalizeUtcTimestamp(value)).toLocaleString("ja-JP");
}

function nowIso(): string {
  return new Date().toISOString();
}

function authorLabel(message: MessageFeedRow): string {
  return message.display_name || message.user_name || message.user_id;
}

/** チャンネルのメッセージ一覧・送信・編集・論理削除。 */
export function MessagePane({ channelId }: { channelId: string }) {
  const { messages, config } = getCollections();
  const messagesCollection = messages(channelId);
  const messagesQuery = useMessageFeed(channelId);
  const feed = messagesQuery.data ?? [];
  const currentUserId = config.devUserId;
  const { error, run } = useWriteRunner();
  const [draft, setDraft] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");

  async function sendMessage(event: FormEvent) {
    event.preventDefault();
    const content = draft.trim();
    if (!content) return;
    const channel_id = channelId;
    setDraft("");
    await run(() =>
      messagesCollection.insert({
        id: crypto.randomUUID(),
        user_id: currentUserId ?? "",
        channel_id,
        content,
        created_at: nowIso(),
        updated_at: null,
        deleted_at: null,
      }),
    );
  }

  async function saveEdit(messageId: string) {
    const content = editingText.trim();
    if (!content) return;
    setEditingId(null);
    await run(() =>
      messagesCollection.update(messageId, (message) => {
        message.content = content;
      }),
    );
  }

  async function deleteMessage(messageId: string) {
    // 論理削除は `deleted_at` の update。Electric が流す operation と揃えて
    // delete→insert の再出現（ちらつき）を避ける（Phase 3 の扱いを踏襲）。
    await run(() =>
      messagesCollection.update(messageId, (message) => {
        message.deleted_at = nowIso();
      }),
    );
  }

  return (
    <section className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-3 border-b px-4 py-2 text-sm">
        <h2 className="text-sm font-semibold">メッセージ</h2>
        <span className="rounded bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
          {messagesQuery.isLoading ? "…" : `${feed.length} 件`}
        </span>
        {messagesQuery.isError ? (
          <span className="text-xs text-destructive">メッセージ取得に失敗しました</span>
        ) : null}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
        {error ? <p className="mb-3 text-sm text-destructive">操作失敗: {error}</p> : null}

        <ul className="flex flex-col gap-3">
          {feed.map((message) => (
            <li key={message.id} className="rounded-lg border bg-card p-3">
              <div className="flex items-baseline justify-between gap-2">
                <span className="font-medium">{authorLabel(message)}</span>
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
                  {currentUserId && message.user_id === currentUserId ? (
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
                  ) : null}
                </>
              )}
            </li>
          ))}
        </ul>

        {feed.length === 0 && !messagesQuery.isLoading ? (
          <p className="text-sm text-muted-foreground">メッセージがありません。</p>
        ) : null}
      </div>

      <form className="flex items-end gap-2 border-t px-4 py-3" onSubmit={sendMessage}>
        <textarea
          className={`${inputClassName()} min-h-10 grow py-1.5`}
          placeholder="メッセージを送信"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          rows={1}
        />
        <Button type="submit">送信</Button>
      </form>
    </section>
  );
}
