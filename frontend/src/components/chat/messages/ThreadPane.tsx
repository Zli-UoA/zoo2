import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, XIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { CommandSuggestion } from "./composerSuggestions";
import type { UploadedAttachment } from "../../../lib/api/attachments";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { AttachmentRow } from "../../../lib/electric/types";
import { groupReactions, type ReactionGroup } from "../../../lib/emoji/reactions";
import { Button } from "../../ui/button";
import { PaneHeader } from "../../layout/PaneHeader";
import { AppAvatar, AuthorIdentity } from "../AppIdentity";
import { MessageAppContent, hasCardBlocks } from "../AppBlocks";

import type { TypingUser } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { MessageBody, MessageRow } from "./MessageRow";
import { authorLabel, type FeedMessage } from "./messageFormat";
import { MfmContent } from "./MfmContent";
import { buildMessageItems } from "./messageGrouping";
import { isNearBottom } from "./messageScroll";

export interface ThreadPaneProps {
  commands?: readonly CommandSuggestion[];
  onRequestCommands?: () => void;
  targetMessageId?: string;
  mentionUsers?: import("../../../lib/electric/types").UserRow[];
  mentionMemberIds?: ReadonlySet<string>;
  isMember?: boolean;
  /** オフラインか。返信がないときの案内と composer を切り替える。 */
  isOffline?: boolean;
  typingUsers?: TypingUser[];
  onTyping?: (active: boolean) => void;
  root: FeedMessage;
  replies: FeedMessage[];
  onNavigateToMessage?: (messageId: string) => void;
  unavailableReplyTargetId?: string;
  isLoading: boolean;
  isError: boolean;
  currentUserId: string | null;
  canModerate?: boolean;
  isArchived?: boolean;
  readOnly?: boolean;
  reactions: Map<string, ReactionGroup[]>;
  attachments?: Map<string, AttachmentRow[]>;
  kitchenCatalog: KitchenCatalog | null;
  canReact: boolean;
  onReaction: (messageId: string, emoji: string) => void;
  onPickReaction: (messageId: string) => void;
  editingId: string | null;
  editingText: string;
  onEditingTextChange: (value: string) => void;
  onStartEdit: (id: string, content: string) => void;
  onCancelEdit: () => void;
  onSaveEdit: () => void;
  onRequestDelete: (id: string) => void;
  onUploadAttachment?: (file: File) => Promise<UploadedAttachment>;
  onSendReply: (
    content: string,
    commandAppId?: string,
    attachmentIds?: string[],
    sensitiveAttachmentIds?: string[],
    replyToMessageId?: string,
  ) => Promise<boolean>;
  /** 上端付近で古い返信を読み足す。 */
  onLoadOlder?: () => void;
  /** 端末より古い返信が、まだあるか。 */
  hasOlder?: boolean;
  /** 古いページを取っている最中か。 */
  isLoadingOlder?: boolean;
  /** 取得失敗時の再試行。 */
  onRetry?: () => void;
  onClose: () => void;
}

/**
 * ルートメッセージと返信一覧を出すスレッドペイン。
 *
 * 返信は専用 Shape ではなく、メインと同じ messages コレクションを
 * `parent_id = <root>` で絞った live query の結果を受け取る。送信も通常の
 * `messages.insert`（`parent_id` 付き）で、txid の確定は MessagePane が待つ。
 */
