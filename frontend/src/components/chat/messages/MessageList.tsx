import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { AttachmentRow } from "../../../lib/electric/types";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { groupReactions } from "../../../lib/emoji/reactions";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { newMessageAnnouncement, type FeedMessage } from "./messageFormat";
import { buildMessageItems } from "./messageGrouping";
import { MessageRow } from "./MessageRow";
import { parseUtcTimestamp } from "../../../lib/timestamps";
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
/** 上端からこの距離まで来たら、過去のメッセージを読み足す。 */
const LOAD_OLDER_THRESHOLD = 64;
/** 未読の区切りを置くために、既読位置まで読み足すページ数の上限。 */
const MAX_UNREAD_PAGES = 20;
/** Shape が無言で止まったときにスケルトン無限表示を避けるための上限。 */
const LOADING_TIMEOUT_MS = 8_000;

interface MessageListProps {
  targetMessageId?: string;
  channelId?: string;
  messages: FeedMessage[];
  isLoading: boolean;
  isWritePending?: boolean;
  isError: boolean;
  currentUserId: string | null;
  canModerate?: boolean;
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
  attachments?: Map<string, AttachmentRow[]>;
  onOpenThread?: (messageId: string) => void;
  onReply?: (messageId: string) => void;
  /** 送信できる状態か。false なら空状態で「最初のメッセージ」を促さない。 */
  canCompose?: boolean;
  /** オフラインか。スケルトンの代わりに保存済みがない旨を出す。 */
  isOffline?: boolean;
  /** 上端付近で過去のメッセージを読み足す。 */
  onLoadOlder?: () => void;
  /** 端末より古いメッセージが、まだあるか (ページ単位で取る)。 */
  hasOlder?: boolean;
  /** 古いページを取っている最中か。 */
  isLoadingOlder?: boolean;
  /** 取得失敗・停止時の再試行。 */
  onRetry?: () => void;
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
  action,
}: {
  title: string;
  description?: string;
  destructive?: boolean;
  action?: ReactNode;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center">
      <p className={destructive ? "text-sm font-medium text-destructive" : "text-sm font-medium"}>
        {title}
      </p>
      {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
      {action}
    </div>
  );
}

/** 読み込みが長時間終わらないときだけ true にする。 */
function useLoadingTimeout(active: boolean, timeoutMs = LOADING_TIMEOUT_MS): boolean {
  const [timedOut, setTimedOut] = useState(false);
  useEffect(() => {
    if (!active) {
      setTimedOut(false);
      return;
    }
    const timer = setTimeout(() => setTimedOut(true), timeoutMs);
    return () => clearTimeout(timer);
  }, [active, timeoutMs]);
  return active && timedOut;
}

/**
 * メッセージ一覧。日付区切りと連投グルーピングを描き、`@tanstack/react-virtual`
 * で可視行のみ DOM 化する。新規受信は atBottom のときだけ追従し、古いページの
 * 追加ではスクロール位置を補正して維持する。最下部にいないときの新着は
 * 「新着メッセージ」ボタンに加えて `aria-live` でも読み上げる。
 */
