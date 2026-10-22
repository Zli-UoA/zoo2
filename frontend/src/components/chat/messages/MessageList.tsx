import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { newMessageAnnouncement, type FeedMessage } from "./messageFormat";
import { buildMessageItems } from "./messageGrouping";
import { MessageRow } from "./MessageRow";
import { useMessageReadState } from "./useMessageReadState";
import {
  classifyScrollChange,
  isNearBottom,
  shouldStickToBottom,
  snapshot,
  type ScrollSnapshot,
} from "./messageScroll";

const ESTIMATED_ROW_HEIGHT = 64;
const SKELETON_ROWS = [0, 1, 2, 3, 4, 5];

interface MessageListProps {
  channelId?: string;
  messages: FeedMessage[];
  isLoading: boolean;
  isWritePending?: boolean;
  isError: boolean;
  currentUserId: string | null;
  editingId: string | null;
  editingText: string;
  onEditingTextChange: (value: string) => void;
  onStartEdit: (id: string, content: string) => void;
  onCancelEdit: () => void;
  onSaveEdit: () => void;
  onRequestDelete: (id: string) => void;
  reactions: Map<string, ReactionGroup[]>;
  kitchenCatalog: KitchenCatalog | null;
  canReact: boolean;
  onReaction: (messageId: string, emoji: string) => void;
  onPickReaction: (messageId: string) => void;
  replyCounts: Map<string, number>;
  onOpenThread: (messageId: string) => void;
  /** 送信できる状態か。false なら空状態で「最初のメッセージ」を促さない。 */
  canCompose?: boolean;
}

function MessageSkeleton() {
  return (
    <div className="flex flex-col gap-4 p-4" aria-hidden="true">
      {SKELETON_ROWS.map((row) => (
        <div key={row} className="flex gap-3">
          <Skeleton className="size-9 shrink-0 rounded-full" />
          <div className="flex flex-1 flex-col gap-2">
            <Skeleton className="h-3 w-28" />
            <Skeleton className="h-4 w-full max-w-md" />
          </div>
        </div>
      ))}
    </div>
  );
}

