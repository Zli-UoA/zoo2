import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { useCurrentUser } from "../../../hooks/useCurrentUser";
import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { useMessageFeed, useMessageReactions } from "../../../hooks/useRealtime";
import { useReplyCounts, useThreadFeed } from "../../../hooks/useThread";
import { useWriteRunner } from "../../../hooks/useWriteRunner";
import { getCollections } from "../../../lib/electric/collections";
import type { ChannelUserWriteMetadata } from "../../../lib/electric/writes";
import { groupReactions } from "../../../lib/emoji/reactions";
import { reactionKey } from "../../../lib/emoji/kitchen";
import { nowIso } from "../../../lib/time";
import { useShell } from "../../layout/shell-context";
import { Button } from "../../ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "../../ui/alert-dialog";
import { useTyping } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ReactionPicker } from "./ReactionPicker";
import { ThreadPane } from "./ThreadPane";

function JoinToPostPrompt({
  onJoin,
  joining,
  disabled,
}: {
  onJoin: () => void;
  joining: boolean;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="border-t px-3 py-3">
      <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed bg-muted/30 px-4 py-4 text-center">
        <p className="text-sm text-muted-foreground">{t("messages.joinToPostDescription")}</p>
        <Button size="sm" onClick={onJoin} disabled={disabled || joining} aria-busy={joining}>
          {t("messages.joinToPost")}
        </Button>
      </div>
    </div>
  );
}

/**
 * チャンネルのメッセージ表示・送信・編集・論理削除に加え、リアクションと
 * スレッドを扱う。
 *
 * データ層は `useMessageFeed`（messages ⋈ users の live query）と
 * `useWriteRunner`（楽観更新の確定待ち）をそのまま使い、表示だけを作り込む。
 * スレッドも同じ messages コレクションを `parent_id` で絞るだけで、専用の
 * Shape / コレクションは作らない。
 *
 * 未参加の公開チャンネルでは Composer の代わりに参加導線を出し、参加後に
 * 通常の Composer へ戻す（`isMember` は `useChannelAccess` の判定）。
 */
