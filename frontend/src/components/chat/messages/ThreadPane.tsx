import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, XIcon } from "lucide-react";
import { type KeyboardEvent, useEffect, useLayoutEffect, useMemo, useRef } from "react";

import type { CommandSuggestion } from "./composerSuggestions";
import type { UploadedAttachment } from "../../../lib/api/attachments";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { AttachmentRow } from "../../../lib/electric/types";
import { groupReactions, type ReactionGroup } from "../../../lib/emoji/reactions";
import { Button } from "../../ui/button";
import { PaneHeader } from "../../layout/PaneHeader";
import { useMediaQuery } from "../../layout/useMediaQuery";

import type { TypingUser } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { MessageRow } from "./MessageRow";
import type { FeedMessage } from "./messageFormat";
import { buildMessageItems } from "./messageGrouping";
import { isNearBottom } from "./messageScroll";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

export interface ThreadPaneProps {
  commands?: readonly CommandSuggestion[];
  onRequestCommands?: () => void;
  targetMessageId?: string;
  mentionUsers?: import("../../../lib/electric/types").UserRow[];
  mentionMemberIds?: ReadonlySet<string>;
  isMember?: boolean;
  /** オフラインか。返信がないときの案内と composer を切り替える。 */
  isOffline?: boolean;
  /** 返信を送れない理由（暗号化チャンネルで暗号化の送信がまだ使えない、など）。指定すると入力欄を止める。 */
  composeDisabledReason?: string;
  typingUsers?: TypingUser[];
  onTyping?: (active: boolean) => void;
  root: FeedMessage;
  replies: FeedMessage[];
  membership?: "none" | "joined" | "left";
  membershipPending?: boolean;
  membershipError?: boolean;
  onMembershipRetry?: () => void;
  onJoinThread?: () => void;
  onLeaveThread?: () => void;
  onReopenThread?: () => void;
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
  ) => Promise<boolean>;
  /** 上端付近で古い返信を読み足す。 */
  onLoadOlder?: () => void;
  /** 端末より古い返信が、まだあるか。 */
  hasOlder?: boolean;
  /** 古いページを取っている最中か。 */
  isLoadingOlder?: boolean;
  /** 取得失敗時の再試行。 */
  onRetry?: () => void;
  /** Read existing nested threads until their staged migration completes. */
  onOpenThread?: (id: string) => void;
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
  onOpenThread,
  targetMessageId,
  mentionUsers,
  mentionMemberIds,
  isMember = true,
  isOffline = false,
  composeDisabledReason,
  typingUsers = [],
  onTyping,
  root,
  replies,
  membership,
  membershipPending = false,
  membershipError = false,
  onMembershipRetry,
  onJoinThread,
  onLeaveThread,
  onReopenThread,
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
  // 狭幅（sm 未満）は absolute のオーバーレイ。ダイアログとして扱うのはこのときだけ。
  // 広幅の sm:static なサイドペインは従来どおりインラインのままにする。
  const isWidePane = useMediaQuery("(min-width: 640px)");
  const items = useMemo(() => buildMessageItems(replies), [replies]);
  const paneRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
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

  // 幅が変わっても同じ要素ツリーを保つため、ダイアログの属性とフォーカス制御は
  // この要素にだけ付け外しする（Composer の下書きやスクロール位置を失わないため）。
  useEffect(() => {
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => restoreFocusRef.current?.focus();
  }, []);

  useEffect(() => {
    if (isWidePane) return;
    // 広幅から狭幅へ戻ったとき、フォーカスが背後に残っていたらペイン内へ移す。
    // すでにペイン内にある場合はユーザーのフォーカス位置を維持する。
    const pane = paneRef.current;
    if (pane && !pane.contains(document.activeElement)) pane.focus();
  }, [isWidePane]);

  function handlePaneKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (isWidePane) return;
    // 削除確認やリアクションピッカーなど、子のダイアログが Escape を処理する場合は干渉しない。
    if ((event.target as HTMLElement).closest('[role="dialog"]') !== event.currentTarget) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusables = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
    ).filter(
      (element) =>
        element.getClientRects().length > 0 &&
        !element.hasAttribute("disabled") &&
        element.getAttribute("aria-disabled") !== "true",
    );
    if (focusables.length === 0) {
      event.preventDefault();
      event.currentTarget.focus();
      return;
    }
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (document.activeElement === event.currentTarget) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    } else if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  const paneClassName =
    "absolute inset-0 z-20 flex min-h-0 min-w-0 flex-col border-l bg-background sm:static sm:z-auto sm:w-80 sm:shrink-0 xl:w-96";

  const paneContent = (
    <>
      <PaneHeader className="gap-2" role="none">
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-11 sm:hidden"
          aria-label={t("threads.close")}
          onClick={onClose}
        >
          <ArrowLeftIcon />
        </Button>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{t("threads.title")}</h2>
          <p className="text-xs tabular-nums text-muted-foreground">
            {isLoading
              ? t("threads.loading")
              : t("threads.replyCount", {
                  count: root.display_data?.reply_count ?? replies.length,
                })}
          </p>
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

      {!readOnly && isMember ? (
        <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2">
          {isArchived && onReopenThread ? (
            <Button
              variant="outline"
              size="sm"
              disabled={membershipPending || isOffline}
              onClick={onReopenThread}
            >
              {t("threads.reopen")}
            </Button>
          ) : null}
          {membership === "joined" && onLeaveThread ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={membershipPending || isOffline}
              onClick={onLeaveThread}
            >
              {t("threads.leave")}
            </Button>
          ) : !isArchived && onJoinThread ? (
            <Button
              variant="outline"
              size="sm"
              disabled={membership === undefined || membershipPending || isOffline}
              onClick={onJoinThread}
            >
              {t("threads.join")}
            </Button>
          ) : null}
          {membershipError ? (
            <p role="alert" className="text-xs text-destructive">
              {t("threads.membershipError")}{" "}
              <Button variant="ghost" size="sm" onClick={onMembershipRetry}>
                {t("offline.retry")}
              </Button>
            </p>
          ) : null}
        </div>
      ) : null}
      <div className="max-h-60 shrink-0 overflow-y-auto border-b bg-muted/30 px-2 py-2">
        <MessageRow
          item={{ message: root, isGroupStart: true, isGroupEnd: true, showDateSeparator: false }}
          isOwn={!!currentUserId && root.user_id === currentUserId}
          canDelete={canModerate || (!!currentUserId && root.user_id === currentUserId)}
          isEditing={editingId === root.id}
          editingText={editingText}
          onEditingTextChange={onEditingTextChange}
          onStartEdit={() => onStartEdit(root.id, root.content)}
          onCancelEdit={onCancelEdit}
          onSaveEdit={onSaveEdit}
          onRequestDelete={() => onRequestDelete(root.id)}
          reactions={
            root.display_data
              ? (groupReactions(root.display_data.reactions, currentUserId).get(root.id) ?? [])
              : (reactions.get(root.id) ?? [])
          }
          attachments={root.display_data?.attachments ?? attachments?.get(root.id) ?? []}
          kitchenCatalog={kitchenCatalog}
          canReact={canReact}
          onReaction={(emoji) => onReaction(root.id, emoji)}
          onPickReaction={() => onPickReaction(root.id)}
        />
      </div>

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
                  onOpenThread={
                    (item.message.display_data?.reply_count ?? 0) > 0 && onOpenThread
                      ? () => onOpenThread(item.message.id)
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
          {isArchived ? (
            <p className="px-3 py-2 text-xs text-muted-foreground">{t("threads.reopenOnSend")}</p>
          ) : null}
          <TypingIndicator users={typingUsers} />
          <Composer
            commands={commands}
            onRequestCommands={onRequestCommands}
            mentionUsers={mentionUsers}
            mentionMemberIds={mentionMemberIds}
            disabled={!isMember || isOffline || composeDisabledReason !== undefined}
            onTyping={onTyping}
            onSend={onSendReply}
            onUploadAttachment={onUploadAttachment}
            placeholder={
              composeDisabledReason ??
              (isOffline ? t("offline.composeDisabled") : t("threads.compose"))
            }
            ariaLabel={t("threads.compose")}
          />
        </>
      ) : null}
    </>
  );

  return (
    <div
      ref={paneRef}
      role={isWidePane ? "complementary" : "dialog"}
      aria-modal={isWidePane ? undefined : true}
      aria-label={t("threads.title")}
      tabIndex={-1}
      className={paneClassName}
      onKeyDown={handlePaneKeyDown}
    >
      {paneContent}
    </div>
  );
}