export function MessageList({
  targetMessageId,
  channelId,
  messages,
  isLoading,
  isWritePending = false,
  isError,
  currentUserId,
  canModerate = false,
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
  attachments,
  onOpenThread,
  onReply,
  canCompose = true,
  isOffline = false,
  onLoadOlder,
  hasOlder = true,
  isLoadingOlder = false,
  onRetry,
}: MessageListProps) {
  const { t } = useTranslation();
  const {
    firstUnreadId,
    lastRead,
    markRead,
    isLoading: readStateLoading,
    isError: readStateError,
  } = useMessageReadState(channelId, currentUserId, messages, isLoading || isError, isWritePending);
  // 古いページを取っている最中や、古いメッセージがないときは、要求しない (同じ要求を重ねない)。
  const loadOlder = useCallback(() => {
    if (hasOlder && !isLoadingOlder) onLoadOlder?.();
  }, [hasOlder, isLoadingOlder, onLoadOlder]);
  // 既読位置の取得に失敗しても、保存済みのメッセージは隠さない。
  // 未読区切りだけを出さずに描画する（オフライン時に空になるのを防ぐ）。
  const items = useMemo(
    () =>
      buildMessageItems(
        messages,
        undefined,
        readStateLoading || readStateError ? null : firstUnreadId,
      ),
    [messages, firstUnreadId, readStateLoading, readStateError],
  );
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_HEIGHT,
    overscan: 8,
    // Commit measured row sizes outside ResizeObserver delivery: anchoring can scroll
    // and synchronously render a different virtual range during that delivery.
    useAnimationFrameWithResizeObserver: true,
    getItemKey: (index) => items[index]?.message.id ?? index,
    // URLカードや画像の遅延読込で行が伸びても、最下部付近なら末尾を維持する。
    // 編集中は入力欄の高さが変わるため、編集中の位置を保つ。
    anchorTo: atBottomRef.current && editingId === null ? "end" : "start",
    scrollEndThreshold: 48,
  });

  useLayoutEffect(() => {
    if (editingId !== null) atBottomRef.current = false;
  }, [editingId]);
  const [hasNewBelow, setHasNewBelow] = useState(false);
  // 最下部にいないとき届いた新着を `aria-live` で読み上げる（「新着メッセージ」
  // ボタンと同じ条件）。最下部へ戻ったら空にして、同文の連投も再度読み上げられる。
  const [liveAnnouncement, setLiveAnnouncement] = useState("");
  const snapshotRef = useRef<ScrollSnapshot>({ firstId: null, lastId: null });
  const scrollAnchorRef = useRef<{ id: string; offset: number } | null>(null);
  const restoringAnchorRef = useRef(false);
  const captureScrollAnchor = useCallback(() => {
    const element = scrollRef.current;
    if (!element || restoringAnchorRef.current) return;
    const viewport = element.getBoundingClientRect();
    for (const row of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
      const bounds = row.getBoundingClientRect();
      if (bounds.bottom > viewport.top && bounds.top < viewport.bottom) {
        scrollAnchorRef.current = {
          id: row.dataset.messageId!,
          offset: bounds.top - viewport.top,
        };
        break;
      }
    }
  }, []);

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

  const totalSize = virtualizer.getTotalSize();

  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    captureScrollAnchor();
    const near = isNearBottom(element.scrollTop, element.scrollHeight, element.clientHeight);
    atBottomRef.current = near && editingId === null;
    if (near) {
      setHasNewBelow(false);
      markRead();
    }
    if (element.scrollTop <= LOAD_OLDER_THRESHOLD && messages.length > 0) {
      loadOlder();
    }
  }, [captureScrollAnchor, editingId, markRead, messages.length, loadOlder]);

  // 既読位置が、最初に読み込んだ範囲より古いとき、読み足しが終わるまで未読の先頭へ移動するのを待つ。
  const unreadPagesRef = useRef(0);
  const pendingUnreadJumpRef = useRef(false);
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const ids = items.map((item) => item.message.id);
    const next = snapshot(ids);
    const change = classifyScrollChange(snapshotRef.current, next, new Set(ids));

    // 件数の上限で、新着の分だけ最古の行が外れたとき (先頭が変わる append)。最下部にいなければ、
    // 読んでいる行の位置を保つ。
    const lastItem = items[items.length - 1];
    const slid =
      change === "append" &&
      snapshotRef.current.firstId !== next.firstId &&
      // 自分の送信は、上にスクロールしていても最下部へ追従する (下の append の処理)。
      lastItem?.message.user_id !== currentUserId;
    let stopRestoring: (() => void) | undefined;
    const oldest = messages[0] ? parseUtcTimestamp(messages[0].created_at).getTime() : NaN;
    const unreadCovered =
      lastRead === null ||
      lastRead <= 0 ||
      !Number.isFinite(oldest) ||
      oldest <= lastRead ||
      !hasOlder ||
      unreadPagesRef.current >= MAX_UNREAD_PAGES;
    const awaitingUnread = pendingUnreadJumpRef.current && change === "prepend";
    if (!awaitingUnread && (change === "prepend" || (slid && !atBottomRef.current))) {
      // Estimates for unmounted rows must not decide which message the reader sees.
      const anchor = scrollAnchorRef.current;
      const index = anchor ? items.findIndex((item) => item.message.id === anchor.id) : -1;
      if (anchor && index >= 0) {
        restoringAnchorRef.current = true;
        let frame = 0;
        let stopped = false;
        let stableFrames = 0;
        const events = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
        const stop = () => {
          stopped = true;
          restoringAnchorRef.current = false;
          cancelAnimationFrame(frame);
          for (const event of events) element.removeEventListener(event, stop);
        };
        stopRestoring = stop;
        for (const event of events) element.addEventListener(event, stop, { passive: true });
        const restore = () => {
          if (stopped || !element.isConnected) {
            stop();
            return;
          }
          const row = Array.from(element.querySelectorAll<HTMLElement>("[data-message-id]")).find(
            (entry) => entry.dataset.messageId === anchor.id,
          );
          if (row) {
            const delta =
              row.getBoundingClientRect().top - element.getBoundingClientRect().top - anchor.offset;
            if (Math.abs(delta) > 1) {
              stableFrames = 0;
              const previousTop = element.scrollTop;
              virtualizer.scrollToOffset(previousTop + delta);
              // A short list or viewport resize can make the requested offset unreachable.
              if (Math.abs(element.scrollTop - previousTop) < 0.5) {
                stop();
                captureScrollAnchor();
                return;
              }
            } else stableFrames++;
          } else {
            // Mount the anchor's new virtual range first, then correct its measured position.
            stableFrames = 0;
            const offset = virtualizer.getOffsetForIndex(index, "start");
            if (offset)
              virtualizer.scrollToOffset(
                offset[0] + parseFloat(getComputedStyle(element).paddingTop) - anchor.offset,
              );
          }
          // ResizeObserver and React can commit new measurements on a later frame.
          if (stableFrames < 2) frame = requestAnimationFrame(restore);
          else {
            stop();
            captureScrollAnchor();
          }
        };
        restore();
      }
      if (slid) {
        setHasNewBelow(true);
        if (lastItem) setLiveAnnouncement(newMessageAnnouncement(lastItem.message, t));
      }
    } else if (change === "initial" && firstUnreadId) {
      const index = items.findIndex((item) => item.message.id === firstUnreadId);
      atBottomRef.current = false;
      pendingUnreadJumpRef.current = !unreadCovered && !targetMessageId;
      requestAnimationFrame(() => virtualizer.scrollToIndex(index, { align: "start" }));
    } else if (awaitingUnread) {
      // 読み足しの途中: 位置は動かさない (終わったら、下で未読の先頭へ合わせる)。
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

    if (pendingUnreadJumpRef.current && change !== "initial" && unreadCovered && !isLoadingOlder) {
      pendingUnreadJumpRef.current = false;
      const index = firstUnreadId
        ? items.findIndex((item) => item.message.id === firstUnreadId)
        : -1;
      if (index >= 0) {
        atBottomRef.current = false;
        requestAnimationFrame(() => virtualizer.scrollToIndex(index, { align: "start" }));
      }
    }

    snapshotRef.current = next;
    return () => stopRestoring?.();
  }, [
    items,
    messages,
    lastRead,
    hasOlder,
    isLoadingOlder,
    currentUserId,
    scrollToBottom,
    t,
    firstUnreadId,
    virtualizer,
    captureScrollAnchor,
  ]);

  // Decide the initial unread position before following resized rows.
  useLayoutEffect(() => {
    // ResizeObserver can request a bottom adjustment before the sizer grows,
    // which the browser clamps. Reapply after committing the measured height.
    if (atBottomRef.current && editingId === null) scrollToBottom();
  }, [totalSize, editingId, scrollToBottom]);

  // Also capture after measurements commit, not just when the user scrolls.
  useLayoutEffect(() => {
    captureScrollAnchor();
  });

  // 一覧がスクロールできないほど短い場合でも、上端スクロール相当の操作が
  // できないため、古いメッセージがあれば自動で 1 ページずつ読み足す。
  useEffect(() => {
    const element = scrollRef.current;
    if (!element || messages.length === 0) {
      return;
    }
    if (element.scrollHeight <= element.clientHeight + LOAD_OLDER_THRESHOLD) {
      loadOlder();
    }
  }, [messages, loadOlder]);

  // 既読位置が、いま読み込んでいる範囲より古いときは、既読位置を含むまで読み足す (未読の区切りを置くため)。
  // 未読が多すぎるときのために、読み足す回数には上限がある。
  useEffect(() => {
    if (lastRead === null || lastRead <= 0 || messages.length === 0) return;
    const oldest = parseUtcTimestamp(messages[0]!.created_at).getTime();
    if (!Number.isFinite(oldest) || oldest <= lastRead) return;
    if (!hasOlder || isLoadingOlder || !onLoadOlder) return;
    if (unreadPagesRef.current >= MAX_UNREAD_PAGES) {
      // 上限までに、既読位置まで届かなかった。未読の先頭は、読み込んでいない範囲にある。
      toast.info(t("threads.unreadTooOld"), { id: "unread-too-old" });
      return;
    }
    unreadPagesRef.current += 1;
    onLoadOlder();
  }, [lastRead, messages, hasOlder, isLoadingOlder, onLoadOlder, t]);

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

  const scrolledTarget = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!targetMessageId) {
      scrolledTarget.current = undefined;
      return;
    }
    if (scrolledTarget.current === targetMessageId) return;
    const index = items.findIndex((item) => item.message.id === targetMessageId);
    if (index < 0) return;
    atBottomRef.current = false;
    const frame = requestAnimationFrame(() => {
      atBottomRef.current = false;
      virtualizer.scrollToIndex(index, { align: "center" });
      scrolledTarget.current = targetMessageId;
      // 通知・リンクの対象へ移動したら、未読の先頭への保留中の移動は取り消す。
      pendingUnreadJumpRef.current = false;
    });
    return () => cancelAnimationFrame(frame);
  }, [targetMessageId, items, virtualizer]);

  // 各行の measureElement / ResizeObserver が編集・保存・折り返しによる
  // 高さの変化を追跡する。measure() で全行の実測値を消すと、変化していない
  // 行まで推定高へ戻り、次の ResizeObserver 通知まで配置がずれてしまう。

  useEffect(() => {
    if (!hasNewBelow) {
      setLiveAnnouncement("");
    }
  }, [hasNewBelow]);

  const stalled = useLoadingTimeout(
    (isLoading || readStateLoading) && messages.length === 0 && !isOffline,
  );
  const showOffline = isOffline && items.length === 0;
  const showSkeleton =
    (isLoading || readStateLoading) && items.length === 0 && !isOffline && !stalled;
  const showError = !showOffline && (isError || readStateError || stalled) && items.length === 0;
  const showEmpty =
    !showSkeleton && !showOffline && !isError && !readStateError && !stalled && items.length === 0;

  return (
    <div className="relative min-h-0 flex-1">
      {showSkeleton ? (
        <MessageSkeleton />
      ) : showOffline ? (
        <MessageState
          title={t("offline.emptyTitle")}
          description={t("offline.emptyDescription")}
          action={
            onRetry ? (
              <Button variant="outline" size="sm" onClick={onRetry}>
                {t("offline.retry")}
              </Button>
            ) : null
          }
        />
      ) : showError ? (
        <MessageState
          destructive
          title={t("messages.loadError")}
          description={t("messages.retryHelp")}
          action={
            onRetry ? (
              <Button variant="outline" size="sm" onClick={onRetry}>
                {t("offline.retry")}
              </Button>
            ) : null
          }
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
          data-testid="message-scroll"
          className="h-full overflow-y-auto overscroll-contain px-2 py-3 [overflow-anchor:none]"
        >
          <div role="list" className="relative w-full" style={{ height: totalSize }}>
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
                  data-message-id={item.message.id}
                  role="listitem"
                  className={`absolute left-0 top-0 w-full ${item.message.id === targetMessageId ? "rounded-lg bg-brand-muted/40 ring-1 ring-brand" : ""}`}
                  style={{ transform: `translateY(${virtualRow.start}px)` }}
                >
                  {item.message.id === firstUnreadId ? (
                    <div
                      role="separator"
                      aria-label={t("messages.new")}
                      className="flex items-center gap-2 py-2 text-xs font-semibold text-destructive"
                    >
                      <span className="h-px flex-1 bg-destructive" />
                      <span>{t("messages.new")}</span>
                      <span className="h-px w-6 bg-destructive" />
                    </div>
                  ) : null}
                  <MessageRow
                    item={item}
                    reactions={
                      item.message.display_data
                        ? (groupReactions(item.message.display_data.reactions, currentUserId).get(
                            item.message.id,
                          ) ?? [])
                        : (reactions.get(item.message.id) ?? [])
                    }
                    kitchenCatalog={kitchenCatalog}
                    canReact={canReact}
                    onReaction={(emoji) => onReaction(item.message.id, emoji)}
                    onPickReaction={() => onPickReaction(item.message.id)}
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
                      onOpenThread &&
                      item.message.thread_activity_at &&
                      (item.message.display_data?.reply_count ?? 0) > 0
                        ? () => onOpenThread(item.message.id)
                        : undefined
                    }
                    onReply={onReply ? () => onReply(item.message.id) : undefined}
                    attachments={
                      item.message.display_data?.attachments ??
                      attachments?.get(item.message.id) ??
                      []
                    }
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