export function MessagePane({ channelId, isMember }: { channelId: string; isMember: boolean }) {
  const { t } = useTranslation();
  const { messages, channelUsers, reactions } = getCollections();
  const messagesCollection = messages(channelId);
  const reactionsCollection = reactions(channelId);
  const messagesQuery = useMessageFeed(channelId);
  const feed = messagesQuery.data ?? [];
  const reactionsQuery = useMessageReactions(channelId);
  const { currentUserId } = useCurrentUser();
  const typing = useTyping(channelId, null, isMember && !!currentUserId);
  const { error, setError, run } = useWriteRunner();

  const [reactionMessageId, setReactionMessageId] = useState<string | null>(null);
  const reactionRows = reactionsQuery.data ?? [];
  const reactionGroups = useMemo(
    () => groupReactions(reactionRows, currentUserId),
    [reactionRows, currentUserId],
  );
  const { catalog } = useKitchenCatalog(
    reactionRows.some((row) => row.emoji.startsWith("kitchen:")),
  );
  const pendingReactions = useRef(new Set<string>());

  const handleReaction = useCallback(
    async (messageId: string, emoji: string) => {
      if (
        !currentUserId ||
        !messagesCollection.get(messageId) ||
        messagesCollection.get(messageId)?.deleted_at
      ) {
        return;
      }
      const key = reactionKey(emoji);
      const pendingKey = `${messageId}:${key}`;
      if (pendingReactions.current.has(pendingKey)) {
        return;
      }
      pendingReactions.current.add(pendingKey);
      try {
        await run(() => {
          const own = Array.from(reactionsCollection.values()).find(
            (row) =>
              row.message_id === messageId &&
              row.user_id === currentUserId &&
              reactionKey(row.emoji) === key,
          );
          return own
            ? reactionsCollection.delete(own.id)
            : reactionsCollection.insert({
                id: crypto.randomUUID(),
                message_id: messageId,
                user_id: currentUserId,
                emoji,
                created_at: nowIso(),
              });
        });
      } finally {
        pendingReactions.current.delete(pendingKey);
      }
    },
    [currentUserId, messagesCollection, reactionsCollection, run],
  );

  const replyCounts = useReplyCounts(channelId);

  const { threadRootId, openThread, closeThread } = useShell();
  const threadQuery = useThreadFeed(channelId, threadRootId);
  const threadReplies = threadQuery.data ?? [];
  const threadTyping = useTyping(
    channelId,
    threadRootId,
    isMember && !!currentUserId && !!threadRootId,
  );
  const threadRoot = useMemo(
    () => feed.find((message) => message.id === threadRootId) ?? null,
    [feed, threadRootId],
  );

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [isJoining, setIsJoining] = useState(false);

  // チャンネルを移動したら開いていたスレッドは閉じる。スレッドの状態は
  // メンバーパネルとの排他を管理する AppShell が持つため、離脱時にも閉じる。
  useEffect(() => {
    closeThread();
  }, [channelId, closeThread]);
  useEffect(() => closeThread, [closeThread]);

  // ルートが（他ユーザーの操作などで）見えなくなったらスレッドも閉じる。
  // 論理削除なら、返信が残っていることを利用者に伝えてから閉じる。
  useEffect(() => {
    if (!threadRootId || messagesQuery.isLoading || threadRoot) {
      return;
    }
    const rootRow = messagesCollection.get(threadRootId);
    closeThread();
    if (rootRow?.deleted_at) {
      toast.info(t("threads.rootDeleted"), { id: "thread-root-deleted" });
    }
  }, [threadRootId, threadRoot, messagesQuery.isLoading, messagesCollection, closeThread, t]);

  // 書き込み失敗は sonner で通知する（既存の `useWriteRunner` の error を監視）。
  useEffect(() => {
    if (!error) {
      return;
    }
    toast.error(t("common.writeError"), { description: error, id: "message-write-error" });
    setError(null);
  }, [error, setError, t]);

  const handleJoin = useCallback(async () => {
    if (!currentUserId || isJoining) {
      return;
    }
    setIsJoining(true);
    try {
      await run(() =>
        channelUsers.insert(
          { user_id: currentUserId, channel_id: channelId, joined_at: nowIso() },
          { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
        ),
      );
    } finally {
      setIsJoining(false);
    }
  }, [channelId, channelUsers, currentUserId, isJoining, run]);

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
          parent_id: null,
        }),
      );
      // `run` は失敗を握ってロールバックするため、確定後に楽観行が残っているかで
      // 成否を判定する（Composer が失敗時に下書きを復元できるようにする）。
      return messagesCollection.get(id) !== undefined;
    },
    [run, messagesCollection, currentUserId, channelId],
  );

  const handleSendReply = useCallback(
    async (content: string): Promise<boolean> => {
      if (!isMember) {
        toast.info(t("messages.joinToPostDescription"), { id: "thread-reply-join-required" });
        return false;
      }
      const rootId = threadRootId;
      if (!rootId) {
        return false;
      }
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
          parent_id: rootId,
        }),
      );
      return messagesCollection.get(id) !== undefined;
    },
    [run, messagesCollection, currentUserId, channelId, threadRootId, isMember, t],
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
    <section className="relative flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b px-4 py-2">
          <h2 className="text-sm font-semibold">{t("messages.title")}</h2>
          <span className="rounded bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
            {messagesQuery.isLoading ? "…" : t("messages.count", { count: feed.length })}
          </span>
          {reactionsQuery.isError ? (
            <span className="text-xs text-destructive">{t("reactions.fetchError")}</span>
          ) : null}
          {messagesQuery.isError ? (
            <span className="text-xs text-destructive">{t("messages.fetchError")}</span>
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
          reactions={reactionGroups}
          kitchenCatalog={catalog}
          canReact={!!currentUserId && !reactionsQuery.isLoading && !reactionsQuery.isError}
          onReaction={handleReaction}
          onPickReaction={setReactionMessageId}
          replyCounts={replyCounts}
          onOpenThread={openThread}
          canCompose={isMember}
        />

        {isMember ? (
          <>
            <TypingIndicator users={typing.users} />
            <Composer key={channelId} onSend={handleSend} onTyping={typing.onTyping} />
          </>
        ) : (
          <JoinToPostPrompt
            onJoin={() => void handleJoin()}
            joining={isJoining}
            disabled={!currentUserId}
          />
        )}
      </div>

      {threadRoot ? (
        <ThreadPane
          key={threadRoot.id}
          isMember={isMember}
          typingUsers={threadTyping.users}
          onTyping={threadTyping.onTyping}
          root={threadRoot}
          replies={threadReplies}
          isLoading={threadQuery.isLoading}
          isError={threadQuery.isError}
          currentUserId={currentUserId}
          reactions={reactionGroups}
          kitchenCatalog={catalog}
          canReact={!!currentUserId && !reactionsQuery.isLoading && !reactionsQuery.isError}
          onReaction={handleReaction}
          onPickReaction={setReactionMessageId}
          editingId={editingId}
          editingText={editingText}
          onEditingTextChange={setEditingText}
          onStartEdit={handleStartEdit}
          onCancelEdit={handleCancelEdit}
          onSaveEdit={handleSaveEdit}
          onRequestDelete={setPendingDeleteId}
          onSendReply={handleSendReply}
          onClose={closeThread}
        />
      ) : null}

      <ReactionPicker
        open={reactionMessageId !== null}
        onOpenChange={(open) => {
          if (!open) {
            setReactionMessageId(null);
          }
        }}
        onSelect={(emoji) => {
          if (reactionMessageId) {
            void handleReaction(reactionMessageId, emoji);
          }
        }}
      />

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
            <AlertDialogTitle>{t("messages.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("messages.deleteDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancelShort")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleConfirmDelete}>
              {t("messages.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