export function ThreadPane({
  commands,
  onRequestCommands,
  targetMessageId,
  mentionUsers,
  mentionMemberIds,
  isMember = true,
  isOffline = false,
  typingUsers = [],
  onTyping,
  root,
  replies,
  onNavigateToMessage,
  unavailableReplyTargetId,
  isLoading,
  isError,
  currentUserId,
  canModerate = false,
  isArchived = false,
  readOnly = false,
  reactions,
  attachments,
  kitchenCatalog,
  canReact,
  onReaction,
  onPickReaction,
  editingId,
  editingText,
  onEditingTextChange,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onRequestDelete,
  onUploadAttachment,
  onSendReply,
  onLoadOlder,
  hasOlder = true,
  isLoadingOlder = false,
  onRetry,
  onClose,
}: ThreadPaneProps) {
  const { t } = useTranslation();
  const items = useMemo(() => buildMessageItems(replies), [replies]);
  const repliesById = useMemo(() => new Map(replies.map((reply) => [reply.id, reply])), [replies]);
  const [replyTargetId, setReplyTargetId] = useState<string | null>(null);
  const replyTarget = replyTargetId === root.id ? root : repliesById.get(replyTargetId ?? "");
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);
  // 件数が上限のままでも、新着で末尾の返信が変わったら、最下部にいるときは追従する。
  const lastReplyId = replies[replies.length - 1]?.id;

  useEffect(() => {
    atBottomRef.current = true;
  }, [root.id]);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element || !atBottomRef.current) {
      return;
    }
    requestAnimationFrame(() => {
      element.scrollTop = element.scrollHeight;
    });
  }, [items.length, lastReplyId, root.id]);

  // 古い返信のページが先頭に足されたときは、読んでいた位置が動かないよう、増えた高さの分だけ補正する。
  const scrollSnapshot = useRef({ firstId: "", lastId: "", height: 0 });
  useLayoutEffect(() => {
    const element = scrollRef.current;
    const previous = scrollSnapshot.current;
    const firstId = replies[0]?.id ?? "";
    const lastId = replies[replies.length - 1]?.id ?? "";
    if (element) {
      const prepended =
        previous.firstId !== "" &&
        firstId !== previous.firstId &&
        lastId === previous.lastId &&
        !atBottomRef.current;
      if (prepended) element.scrollTop += element.scrollHeight - previous.height;
      scrollSnapshot.current = { firstId, lastId, height: element.scrollHeight };
    }
  }, [replies]);

  const scrolledTarget = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!targetMessageId) {
      scrolledTarget.current = undefined;
      return;
    }
    if (scrolledTarget.current === targetMessageId) return;
    if (!replies.some((reply) => reply.id === targetMessageId)) return;
    const frame = requestAnimationFrame(() => {
      const target = Array.from(
        scrollRef.current?.querySelectorAll<HTMLElement>("[data-message-id]") ?? [],
      ).find((element) => element.dataset.messageId === targetMessageId);
      if (target && scrollRef.current) {
        atBottomRef.current = false;
        scrolledTarget.current = targetMessageId;
        scrollRef.current.scrollTop = target.offsetTop - scrollRef.current.offsetTop;
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [targetMessageId, replies]);

  return (
    <aside
      aria-label={t("threads.title")}
      className="absolute inset-0 z-20 flex min-h-0 min-w-0 flex-col border-l bg-background sm:static sm:z-auto sm:w-80 sm:shrink-0 xl:w-96"
    >
      <PaneHeader className="h-auto min-h-11 gap-2 py-1">
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-11 sm:hidden"
          aria-label={t("threads.close")}
          onClick={onClose}
        >
          <ArrowLeftIcon />
        </Button>
        <AppAvatar
          size="sm"
          className="size-4"
          label={authorLabel(root)}
          isBot={!!root.is_bot}
          src={root.avatar_url}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-baseline gap-1.5">
            <AuthorIdentity message={root} />
            {!root.card && !hasCardBlocks(root.app_blocks) ? (
              <div className="min-w-0 flex-1 truncate text-xs text-foreground/80">
                <MfmContent content={root.content} emojis={root.display_data?.emojis} />
              </div>
            ) : null}
          </div>
        </div>
        {isArchived ? (
          <span className="text-xs text-muted-foreground">{t("moderation.archived")}</span>
        ) : null}
        <Button
          variant="ghost"
          size="icon-sm"
          className="hidden sm:inline-flex"
          aria-label={t("threads.close")}
          onClick={onClose}
        >
          <XIcon />
        </Button>
      </PaneHeader>

      {root.card ? (
        <div className="shrink-0 border-b px-3 py-2">
          <MessageBody
            emojis={root.display_data?.emojis}
            content={root.content}
            card={root.card}
            appBlocks={root.app_blocks}
            editedAt={root.updated_at}
          />
        </div>
      ) : null}
      <MessageAppContent message={root} />

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 py-2"
        onScroll={(event) => {
          const element = event.currentTarget;
          scrollSnapshot.current.height = element.scrollHeight;
          atBottomRef.current = isNearBottom(
            element.scrollTop,
            element.scrollHeight,
            element.clientHeight,
          );
          if (element.scrollTop <= 48 && hasOlder && !isLoadingOlder) {
            onLoadOlder?.();
          }
        }}
      >
        {isLoading && replies.length === 0 ? (
          <p className="p-3 text-xs text-muted-foreground">{t("threads.repliesLoading")}</p>
        ) : isOffline && replies.length === 0 ? (
          <div className="flex flex-col items-start gap-2 p-3">
            <p className="text-xs text-muted-foreground">{t("offline.emptyTitle")}</p>
            {onRetry ? (
              <Button variant="outline" size="sm" onClick={onRetry}>
                {t("offline.retry")}
              </Button>
            ) : null}
          </div>
        ) : isError && replies.length === 0 ? (
          <div className="flex flex-col items-start gap-2 p-3">
            <p className="text-xs text-destructive">{t("threads.loadError")}</p>
            {onRetry ? (
              <Button variant="outline" size="sm" onClick={onRetry}>
                {t("offline.retry")}
              </Button>
            ) : null}
          </div>
        ) : replies.length === 0 ? (
          <p className="p-3 text-xs text-muted-foreground">{t("threads.empty")}</p>
        ) : (
          <div role="list">
            {items.map((item) => (
              <div
                key={item.message.id}
                data-message-id={item.message.id}
                role="listitem"
                className={
                  item.message.id === targetMessageId
                    ? "rounded-lg bg-brand-muted/40 ring-1 ring-brand"
                    : undefined
                }
              >
                <MessageRow
                  item={item}
                  isOwn={!!currentUserId && item.message.user_id === currentUserId}
                  canDelete={
                    canModerate || (!!currentUserId && item.message.user_id === currentUserId)
                  }
                  isEditing={editingId === item.message.id}
                  editingText={editingText}
                  onEditingTextChange={onEditingTextChange}
                  onStartEdit={() => onStartEdit(item.message.id, item.message.content)}
                  onCancelEdit={onCancelEdit}
                  onSaveEdit={onSaveEdit}
                  onRequestDelete={() => onRequestDelete(item.message.id)}
                  replyCount={item.message.display_data?.reply_count ?? 0}
                  onReplyToMessage={
                    !readOnly && isMember && !item.message.deleted_at
                      ? () => setReplyTargetId(item.message.id)
                      : undefined
                  }
                  onNavigateToMessage={onNavigateToMessage}
                  unavailableReplyTargetId={unavailableReplyTargetId}
                  replyTarget={
                    item.message.reply_to_message_id
                      ? repliesById.get(item.message.reply_to_message_id)
                      : undefined
                  }
                  reactions={
                    item.message.display_data
                      ? (groupReactions(item.message.display_data.reactions, currentUserId).get(
                          item.message.id,
                        ) ?? [])
                      : (reactions.get(item.message.id) ?? [])
                  }
                  attachments={
                    item.message.display_data?.attachments ??
                    attachments?.get(item.message.id) ??
                    []
                  }
                  kitchenCatalog={kitchenCatalog}
                  canReact={canReact}
                  onReaction={(emoji) => onReaction(item.message.id, emoji)}
                  onPickReaction={() => onPickReaction(item.message.id)}
                />
              </div>
            ))}
          </div>
        )}
      </div>

      {!readOnly ? (
        <>
          <TypingIndicator users={typingUsers} />
          {replyTargetId ? (
            <div className="flex items-center justify-between gap-2 border-t px-3 py-1 text-xs text-muted-foreground">
              <span className="min-w-0 truncate">
                {replyTarget && !replyTarget.deleted_at
                  ? t("home.replyTo", { name: authorLabel(replyTarget) })
                  : t("notifications.messageUnavailable")}
              </span>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2"
                onClick={() => setReplyTargetId(null)}
              >
                {t("common.cancelShort")}
              </Button>
            </div>
          ) : null}
          <Composer
            commands={commands}
            onRequestCommands={onRequestCommands}
            mentionUsers={mentionUsers}
            mentionMemberIds={mentionMemberIds}
            disabled={
              !isMember ||
              isOffline ||
              Boolean(replyTargetId && (!replyTarget || replyTarget.deleted_at))
            }
            onTyping={onTyping}
            onSend={async (content, commandAppId, attachmentIds, sensitiveAttachmentIds) => {
              const sent = replyTargetId
                ? await onSendReply(
                    content,
                    commandAppId,
                    attachmentIds,
                    sensitiveAttachmentIds,
                    replyTargetId,
                  )
                : sensitiveAttachmentIds !== undefined
                  ? await onSendReply(content, commandAppId, attachmentIds, sensitiveAttachmentIds)
                  : attachmentIds !== undefined
                    ? await onSendReply(content, commandAppId, attachmentIds)
                    : commandAppId !== undefined
                      ? await onSendReply(content, commandAppId)
                      : await onSendReply(content);
              if (sent) setReplyTargetId(null);
              return sent;
            }}
            onUploadAttachment={onUploadAttachment}
            placeholder={isOffline ? t("offline.composeDisabled") : t("threads.compose")}
            ariaLabel={t("threads.compose")}
          />
        </>
      ) : null}
    </aside>
  );
}
