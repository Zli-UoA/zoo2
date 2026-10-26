import { useTranslation } from "react-i18next";
import { XIcon } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";

import { useLocale } from "../../i18n/LocaleProvider";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { Avatar, AvatarFallback } from "../../ui/avatar";
import { Button } from "../../ui/button";
import { initials } from "../format";
import type { TypingUser } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { MessageBody, MessageRow } from "./MessageRow";
import { authorLabel, formatTime, fullTimestamp, type FeedMessage } from "./messageFormat";
import { buildMessageItems } from "./messageGrouping";
import { isNearBottom } from "./messageScroll";

export interface ThreadPaneProps {
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
  isLoading: boolean;
  isError: boolean;
  currentUserId: string | null;
  reactions: Map<string, ReactionGroup[]>;
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
  onSendReply: (content: string) => Promise<boolean>;
  /** 上端付近で古い返信を読み足す。 */
  onLoadOlder?: () => void;
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
  targetMessageId,
  mentionUsers,
  mentionMemberIds,
  isMember = true,
  isOffline = false,
  typingUsers = [],
  onTyping,
  root,
  replies,
  isLoading,
  isError,
  currentUserId,
  reactions,
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
  onSendReply,
  onLoadOlder,
  onRetry,
  onClose,
}: ThreadPaneProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const items = useMemo(() => buildMessageItems(replies), [replies]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);

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
  }, [items.length, root.id]);

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
      <header className="flex items-center gap-2 border-b px-4 py-2.5">
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{t("threads.title")}</h2>
          <p className="text-xs tabular-nums text-muted-foreground">
            {isLoading ? t("threads.loading") : t("threads.replyCount", { count: replies.length })}
          </p>
        </div>
        <Button variant="ghost" size="icon-sm" aria-label={t("threads.close")} onClick={onClose}>
          <XIcon />
        </Button>
      </header>

      <div className="border-b bg-muted/30 px-4 py-3">
        <div className="flex gap-3">
          <Avatar size="sm" className="mt-0.5">
            <AvatarFallback>{initials(authorLabel(root))}</AvatarFallback>
          </Avatar>
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              <span className="truncate text-sm font-semibold">{authorLabel(root)}</span>
              <time
                className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
                dateTime={root.created_at}
                title={fullTimestamp(root.created_at, locale)}
              >
                {formatTime(root.created_at, locale)}
              </time>
            </div>
            <div className="max-h-40 overflow-y-auto">
              <MessageBody content={root.content} card={root.card} />
            </div>
          </div>
        </div>
      </div>

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 py-2"
        onScroll={(event) => {
          const element = event.currentTarget;
          atBottomRef.current = isNearBottom(
            element.scrollTop,
            element.scrollHeight,
            element.clientHeight,
          );
          if (element.scrollTop <= 48) {
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
                  isEditing={editingId === item.message.id}
                  editingText={editingText}
                  onEditingTextChange={onEditingTextChange}
                  onStartEdit={() => onStartEdit(item.message.id, item.message.content)}
                  onCancelEdit={onCancelEdit}
                  onSaveEdit={onSaveEdit}
                  onRequestDelete={() => onRequestDelete(item.message.id)}
                  reactions={reactions.get(item.message.id) ?? []}
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

      <TypingIndicator users={typingUsers} />
      <Composer
        mentionUsers={mentionUsers}
        mentionMemberIds={mentionMemberIds}
        disabled={!isMember || isOffline}
        onTyping={onTyping}
        onSend={onSendReply}
        placeholder={isOffline ? t("offline.composeDisabled") : t("threads.compose")}
        ariaLabel={t("threads.compose")}
      />
    </aside>
  );
}
