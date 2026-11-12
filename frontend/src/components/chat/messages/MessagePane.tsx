import { refreshAppSurfaces } from "../../../lib/appSurfaceEvents";
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
import { useMessageById, useThreadFeed, useThreadRoot } from "../../../hooks/useThread";
import { useWriteRunner } from "../../../hooks/useWriteRunner";
import { groupAttachmentsByMessage } from "../../../lib/attachments";
import { useCollections } from "../../../lib/electric/collections-context";
import type { ChannelUserWriteMetadata } from "../../../lib/electric/writes";
import { groupReactions, reactionUserLabel } from "../../../lib/emoji/reactions";
import { useOfflineStatus } from "../../../lib/offline/offline-context";
import { reactionKey } from "../../../lib/emoji/kitchen";
import { nowIso } from "../../../lib/time";
import { useShell } from "../../layout/shell-context";
import { Button } from "../../ui/button";
import { ConfirmActionDialog } from "../../ui/confirm-action-dialog";
import { useTyping } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ReactionPicker } from "./ReactionPicker";
import { ThreadPane } from "./ThreadPane";

/** 通知・リンクの対象を含むまで、古い方へ広げるページ数の上限 (1 ページは `MESSAGE_PAGE_SIZE` 件)。 */
const MAX_REVEAL_PAGES = 200;

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
  const usersQuery = useUsers();
  const mentionUsers = (usersQuery.data ?? []).filter(
    (user) => !user.is_bot || user.bot_mentionable,
  );
  const members = useChannelMembers(channelId).data;
  const mentionMemberIds = useMemo(
    () => new Set((members ?? []).map((member) => member.user_id)),
    [members],
  );
  const messagesCollection = messages(channelId);
  const reactionsCollection = reactions(channelId);
  // 新しい方から `MESSAGE_PAGE_SIZE` 件だけを同期し、上端付近で `loadOlder()` が古い方へ 1 ページずつ広げる。
  const messagesQuery = useMessageFeed(channelId);
  const feed = messagesQuery.data ?? [];
  const reactionsQuery = useMessageReactions(channelId);
  const uploadAttachment = useAttachmentUpload(channelId);
  const { currentUserId } = useCurrentUser();
  const commandSuggestions = useChannelCommands(channelId, currentUserId);
  const { isOnline } = useOfflineStatus();
  const typing = useTyping(channelId, null, !readOnly && isMember && !!currentUserId);
  const { error, setError, run, isPending: isWritePending } = useWriteRunner();

  const [reactionMessageId, setReactionMessageId] = useState<string | null>(null);
  const reactionRows = reactionsQuery.data ?? [];
  const reactionGroups = useMemo(() => {
    const names = Object.fromEntries(
      (usersQuery.data ?? []).map((user) => [user.id, reactionUserLabel(user)]),
    );
    const groups = groupReactions(reactionRows, currentUserId);
    for (const reactions of groups.values()) {
      for (const reaction of reactions) {
        reaction.userNames = { ...reaction.userNames, ...names };
        reaction.usersLoading = usersQuery.isLoading;
      }
    }
    return groups;
  }, [reactionRows, currentUserId, usersQuery.data, usersQuery.isLoading]);
  const { catalog } = useKitchenCatalog(
    reactionRows.some((row) => row.emoji.startsWith("kitchen:")),
  );
  const pendingReactions = useRef(new Set<string>());

  const { threadRootId, openThread, closeThread } = useShell();
  const threadQuery = useThreadFeed(channelId, threadRootId);
  const threadReplies = threadQuery.data ?? [];
  const threadTyping = useTyping(
    channelId,
    threadRootId,
    !readOnly && isMember && !!currentUserId && !!threadRootId,
  );
  const threadRootQuery = useThreadRoot(channelId, threadRootId);
  const threadRootRow = threadRootQuery.data?.[0] ?? null;
  const threadRoot = threadRootRow && !threadRootRow.deleted_at ? threadRootRow : null;
  const threadIsArchived = !!threadRoot?.thread_archive_at;

  const attachmentsQuery = useMessageAttachments(
    channelId,
    [...feed, ...threadReplies, ...(threadRoot ? [threadRoot] : [])].some(
      (row) => !row.display_data,
    ),
  );
  const attachmentsByMessage = useMemo(
    () => groupAttachmentsByMessage(attachmentsQuery.data ?? []),
    [attachmentsQuery.data],
  );

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [isJoining, setIsJoining] = useState(false);

  const isArchivedMessage = useCallback(
    (id: string) => {
      const message = messagesCollection.get(id);
      const rootId = message?.parent_id ?? id;
      return (
        !!messagesCollection.get(rootId)?.thread_archive_at ||
        (threadIsArchived &&
          (id === threadRootId ||
            message?.parent_id === threadRootId ||
            threadReplies.some((reply) => reply.id === id)))
      );
    },
    [messagesCollection, threadIsArchived, threadRootId, threadReplies],
  );

  useEffect(() => {
    if (editingId && isArchivedMessage(editingId)) {
      setEditingId(null);
      setEditingText("");
    }
    if (pendingDeleteId && isArchivedMessage(pendingDeleteId)) setPendingDeleteId(null);
    if (reactionMessageId && isArchivedMessage(reactionMessageId)) setReactionMessageId(null);
  }, [editingId, pendingDeleteId, reactionMessageId, isArchivedMessage]);

  const handleReaction = useCallback(
    async (messageId: string, emoji: string) => {
      if (
        !currentUserId ||
        isArchivedMessage(messageId) ||
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
    [currentUserId, messagesCollection, reactionsCollection, run, isArchivedMessage],
  );

  // 通知・リンクの対象 (`?message=`)。メイン一覧の範囲とは関係なく、その行だけを取る。
  const targetQuery = useMessageById(channelId, targetMessageId ?? null);
  const target = targetQuery.data?.[0];

  // 対象が返信なら、そのスレッドを開く。
  useEffect(() => {
    if (target?.parent_id && !target.deleted_at) openThread(target.parent_id);
  }, [target?.id, target?.parent_id, target?.deleted_at, openThread]);

  // 対象がトップレベルで、いま読み込んでいる範囲より古いときは、含まれるまで 1 ページずつ古い方へ広げる
  // (連続した範囲になるので、間のメッセージが欠けない)。広げる回数には上限がある。
  const revealedPages = useRef({ id: "", pages: 0 });
  const { hasOlder, isLoadingOlder, loadOlder } = messagesQuery;
  useEffect(() => {
    if (!target || target.deleted_at || target.parent_id) return;
    if (revealedPages.current.id !== target.id) revealedPages.current = { id: target.id, pages: 0 };
    if (feed.some((row) => row.id === target.id)) return;
    if (!hasOlder || isLoadingOlder) return;
    if (revealedPages.current.pages >= MAX_REVEAL_PAGES) {
      toast.info(t("threads.targetTooOld"), { id: "target-too-old" });
      return;
    }
    revealedPages.current.pages += 1;
    void loadOlder();
  }, [target, feed, hasOlder, isLoadingOlder, loadOlder, t]);

  // 対象がスレッドの返信なら、スレッドの範囲も、含まれるまで広げる。
  const revealedThreadPages = useRef({ id: "", pages: 0 });
  const { hasOlder: threadHasOlder, isLoadingOlder: threadLoadingOlder } = threadQuery;
  const loadOlderReplies = threadQuery.loadOlder;
  useEffect(() => {
    if (!target || target.deleted_at || !target.parent_id || target.parent_id !== threadRootId) {
      return;
    }
    if (revealedThreadPages.current.id !== target.id) {
      revealedThreadPages.current = { id: target.id, pages: 0 };
    }
    if (threadReplies.some((row) => row.id === target.id)) return;
    if (!threadHasOlder || threadLoadingOlder) return;
    if (revealedThreadPages.current.pages >= MAX_REVEAL_PAGES) {
      toast.info(t("threads.targetTooOld"), { id: "target-too-old" });
      return;
    }
    revealedThreadPages.current.pages += 1;
    void loadOlderReplies();
  }, [
    target,
    threadRootId,
    threadReplies,
    threadHasOlder,
    threadLoadingOlder,
    loadOlderReplies,
    t,
  ]);

  // スレッドの親が (他ユーザーの操作などで) 見えなくなったらスレッドも閉じる。
  // 論理削除なら、返信が残っていることを利用者に伝えてから閉じる。親は、メイン一覧の範囲とは関係なく、
  // ID で取る。取得が終わるまでは判断しない。
  useEffect(() => {
    if (!threadRootId || threadRootQuery.isLoading || threadRootQuery.isError) {
      return;
    }
    if (threadRootRow && !threadRootRow.deleted_at) return;
    closeThread();
    if (threadRootRow?.deleted_at) {
      toast.info(t("threads.rootDeleted"), { id: "thread-root-deleted" });
    }
  }, [
    threadRootId,
    threadRootRow,
    threadRootQuery.isLoading,
    threadRootQuery.isError,
    closeThread,
    t,
  ]);

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
        refreshAppSurfaces();
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
      sensitiveAttachmentIds: string[] = [],
    ): Promise<boolean> => {
      if (attachmentIds.length === 0) {
        const commandResult = await runCommand(content, commandAppId);
        if (commandResult !== null) return commandResult;
      }
      const id = crypto.randomUUID();
      const metadata =
        attachmentIds.length > 0
          ? {
              attachment_ids: attachmentIds,
              ...(sensitiveAttachmentIds.length
                ? { sensitive_attachment_ids: sensitiveAttachmentIds }
                : {}),
            }
          : undefined;
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
      sensitiveAttachmentIds: string[] = [],
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
      const metadata =
        attachmentIds.length > 0
          ? {
              attachment_ids: attachmentIds,
              ...(sensitiveAttachmentIds.length
                ? { sensitive_attachment_ids: sensitiveAttachmentIds }
                : {}),
            }
          : undefined;
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
    if (!id || !content || isArchivedMessage(id)) {
      return;
    }
    setEditingId(null);
    setEditingText("");
    await run(() =>
      messagesCollection.update(id, (message) => {
        message.content = content;
      }),
    );
  }, [editingId, editingText, run, messagesCollection, isArchivedMessage]);

  const handleConfirmDelete = useCallback(async () => {
    const id = pendingDeleteId;
    setPendingDeleteId(null);
    if (!id || isArchivedMessage(id)) {
      return;
    }
    // 論理削除は `deleted_at` の update。Electric が流す operation と揃えて
    // delete→insert の再出現（ちらつき）を避ける（Phase 3 の扱いを踏襲）。
    await run(() =>
      messagesCollection.update(id, (message) => {
        message.deleted_at = nowIso();
      }),
    );
  }, [pendingDeleteId, run, messagesCollection, isArchivedMessage]);

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
          onOpenThread={openThread}
          canCompose={isMember}
          isOffline={!isOnline}
          hasOlder={messagesQuery.hasOlder}
          isLoadingOlder={messagesQuery.isLoadingOlder}
          onLoadOlder={() => void messagesQuery.loadOlder()}
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
              // 最初のページが届く前の書き込みは、確認 (txid) を待つ間、永続化層が
              // 最初のページの取得を止めるため、互いに待ち合って失敗する。
              disabled={readOnly || !isOnline || messagesQuery.isLoading}
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
          isArchived={threadIsArchived}
          isMember={!readOnly && isMember}
          isOffline={!isOnline}
          typingUsers={threadTyping.users}
          onTyping={threadTyping.onTyping}
          root={threadRoot}
          replies={threadReplies}
          onOpenThread={openThread}
          isLoading={threadQuery.isLoading}
          isError={threadQuery.isError}
          currentUserId={readOnly || threadIsArchived ? null : currentUserId}
          canModerate={canModerate && !threadIsArchived}
          reactions={reactionGroups}
          attachments={attachmentsByMessage}
          onUploadAttachment={uploadAttachment}
          kitchenCatalog={catalog}
          canReact={
            !readOnly &&
            !threadIsArchived &&
            !!currentUserId &&
            !reactionsQuery.isLoading &&
            !reactionsQuery.isError
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
          hasOlder={threadQuery.hasOlder}
          isLoadingOlder={threadQuery.isLoadingOlder}
          onLoadOlder={() => void threadQuery.loadOlder()}
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
