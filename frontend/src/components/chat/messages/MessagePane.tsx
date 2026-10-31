import { ApiError, createConfiguredApiClient } from "../../../lib/api/client";
import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { useChannelCommands } from "../../../hooks/useChannelCommands";
import { useCurrentUser } from "../../../hooks/useCurrentUser";
import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { useAttachmentUpload, useMessageAttachments } from "../../../hooks/useAttachments";
import {
  useChannelMembers,
  useUsers,
  useMessageFeed,
  useMessageReactions,
} from "../../../hooks/useRealtime";
import { useReplyCounts, useThreadFeed } from "../../../hooks/useThread";
import { useWriteRunner } from "../../../hooks/useWriteRunner";
import { groupAttachmentsByMessage } from "../../../lib/attachments";
import { useCollections } from "../../../lib/electric/collections-context";
import { expandWindowSince, messageWindowStart } from "../../../lib/electric/messageWindow";
import type { MessageRow } from "../../../lib/electric/types";
import type { ChannelUserWriteMetadata } from "../../../lib/electric/writes";
import { groupReactions } from "../../../lib/emoji/reactions";
import { useOfflineStatus } from "../../../lib/offline/offline-context";
import { reactionKey } from "../../../lib/emoji/kitchen";
import { nowIso } from "../../../lib/time";
import { parseUtcTimestamp } from "../../../lib/timestamps";
import { useShell } from "../../layout/shell-context";
import { Button } from "../../ui/button";
import { ConfirmActionDialog } from "../../ui/confirm-action-dialog";
import { useTyping } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ReactionPicker } from "./ReactionPicker";
import { ThreadPane } from "./ThreadPane";