function MessageState({
  title,
  description,
  destructive = false,
}: {
  title: string;
  description?: string;
  destructive?: boolean;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-1 p-6 text-center">
      <p className={destructive ? "text-sm font-medium text-destructive" : "text-sm font-medium"}>
        {title}
      </p>
      {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
    </div>
  );
}

/**
 * メッセージ一覧。日付区切りと連投グルーピングを描き、`@tanstack/react-virtual`
 * で可視行のみ DOM 化する。新規受信は atBottom のときだけ追従し、古いページの
 * 追加ではスクロール位置を補正して維持する。最下部にいないときの新着は
 * 「新着メッセージ」ボタンに加えて `aria-live` でも読み上げる。
 */
export function MessageList({
  channelId,
  messages,
  isLoading,
  isWritePending = false,
  isError,
  currentUserId,
  editingId,
  editingText,
  onEditingTextChange,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onRequestDelete,
  reactions,
  kitchenCatalog,
  canReact,
  onReaction,
  onPickReaction,
  replyCounts,
  onOpenThread,
  canCompose = true,
}: MessageListProps) {
  const { t } = useTranslation();
  const {
    firstUnreadId,
    markRead,
    isLoading: readStateLoading,
    isError: readStateError,
  } = useMessageReadState(channelId, currentUserId, messages, isLoading || isError, isWritePending);
  const items = useMemo(
    () =>
      readStateLoading || readStateError
        ? []
        : buildMessageItems(messages, undefined, firstUnreadId),
    [messages, firstUnreadId, readStateLoading, readStateError],
  );
  const scrollRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_HEIGHT,
    overscan: 8,
    getItemKey: (index) => items[index]?.message.id ?? index,
  });

  const atBottomRef = useRef(true);
  const [hasNewBelow, setHasNewBelow] = useState(false);
  // 最下部にいないとき届いた新着を `aria-live` で読み上げる（「新着メッセージ」
  // ボタンと同じ条件）。最下部へ戻ったら空にして、同文の連投も再度読み上げられる。
  const [liveAnnouncement, setLiveAnnouncement] = useState("");
  const snapshotRef = useRef<ScrollSnapshot>({ firstId: null, lastId: null });
  const previousScrollHeightRef = useRef(0);

  const scrollToBottom = useCallback(
    (behavior: ScrollBehavior = "auto") => {
      atBottomRef.current = true;
      setHasNewBelow(false);
      if (items.length === 0) {
        return;
      }
      // 行の実測高さに追従して最下部へ合わせる（scrollHeight 直指定より安定）。
      virtualizer.scrollToIndex(items.length - 1, { align: "end", behavior });
    },
    [items.length, virtualizer],
  );

  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const near = isNearBottom(element.scrollTop, element.scrollHeight, element.clientHeight);
    atBottomRef.current = near;
    if (near) {
      setHasNewBelow(false);
      markRead();
    }
  }, [markRead]);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const ids = items.map((item) => item.message.id);
    const next = snapshot(ids);
    const change = classifyScrollChange(snapshotRef.current, next, new Set(ids));

    if (change === "prepend") {
      const delta = element.scrollHeight - previousScrollHeightRef.current;
      if (delta > 0) {
        element.scrollTop += delta;
      }
    } else if (change === "initial" && firstUnreadId) {
      const index = items.findIndex((item) => item.message.id === firstUnreadId);
      atBottomRef.current = false;
      requestAnimationFrame(() => virtualizer.scrollToIndex(index, { align: "start" }));
    } else if (shouldStickToBottom(change, atBottomRef.current)) {
      requestAnimationFrame(() => scrollToBottom());
    } else if (change === "append") {
      // 自分の送信は、上にスクロールしていても最下部へ追従する。
      const appended = items[items.length - 1];
      if (appended && appended.message.user_id === currentUserId) {
        requestAnimationFrame(() => scrollToBottom());
      } else {
        setHasNewBelow(true);
        if (appended) {
          setLiveAnnouncement(newMessageAnnouncement(appended.message, t));
        }
      }
    }

    snapshotRef.current = next;
    previousScrollHeightRef.current = element.scrollHeight;
  }, [items, currentUserId, scrollToBottom, t, firstUnreadId, virtualizer]);

  useEffect(() => {
    const update = () => {
      const element = scrollRef.current;
      if (element && isNearBottom(element.scrollTop, element.scrollHeight, element.clientHeight)) {
        markRead();
      }
    };
    const frame = requestAnimationFrame(update);
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, [markRead]);

  // 編集の開始・終了で行の高さが変わるため測り直す。
  useEffect(() => {
    virtualizer.measure();
  }, [editingId, virtualizer]);

  useEffect(() => {
    if (!hasNewBelow) {
      setLiveAnnouncement("");
    }
  }, [hasNewBelow]);

  const showSkeleton = (isLoading || readStateLoading) && items.length === 0;
  const showError = (isError || readStateError) && items.length === 0;
  const showEmpty =
    !isLoading && !readStateLoading && !isError && !readStateError && items.length === 0;

  return (
    <div className="relative min-h-0 flex-1">
      {showSkeleton ? (
        <MessageSkeleton />
      ) : showError ? (
        <MessageState
          destructive
          title={t("messages.loadError")}
          description={t("messages.retryHelp")}
        />
      ) : showEmpty ? (
        <MessageState
          title={t("messages.empty")}
          description={canCompose ? t("messages.first") : undefined}
        />
      ) : (
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="h-full overflow-y-auto overscroll-contain px-2 py-3"
        >
          <div
            role="list"
            className="relative w-full"
            style={{ height: virtualizer.getTotalSize() }}
          >
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const item = items[virtualRow.index];
              if (!item) {
                return null;
              }
              return (
                <div
                  key={virtualRow.key}
                  ref={virtualizer.measureElement}
                  data-index={virtualRow.index}
                  role="listitem"
                  className="absolute left-0 top-0 w-full"
                  style={{ transform: `translateY(${virtualRow.start}px)` }}
                >
                  {item.message.id === firstUnreadId ? (
                    <div
                      role="separator"
                      aria-label="new"
                      className="flex items-center gap-2 py-2 text-xs font-semibold text-destructive"
                    >
                      <span className="h-px flex-1 bg-destructive" />
                      <span>new</span>
                      <span className="h-px w-6 bg-destructive" />
                    </div>
                  ) : null}
                  <MessageRow
                    item={item}
                    reactions={reactions.get(item.message.id) ?? []}
                    kitchenCatalog={kitchenCatalog}
                    canReact={canReact}
                    onReaction={(emoji) => onReaction(item.message.id, emoji)}
                    onPickReaction={() => onPickReaction(item.message.id)}
                    isOwn={!!currentUserId && item.message.user_id === currentUserId}
                    isEditing={editingId === item.message.id}
                    editingText={editingText}
                    onEditingTextChange={onEditingTextChange}
                    onStartEdit={() => onStartEdit(item.message.id, item.message.content)}
                    onCancelEdit={onCancelEdit}
                    onSaveEdit={onSaveEdit}
                    onRequestDelete={() => onRequestDelete(item.message.id)}
                    replyCount={replyCounts.get(item.message.id) ?? 0}
                    onOpenThread={() => onOpenThread(item.message.id)}
                  />
                </div>
              );
            })}
          </div>
        </div>
      )}

      {hasNewBelow ? (
        <Button
          size="sm"
          className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full shadow-md"
          onClick={() => scrollToBottom("smooth")}
        >
          <ArrowDownIcon />
          {t("messages.new")}
        </Button>
      ) : null}

      <p role="status" aria-live="polite" aria-atomic="true" className="sr-only">
        {liveAnnouncement}
      </p>
    </div>
  );
}
