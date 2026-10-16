import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

import { useMessageFeed } from "../hooks/useRealtime";
import { useWriteRunner } from "../hooks/useWriteRunner";
import { getCollections } from "../lib/electric/collections";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "./ui/alert-dialog";
import { Composer } from "./chat/messages/Composer";
import { MessageList } from "./chat/messages/MessageList";

function nowIso(): string {
  return new Date().toISOString();
}

/**
 * チャンネルのメッセージ表示・送信・編集・論理削除。
 *
 * データ層は `useMessageFeed`（messages ⋈ users の live query）と
 * `useWriteRunner`（楽観更新の確定待ち）をそのまま使い、表示だけを作り込む。
 */
export function MessagePane({ channelId }: { channelId: string }) {
  const { messages, config } = getCollections();
  const messagesCollection = messages(channelId);
  const messagesQuery = useMessageFeed(channelId);
  const feed = messagesQuery.data ?? [];
  const currentUserId = config.devUserId;
  const { error, setError, run } = useWriteRunner();

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  // 書き込み失敗は sonner で通知する（既存の `useWriteRunner` の error を監視）。
  useEffect(() => {
    if (!error) {
      return;
    }
    toast.error("操作に失敗しました", { description: error, id: "message-write-error" });
    setError(null);
  }, [error, setError]);

  const handleSend = useCallback(
    async (content: string): Promise<boolean> => {
      const id = crypto.randomUUID();
      await run(() =>
        messagesCollection.insert({
          id,
          user_id: currentUserId ?? "",
          channel_id: channelId,
          content,
          created_at: nowIso(),
          updated_at: null,
          deleted_at: null,
        }),
      );
      // `run` は失敗を握ってロールバックするため、確定後に楽観行が残っているかで
      // 成否を判定する（Composer が失敗時に下書きを復元できるようにする）。
      return messagesCollection.get(id) !== undefined;
    },
    [run, messagesCollection, currentUserId, channelId],
  );

  const handleStartEdit = useCallback((id: string, content: string) => {
    setEditingId(id);
    setEditingText(content);
  }, []);

  const handleCancelEdit = useCallback(() => {
    setEditingId(null);
    setEditingText("");
  }, []);

  const handleSaveEdit = useCallback(async () => {
    const id = editingId;
    const content = editingText.trim();
    if (!id || !content) {
      return;
    }
    setEditingId(null);
    setEditingText("");
    await run(() =>
      messagesCollection.update(id, (message) => {
        message.content = content;
      }),
    );
  }, [editingId, editingText, run, messagesCollection]);

  const handleConfirmDelete = useCallback(async () => {
    const id = pendingDeleteId;
    setPendingDeleteId(null);
    if (!id) {
      return;
    }
    // 論理削除は `deleted_at` の update。Electric が流す operation と揃えて
    // delete→insert の再出現（ちらつき）を避ける（Phase 3 の扱いを踏襲）。
    await run(() =>
      messagesCollection.update(id, (message) => {
        message.deleted_at = nowIso();
      }),
    );
  }, [pendingDeleteId, run, messagesCollection]);

  return (
    <section className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b px-4 py-2">
        <h2 className="text-sm font-semibold">メッセージ</h2>
        <span className="rounded bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
          {messagesQuery.isLoading ? "…" : `${feed.length} 件`}
        </span>
        {messagesQuery.isError ? (
          <span className="text-xs text-destructive">メッセージ取得に失敗しました</span>
        ) : null}
      </div>

      <MessageList
        key={channelId}
        messages={feed}
        isLoading={messagesQuery.isLoading}
        isError={messagesQuery.isError}
        currentUserId={currentUserId}
        editingId={editingId}
        editingText={editingText}
        onEditingTextChange={setEditingText}
        onStartEdit={handleStartEdit}
        onCancelEdit={handleCancelEdit}
        onSaveEdit={handleSaveEdit}
        onRequestDelete={setPendingDeleteId}
      />

      <Composer onSend={handleSend} />

      <AlertDialog
        open={pendingDeleteId !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPendingDeleteId(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>メッセージを削除しますか？</AlertDialogTitle>
            <AlertDialogDescription>
              この操作は取り消せません。削除したメッセージはチャンネルのメンバーからも見えなくなります。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleConfirmDelete}>
              削除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