function oldestRowTimestamp(
  rows: Iterable<MessageRow>,
  predicate: (row: MessageRow) => boolean,
): string | null {
  let oldest: { at: string; ms: number } | null = null;
  for (const row of rows) {
    if (row.deleted_at || !predicate(row)) {
      continue;
    }
    const ms = parseUtcTimestamp(row.created_at).getTime();
    if (!Number.isFinite(ms) || (oldest && ms >= oldest.ms)) {
      continue;
    }
    oldest = { at: row.created_at, ms };
  }
  return oldest?.at ?? null;
}

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
export function MessagePane({
  channelId,
  isMember,
  canModerate = false,
  readOnly = false,
  targetMessageId,
}: {
  channelId: string;
  isMember: boolean;
  canModerate?: boolean;
  readOnly?: boolean;
  targetMessageId?: string;
}) {
  const { t } = useTranslation();
  const { messages, channelUsers, reactions, config } = useCollections();
  const appClient = useMemo(() => createConfiguredApiClient(config), [config]);
  const mentionUsers = (useUsers().data ?? []).filter(
    (user) => !user.is_bot || user.bot_mentionable,
  );
  const members = useChannelMembers(channelId).data;
  const mentionMemberIds = useMemo(
    () => new Set((members ?? []).map((member) => member.user_id)),
    [members],
  );
  const messagesCollection = messages(channelId);
  const reactionsCollection = reactions(channelId);
  // 直近30日を初期表示し、上端付近で `since` を過去へ広げる。`limit` は
  // 新着で最古行が落ちるため使わない。
  const [since, setSince] = useState(messageWindowStart);
  const messagesQuery = useMessageFeed(channelId, { since });
  const feed = messagesQuery.data ?? [];
  const reactionsQuery = useMessageReactions(channelId);
  const attachmentsQuery = useMessageAttachments(channelId);
  const attachmentsByMessage = useMemo(
    () => groupAttachmentsByMessage(attachmentsQuery.data ?? []),
    [attachmentsQuery.data],
  );
  const uploadAttachment = useAttachmentUpload(channelId);
  const { currentUserId } = useCurrentUser();
  const commandSuggestions = useChannelCommands(channelId, currentUserId);
  const { isOnline } = useOfflineStatus();
  const typing = useTyping(channelId, null, !readOnly && isMember && !!currentUserId);
  const { error, setError, run, isPending: isWritePending } = useWriteRunner();

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
  const [threadSince, setThreadSince] = useState(messageWindowStart);
  const threadQuery = useThreadFeed(channelId, threadRootId, threadSince);
  const threadReplies = threadQuery.data ?? [];
  const threadTyping = useTyping(
    channelId,
    threadRootId,
    !readOnly && isMember && !!currentUserId && !!threadRootId,
  );
  const threadRoot = useMemo(
    () => feed.find((message) => message.id === threadRootId) ?? null,
    [feed, threadRootId],
  );

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [isJoining, setIsJoining] = useState(false);

  useEffect(() => {
    if (!threadRootId) {
      setThreadSince(messageWindowStart());
      return;
    }
    // 通知から開いた返信が30日窓の外なら、その返信を含む位置から始める。
    const target = targetMessageId ? messagesCollection.get(targetMessageId) : undefined;
    const anchor = target?.parent_id === threadRootId ? target.created_at : null;
    setThreadSince(anchor ? expandWindowSince(messageWindowStart(), anchor) : messageWindowStart());
  }, [messagesCollection, targetMessageId, threadRootId]);

  // 30日窓が空でも、コレクションに古いメッセージがあれば自動で広げる。
  useEffect(() => {
    if (messagesQuery.isLoading || feed.length > 0) {
      return;
    }
    const oldest = oldestRowTimestamp(messagesCollection.values(), (row) => row.parent_id === null);
    if (oldest) {
      setSince((current) => expandWindowSince(current, oldest));
    }
  }, [feed.length, messagesCollection, messagesQuery.isLoading]);

  // 通知から開いたメッセージが30日窓の外でも、その行まで含める。
  useEffect(() => {
    if (!targetMessageId) {
      return;
    }
    const target = messagesCollection.get(targetMessageId);
    if (!target || target.deleted_at) {
      return;
    }
    if (target.parent_id) {
      setThreadSince((current) => expandWindowSince(current, target.created_at));
    } else {
      setSince((current) => expandWindowSince(current, target.created_at));
    }
  }, [messagesCollection, targetMessageId]);

  // スレッドのルートが窓の外でも、返信と一緒に表示できるよう広げる。
  useEffect(() => {
    if (!threadRootId) {
      return;
    }
    const root = messagesCollection.get(threadRootId);
    if (root) {
      setSince((current) => expandWindowSince(current, root.created_at));
    }
  }, [messagesCollection, threadRootId]);

  // 返信が30日窓で空なら、コレクション内の最も古い返信まで広げる。
  useEffect(() => {
    if (!threadRootId || threadQuery.isLoading || threadReplies.length > 0) {
      return;
    }
    const oldest = oldestRowTimestamp(
      messagesCollection.values(),
      (row) => row.parent_id === threadRootId,
    );
    if (oldest) {
      setThreadSince((current) => expandWindowSince(current, oldest));
    }
  }, [messagesCollection, threadReplies.length, threadQuery.isLoading, threadRootId]);

  useEffect(() => {
    if (!targetMessageId || messagesQuery.isLoading) return;
    const target = messagesCollection.get(targetMessageId);
    if (target?.parent_id && !target.deleted_at) openThread(target.parent_id);
  }, [targetMessageId, messagesQuery.isLoading, messagesCollection, openThread]);

  // ルートが（他ユーザーの操作などで）見えなくなったらスレッドも閉じる。
  // 論理削除なら、返信が残っていることを利用者に伝えてから閉じる。
  useEffect(() => {
    if (!threadRootId || messagesQuery.isLoading || threadRoot) {
      return;
    }
    const rootRow = messagesCollection.get(threadRootId);
    // The root may exist outside the current feed window; let setSince above load it.
    if (rootRow && !rootRow.deleted_at) return;
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

  const runCommand = useCallback(
    async (content: string, appId?: string): Promise<boolean | null> => {
      const command = /^\/([a-zA-Z0-9_-]+)(?:\s+([\s\S]*))?$/.exec(content.trim());
      if (!command) return null;
      try {
        if (appId) {
          await appClient.request(`/api/apps/${encodeURIComponent(appId)}/invoke`, {
            method: "POST",
            body: {
              kind: "command",
              name: command[1],
              channel_id: channelId,
              values: { text: command[2] ?? "" },
            },
          });
        } else {
          await appClient.request(`/api/apps/commands/${command[1]}`, {
            method: "POST",
            body: { channel_id: channelId, text: command[2] ?? "" },
          });
        }
        return true;
      } catch (error) {
        if (!appId && error instanceof ApiError && error.status === 404) return null;
        toast.error(t("apps.actionError"));
        return false;
      }
    },
    [appClient, channelId, t],
  );

  const handleSend = useCallback(
    async (
      content: string,
      commandAppId?: string,
      attachmentIds: string[] = [],
    ): Promise<boolean> => {
      if (attachmentIds.length === 0) {
        const commandResult = await runCommand(content, commandAppId);
        if (commandResult !== null) return commandResult;
      }
      const id = crypto.randomUUID();
      const metadata = attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : undefined;
      await run(() =>
        messagesCollection.insert(
          {
            id,
            user_id: currentUserId ?? "",
            channel_id: channelId,
            content,
            created_at: nowIso(),
            updated_at: null,
            deleted_at: null,
            parent_id: null,
          },
          metadata ? { metadata } : undefined,
        ),
      );
      // `run` は失敗を握ってロールバックするため、確定後に楽観行が残っているかで
      // 成否を判定する（Composer が失敗時に下書きを復元できるようにする）。
      return messagesCollection.get(id) !== undefined;
    },
    [run, messagesCollection, currentUserId, channelId, runCommand],
  );

  const handleSendReply = useCallback(
    async (
      content: string,
      commandAppId?: string,
      attachmentIds: string[] = [],
    ): Promise<boolean> => {
      if (readOnly || !isMember) {
        toast.info(t("messages.joinToPostDescription"), { id: "thread-reply-join-required" });
        return false;
      }
      if (attachmentIds.length === 0) {
        const commandResult = await runCommand(content, commandAppId);
        if (commandResult !== null) return commandResult;
      }
      const rootId = threadRootId;
      if (!rootId) {
        return false;
      }
      const id = crypto.randomUUID();
      const metadata = attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : undefined;
      await run(() =>
        messagesCollection.insert(
          {
            id,
            user_id: currentUserId ?? "",
            channel_id: channelId,
            content,
            created_at: nowIso(),
            updated_at: null,
            deleted_at: null,
            parent_id: rootId,
          },
          metadata ? { metadata } : undefined,
        ),
      );
      return messagesCollection.get(id) !== undefined;
    },
    [
      run,
      messagesCollection,
      currentUserId,
      channelId,
      threadRootId,
      isMember,
      readOnly,
      t,
      runCommand,
    ],
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
        {reactionsQuery.isError ||
        messagesQuery.isError ||
        messagesQuery.persistedStatus === "error" ? (
          <div
            role="status"
            className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b px-4 py-2"
          >
            {reactionsQuery.isError ? (
              <span className="text-xs text-destructive">{t("reactions.fetchError")}</span>
            ) : null}
            {messagesQuery.isError ? (
              <span
                className={
                  feed.length > 0 ? "text-xs text-muted-foreground" : "text-xs text-destructive"
                }
              >
                {feed.length > 0 ? t("offline.cached") : t("messages.fetchError")}
              </span>
            ) : null}
            {messagesQuery.persistedStatus === "error" ? (
              <span className="text-xs text-destructive">{t("offline.persistenceErrorTitle")}</span>
            ) : null}
          </div>
        ) : null}

        <MessageList
          targetMessageId={targetMessageId}
          key={`${channelId}:${currentUserId}`}
          channelId={channelId}
          messages={feed}
          isWritePending={isWritePending}
          isLoading={messagesQuery.isLoading}
          isError={messagesQuery.isError}
          currentUserId={readOnly ? null : currentUserId}
          canModerate={canModerate}
          editingId={editingId}
          editingText={editingText}
          onEditingTextChange={setEditingText}
          onStartEdit={handleStartEdit}
          onCancelEdit={handleCancelEdit}
          onSaveEdit={handleSaveEdit}
          onRequestDelete={setPendingDeleteId}
          reactions={reactionGroups}
          attachments={attachmentsByMessage}
          kitchenCatalog={catalog}
          canReact={
            !readOnly && !!currentUserId && !reactionsQuery.isLoading && !reactionsQuery.isError
          }
          onReaction={handleReaction}
          onPickReaction={setReactionMessageId}
          replyCounts={replyCounts}
          onOpenThread={openThread}
          canCompose={isMember}
          isOffline={!isOnline}
          onLoadOlder={(oldest) => {
            setSince((current) => expandWindowSince(current, oldest));
          }}
          onRetry={() => window.location.reload()}
        />

        {readOnly ? null : isMember ? (
          <>
            <TypingIndicator users={typing.users} />
            <Composer
              commands={commandSuggestions.commands}
              onRequestCommands={commandSuggestions.load}
              mentionUsers={mentionUsers}
              mentionMemberIds={mentionMemberIds}
              key={channelId}
              onSend={handleSend}
              onUploadAttachment={uploadAttachment}
              onTyping={typing.onTyping}
              disabled={readOnly || !isOnline}
              placeholder={isOnline ? undefined : t("offline.composeDisabled")}
            />
          </>
        ) : (
          <JoinToPostPrompt
            onJoin={() => void handleJoin()}
            joining={isJoining}
            disabled={!currentUserId || !isOnline}
          />
        )}
      </div>

      {threadRoot ? (
        <ThreadPane
          commands={commandSuggestions.commands}
          onRequestCommands={commandSuggestions.load}
          targetMessageId={targetMessageId}
          mentionUsers={mentionUsers}
          mentionMemberIds={mentionMemberIds}
          key={threadRoot.id}
          readOnly={readOnly}
          isMember={!readOnly && isMember}
          isOffline={!isOnline}
          typingUsers={threadTyping.users}
          onTyping={threadTyping.onTyping}
          root={threadRoot}
          replies={threadReplies}
          isLoading={threadQuery.isLoading}
          isError={threadQuery.isError}
          currentUserId={readOnly ? null : currentUserId}
          canModerate={canModerate}
          reactions={reactionGroups}
          attachments={attachmentsByMessage}
          onUploadAttachment={uploadAttachment}
          kitchenCatalog={catalog}
          canReact={
            !readOnly && !!currentUserId && !reactionsQuery.isLoading && !reactionsQuery.isError
          }
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
          onLoadOlder={() => {
            const oldest = threadReplies[0];
            if (oldest) {
              setThreadSince((current) => expandWindowSince(current, oldest.created_at));
            }
          }}
          onRetry={() => window.location.reload()}
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

      <ConfirmActionDialog
        open={pendingDeleteId !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDeleteId(null);
        }}
        title={t("messages.deleteTitle")}
        description={t("messages.deleteDescription")}
        confirmLabel={t("messages.delete")}
        onConfirm={handleConfirmDelete}
      />
    </section>
  );
}
