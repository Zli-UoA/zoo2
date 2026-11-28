import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, LoaderCircleIcon, LockKeyholeIcon } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
  type RefObject,
} from "react";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { AttachmentRow } from "../../../lib/electric/types";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { groupReactions } from "../../../lib/emoji/reactions";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { newMessageAnnouncement, type FeedMessage } from "./messageFormat";
import { useMessageJumpRequest, useMessageTargetFlash } from "./messageTarget";
import { buildMessageItems } from "./messageGrouping";
import { MessageRow } from "./MessageRow";
import {
  isPlaintextBeforeEncryption,
  useEncryptedChannel,
} from "../../../lib/e2ee/messageDecryption";
import { parseUtcTimestamp } from "../../../lib/timestamps";
import { useMessageReadState } from "./useMessageReadState";
import {
  buildChannelScrollKey,
  classifyScrollChange,
  computeInitialScrollPlan,
  findVisibleMessageAnchor,
  getAnchorScrollDelta,
  getChannelMeasurementCache,
  getChannelMeasurementCacheGeneration,
  isNearBottom,
  recordLastViewportRect,
  resolveScrollBehavior,
  saveChannelMeasurementCache,
  shouldStickToBottom,
  snapshot,
  type ScrollSnapshot,
} from "./messageScroll";
import {
  decideUnreadLoad,
  isUnreadCovered,
  resolveUnreadBoundaryId,
} from "./messageUnreadBoundary";
import { useLoadingTimeout } from "./useLoadingTimeout";
import { useMediaQuery } from "../../layout/useMediaQuery";

const ESTIMATED_ROW_HEIGHT = 64;
const DEFAULT_VIEWPORT_HEIGHT = 600;
const SKELETON_ROWS = [0, 1, 2, 3, 4, 5];
/** 上端からこの距離まで来たら、過去のメッセージを読み足す。 */
const LOAD_OLDER_THRESHOLD = 64;
/** 初回位置合わせが非同期レイアウト計測で収束するまでの待機フレーム上限。 */
const MAX_SETTLE_FRAMES = 5;
/** バックグラウンドタブ等で rAF が止まった場合のフォールバック表示時間。 */
const SETTLE_FALLBACK_TIMEOUT_MS = 300;

interface MessageListProps {
  targetMessageId?: string;
  channelId?: string;
  messages: FeedMessage[];
  isLoading: boolean;
  isWritePending?: boolean;
  isError: boolean;
  currentUserId: string | null;
  /** 操作権限を伴わない閲覧者 ID。読み取り専用表示でメンションと自分のリアクションを判定する。 */
  viewerId?: string | null;
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
  onForward?: (message: FeedMessage, attachments: AttachmentRow[]) => void;
  /** 送信できる状態か。false なら空状態で「最初のメッセージ」を促さない。 */
  canCompose?: boolean;
  /** オフラインか。スケルトンの代わりに保存済みがない旨を出す。 */
  isOffline?: boolean;
  /** 未読区切りを明示する（読み取り専用画面では既読 API を使わずに渡す）。 */
  unreadBoundaryId?: string | null;
  /** 境界時刻。読み足した後に未読の先頭へ戻る判定に使う。 */
  unreadBoundaryAt?: number | null;
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

type MessageItem = ReturnType<typeof buildMessageItems>[number];

interface MessageScrollViewportProps {
  channelId?: string;
  targetMessageId?: string;
  messages: FeedMessage[];
  items: MessageItem[];
  firstUnreadId: string | null;
  lastRead: number | null;
  readStatePending: boolean;
  markRead: () => void;
  firstEncryptedIndex: number;
  currentUserId: string | null;
  viewerId?: string | null;
  canModerate: boolean;
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
  onForward?: (message: FeedMessage, attachments: AttachmentRow[]) => void;
  loadOlder: () => void;
  hasOlder: boolean;
  isLoadingOlder: boolean;
  unreadPagesRef: RefObject<number>;
  onLiveAnnouncementChange: (text: string) => void;
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
  viewerId,
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
  onForward,
  canCompose = true,
  isOffline = false,
  unreadBoundaryId,
  unreadBoundaryAt,
  onLoadOlder,
  hasOlder = true,
  isLoadingOlder = false,
  onRetry,
}: MessageListProps) {
  const { t } = useTranslation();
  const encryptedChannel = useEncryptedChannel();
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
  // 読み取り専用画面では既読 API の代わりに渡された境界を使う。
  // 既読位置の取得に失敗しても保存済みのメッセージは残し、区切りだけを出さない。
  const boundaryId = resolveUnreadBoundaryId(
    unreadBoundaryId,
    firstUnreadId,
    readStateLoading,
    readStateError,
  );
  const items = useMemo(
    () => buildMessageItems(messages, undefined, boundaryId),
    [messages, boundaryId],
  );
  const readBoundary = unreadBoundaryAt ?? lastRead;
  // 暗号化チャンネルで、読み込んだうちで最初の暗号化したメッセージ（#367）。ここに「ここから暗号化されています」の
  // 区切りを出す。これより後ろの暗号文のない行は、正しいサーバーが作れないので警告の印を付ける。
  const firstEncryptedIndex = useMemo(
    () =>
      encryptedChannel ? items.findIndex((item) => !isPlaintextBeforeEncryption(item.message)) : -1,
    [encryptedChannel, items],
  );
  const [liveAnnouncement, setLiveAnnouncement] = useState("");
  // 既読位置が、最初に読み込んだ範囲より古いとき、読み足しが終わるまで未読の先頭へ移動するのを待つ。
  const unreadPagesRef = useRef(0);

  // 既読位置が、いま読み込んでいる範囲より古いときは、既読位置を含むまで読み足す (未読の区切りを置くため)。
  // 未読が多すぎるときのために、読み足す回数には上限がある。
  useEffect(() => {
    if (readBoundary === null || readBoundary <= 0 || messages.length === 0) return;
    const decision = decideUnreadLoad({
      readBoundary,
      oldestCreatedAt: parseUtcTimestamp(messages[0]!.created_at).getTime(),
      hasOlder,
      isLoadingOlder,
      canLoadOlder: Boolean(onLoadOlder),
      loadedPages: unreadPagesRef.current,
    });
    if (decision === "load") {
      unreadPagesRef.current += 1;
      onLoadOlder?.();
    } else if (decision === "too-old") {
      // 上限までに、既読位置まで届かなかった。未読の先頭は、読み込んでいない範囲にある。
      toast.info(t("threads.unreadTooOld"), { id: "unread-too-old" });
    }
  }, [readBoundary, messages, hasOlder, isLoadingOlder, onLoadOlder, t]);

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
      {!showSkeleton && !showOffline && !showError && !showEmpty ? (
        <div
          role="status"
          data-testid="loading-older-status"
          className="pointer-events-none absolute inset-x-0 top-0 z-10 flex h-6 items-center justify-center text-xs text-muted-foreground"
        >
          {isLoadingOlder ? (
            <>
              <LoaderCircleIcon
                className="size-4 animate-spin motion-reduce:animate-none"
                aria-hidden="true"
              />
              <span className="sr-only">{t("messages.loadingOlder")}</span>
            </>
          ) : null}
        </div>
      ) : null}
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
        <MessageScrollViewport
          key={channelId ?? ""}
          channelId={channelId}
          targetMessageId={targetMessageId}
          messages={messages}
          items={items}
          firstUnreadId={boundaryId}
          lastRead={readBoundary}
          readStatePending={
            unreadBoundaryId === undefined &&
            Boolean(channelId && currentUserId) &&
            (isLoading || isError || readStateLoading || readStateError || lastRead === null)
          }
          markRead={markRead}
          firstEncryptedIndex={firstEncryptedIndex}
          currentUserId={currentUserId}
          viewerId={viewerId}
          canModerate={canModerate}
          editingId={editingId}
          editingText={editingText}
          onEditingTextChange={onEditingTextChange}
          onStartEdit={onStartEdit}
          onCancelEdit={onCancelEdit}
          onSaveEdit={onSaveEdit}
          onRequestDelete={onRequestDelete}
          reactions={reactions}
          kitchenCatalog={kitchenCatalog}
          canReact={canReact}
          onReaction={onReaction}
          onPickReaction={onPickReaction}
          attachments={attachments}
          onOpenThread={onOpenThread}
          onForward={onForward}
          loadOlder={loadOlder}
          hasOlder={hasOlder}
          isLoadingOlder={isLoadingOlder}
          unreadPagesRef={unreadPagesRef}
          onLiveAnnouncementChange={setLiveAnnouncement}
        />
      )}

      <p role="status" aria-live="polite" aria-atomic="true" className="sr-only">
        {liveAnnouncement}
      </p>
    </div>
  );
}

/**
 * 非空のメッセージ一覧を受け取ってマウントされる仮想スクロール本体。
 * 初回描画から末尾・未読境界・対象メッセージ付近のオフセットを推定して配置し、
 * 上端から末尾への視認できるスクロール遷移を起こさない。
 */
function MessageScrollViewport({
  channelId,
  targetMessageId,
  messages,
  items,
  firstUnreadId,
  lastRead,
  readStatePending,
  markRead,
  firstEncryptedIndex,
  currentUserId,
  viewerId,
  canModerate,
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
  onForward,
  loadOlder,
  hasOlder,
  isLoadingOlder,
  unreadPagesRef,
  onLiveAnnouncementChange,
}: MessageScrollViewportProps) {
  const { t } = useTranslation();
  const reactionUserId = currentUserId ?? viewerId ?? null;
  const [cacheKey] = useState(() =>
    channelId ? buildChannelScrollKey(channelId, currentUserId) : null,
  );
  const [initialPlan] = useState(() => {
    const cached = cacheKey ? getChannelMeasurementCache(cacheKey) : undefined;
    const targetIndex = targetMessageId
      ? items.findIndex((item) => item.message.id === targetMessageId)
      : -1;
    const unreadIndex =
      targetIndex < 0 && firstUnreadId
        ? items.findIndex((item) => item.message.id === firstUnreadId)
        : -1;
    return computeInitialScrollPlan({
      itemIds: items.map((item) => item.message.id),
      cached,
      defaultRowHeight: ESTIMATED_ROW_HEIGHT,
      defaultViewportHeight: DEFAULT_VIEWPORT_HEIGHT,
      targetIndex,
      unreadIndex,
    });
  });

  const scrollRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const hasInitialTargetInItems = Boolean(
    targetMessageId && items.some((item) => item.message.id === targetMessageId),
  );
  const hasInitialUnreadInItems = Boolean(
    !hasInitialTargetInItems &&
    firstUnreadId &&
    items.some((item) => item.message.id === firstUnreadId),
  );
  const atBottomRef = useRef(!hasInitialTargetInItems && !hasInitialUnreadInItems);
  const prefersReducedMotion = useMediaQuery("(prefers-reduced-motion: reduce)");
  const settlingRef = useRef(true);
  const settledOnceRef = useRef(false);
  const latestMetricsRef = useRef({ width: 0, height: 0 });
  const activeEstimateRef = useRef<{
    sizes: ReadonlyMap<string, number> | undefined;
    rowHeight: number;
  }>({
    sizes: initialPlan.cachedSizes,
    rowHeight: initialPlan.estimatedRowHeight,
  });
  const measuredWidthRef = useRef<number | null>(initialPlan.cachedWidth ?? null);
  const widthInvalidatedRef = useRef(false);
  const measuredKeysForWidthRef = useRef<Set<string>>(
    new Set(initialPlan.cachedSizes ? initialPlan.cachedSizes.keys() : []),
  );
  const cacheGenerationRef = useRef(getChannelMeasurementCacheGeneration());

  const invalidateWidthGeneration = useCallback((nextWidth: number, resetEstimates: boolean) => {
    if (resetEstimates) {
      activeEstimateRef.current = { sizes: undefined, rowHeight: ESTIMATED_ROW_HEIGHT };
    }
    widthInvalidatedRef.current = true;
    measuredWidthRef.current = nextWidth;
    measuredKeysForWidthRef.current.clear();
  }, []);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const id = items[index]?.message.id;
      return (
        (id ? activeEstimateRef.current.sizes?.get(id) : undefined) ??
        activeEstimateRef.current.rowHeight
      );
    },
    measureElement: (el, entry, instance) => {
      const index = instance.indexFromElement(el);
      const key = instance.options.getItemKey(index);
      const containerWidth = scrollRef.current?.clientWidth ?? 0;
      if (containerWidth > 0) {
        if (measuredWidthRef.current === null) {
          measuredWidthRef.current = containerWidth;
        } else if (Math.abs(containerWidth - measuredWidthRef.current) > 2) {
          invalidateWidthGeneration(containerWidth, false);
        }
      }
      const box = entry?.borderBoxSize?.[0];
      if (box) {
        if (typeof key === "string") {
          measuredKeysForWidthRef.current.add(key);
        }
        return Math.round(box.blockSize);
      }
      if (!entry) {
        const cachedSize = instance.itemSizeCache.get(key);
        if (
          cachedSize !== undefined &&
          (typeof key !== "string" || measuredKeysForWidthRef.current.has(key))
        ) {
          return cachedSize;
        }
      }
      const measured = (el as HTMLElement).offsetHeight;
      if (typeof key === "string" && measured > 0) {
        measuredKeysForWidthRef.current.add(key);
      }
      return measured;
    },
    overscan: 8,
    initialOffset: initialPlan.initialOffset,
    initialRect: initialPlan.initialRect,
    initialMeasurementsCache: widthInvalidatedRef.current
      ? undefined
      : initialPlan.initialMeasurementsCache,
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
  const snapshotRef = useRef<ScrollSnapshot>({ firstId: null, lastId: null });
  const scrollAnchorRef = useRef<{ id: string; offset: number } | null>(null);
  const restoringAnchorRef = useRef(false);
  const captureScrollAnchor = useCallback(() => {
    const element = scrollRef.current;
    if (!element || restoringAnchorRef.current || settlingRef.current) return;
    const anchor = findVisibleMessageAnchor(element);
    if (anchor) scrollAnchorRef.current = anchor;
  }, []);

  const alignIndexSync = useCallback(
    (index: number, align: "start" | "center") => {
      const element = scrollRef.current;
      if (!element || index < 0) return null;
      const offset =
        typeof virtualizer.getOffsetForIndex === "function"
          ? virtualizer.getOffsetForIndex(index, align)
          : undefined;
      if (!offset) {
        virtualizer.scrollToIndex(index, { align });
        return null;
      }
      const clamped = Math.max(
        0,
        Math.min(offset[0], Math.max(0, element.scrollHeight - element.clientHeight)),
      );
      if (Math.abs(element.scrollTop - clamped) > 1.5) {
        element.scrollTop = clamped;
      }
      return clamped;
    },
    [virtualizer],
  );

  const scrollToBottom = useCallback(
    (behavior: ScrollBehavior = "auto") => {
      atBottomRef.current = true;
      setHasNewBelow(false);
      if (items.length === 0) {
        return;
      }
      const resolved = resolveScrollBehavior(behavior, prefersReducedMotion);
      const element = scrollRef.current;
      if (resolved === "auto" && element) {
        const maxTop = Math.max(0, element.scrollHeight - element.clientHeight);
        if (Math.abs(element.scrollTop - maxTop) > 0.5) {
          element.scrollTop = maxTop;
        }
        return;
      }
      // 行の実測高さに追従して最下部へ合わせる（smooth スクロール時）。
      virtualizer.scrollToIndex(items.length - 1, {
        align: "end",
        behavior: resolved,
      });
    },
    [items.length, virtualizer, prefersReducedMotion],
  );

  const totalSize = virtualizer.getTotalSize();

  const pendingUnreadJumpRef = useRef(false);
  const unreadJumpFrameRef = useRef<number | null>(null);
  const scrolledTarget = useRef<string | undefined>(undefined);
  const initialUnreadResolvedRef = useRef(!readStatePending);
  const userInteractedRef = useRef(false);
  const realignWhileSettlingRef = useRef<(() => void) | null>(null);

  const cancelUnreadJump = useCallback(() => {
    if (unreadJumpFrameRef.current !== null) {
      cancelAnimationFrame(unreadJumpFrameRef.current);
      unreadJumpFrameRef.current = null;
    }
  }, []);

  useEffect(() => cancelUnreadJump, [cancelUnreadJump]);

  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element || settlingRef.current) {
      return;
    }
    captureScrollAnchor();
    const near = isNearBottom(element.scrollTop, element.scrollHeight, element.clientHeight);
    atBottomRef.current = near && editingId === null;
    if (near) {
      setHasNewBelow(false);
      if (
        !readStatePending &&
        !pendingUnreadJumpRef.current &&
        unreadJumpFrameRef.current === null
      ) {
        markRead();
      }
    }
    if (element.scrollTop <= LOAD_OLDER_THRESHOLD && messages.length > 0) {
      loadOlder();
    }
  }, [captureScrollAnchor, editingId, markRead, readStatePending, messages.length, loadOlder]);

  const oldestCreatedAt = messages[0] ? parseUtcTimestamp(messages[0].created_at).getTime() : NaN;
  const unreadCovered = isUnreadCovered({
    lastRead,
    oldestCreatedAt,
    hasOlder,
    loadedPages: unreadPagesRef.current,
  });

  const latestMountDepsRef = useRef({
    items,
    editingId,
    targetMessageId,
    firstUnreadId,
    unreadCovered,
    readStatePending,
    markRead,
    scrollToBottom,
    alignIndexSync,
    captureScrollAnchor,
    initialPlan,
  });
  useLayoutEffect(() => {
    latestMountDepsRef.current = {
      items,
      editingId,
      targetMessageId,
      firstUnreadId,
      unreadCovered,
      readStatePending,
      markRead,
      scrollToBottom,
      alignIndexSync,
      captureScrollAnchor,
      initialPlan,
    };
  });

  const scheduleUnreadJump = useCallback(
    (index: number) => {
      cancelUnreadJump();
      pendingUnreadJumpRef.current = false;
      atBottomRef.current = false;
      alignIndexSync(index, "start");
      unreadJumpFrameRef.current = requestAnimationFrame(() => {
        unreadJumpFrameRef.current = null;
        const deps = latestMountDepsRef.current;
        const activeTargetIndex = deps.targetMessageId
          ? deps.items.findIndex((item) => item.message.id === deps.targetMessageId)
          : -1;
        if (
          activeTargetIndex >= 0 ||
          (deps.targetMessageId && scrolledTarget.current === deps.targetMessageId)
        ) {
          return;
        }
        if (!deps.firstUnreadId) return;
        const idx = deps.items.findIndex((item) => item.message.id === deps.firstUnreadId);
        if (idx < 0) return;
        atBottomRef.current = false;
        virtualizer.scrollToIndex(idx, { align: "start" });
        alignIndexSync(idx, "start");
        const el = scrollRef.current;
        if (el && el.clientHeight > 0 && !settlingRef.current && !pendingUnreadJumpRef.current) {
          const near = isNearBottom(el.scrollTop, el.scrollHeight, el.clientHeight);
          atBottomRef.current = near && deps.editingId === null;
          if (near && !deps.readStatePending) {
            setHasNewBelow(false);
            deps.markRead();
          }
        }
      });
    },
    [alignIndexSync, cancelUnreadJump, virtualizer],
  );

  useEffect(() => {
    if (!readStatePending) return;
    const element = scrollRef.current;
    if (!element) return;
    const markInteracted = () => {
      userInteractedRef.current = true;
    };
    const events = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
    for (const ev of events) element.addEventListener(ev, markInteracted, { passive: true });
    return () => {
      for (const ev of events) element.removeEventListener(ev, markInteracted);
    };
  }, [readStatePending]);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (width <= 0 || height <= 0) return;
      latestMetricsRef.current = { width, height };
      recordLastViewportRect(width, height);
      if (measuredWidthRef.current === null) {
        measuredWidthRef.current = width;
      } else if (Math.abs(width - measuredWidthRef.current) > 2) {
        invalidateWidthGeneration(width, false);
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [invalidateWidthGeneration]);

  // 初回マウント時、描画前に目的位置（末尾 / 未読境界 / 対象メッセージ）へ同期配置し、
  // 未計測行の ResizeObserver 確定が終わるまで（キャッシュ命中時は即時完了）表示を整える。
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) return;

    latestMetricsRef.current = {
      width: element.clientWidth,
      height: element.clientHeight,
    };
    recordLastViewportRect(element.clientWidth, element.clientHeight);

    if (
      latestMountDepsRef.current.initialPlan.cachedWidth !== undefined &&
      element.clientWidth > 0 &&
      Math.abs(element.clientWidth - latestMountDepsRef.current.initialPlan.cachedWidth) > 2
    ) {
      invalidateWidthGeneration(element.clientWidth, true);
      virtualizer.measure();
      if (typeof virtualizer.measureElement === "function") {
        for (const row of element.querySelectorAll<HTMLElement>("[data-index]")) {
          virtualizer.measureElement(row);
        }
      }
    } else if (measuredWidthRef.current === null && element.clientWidth > 0) {
      measuredWidthRef.current = element.clientWidth;
    }

    const finishSettling = () => {
      if (settledOnceRef.current) return;
      settledOnceRef.current = true;
      settlingRef.current = false;
      realignWhileSettlingRef.current = null;
      const deps = latestMountDepsRef.current;
      deps.captureScrollAnchor();
      const el = scrollRef.current;
      if (el && el.clientWidth > 0 && el.clientHeight > 0) {
        latestMetricsRef.current = { width: el.clientWidth, height: el.clientHeight };
      }
      if (
        el &&
        el.clientHeight > 0 &&
        !pendingUnreadJumpRef.current &&
        unreadJumpFrameRef.current === null
      ) {
        const near = isNearBottom(el.scrollTop, el.scrollHeight, el.clientHeight);
        atBottomRef.current = near && deps.editingId === null;
        if (near && !deps.readStatePending) {
          setHasNewBelow(false);
          deps.markRead();
        }
      }
    };

    const alignTargetNow = () => {
      const deps = latestMountDepsRef.current;
      if (deps.targetMessageId) {
        const idx = deps.items.findIndex((item) => item.message.id === deps.targetMessageId);
        if (idx >= 0) {
          atBottomRef.current = false;
          pendingUnreadJumpRef.current = false;
          cancelUnreadJump();
          return {
            mode: "target" as const,
            targetId: deps.targetMessageId,
            expectedTop: deps.alignIndexSync(idx, "center"),
          };
        }
      }
      if (deps.firstUnreadId) {
        const idx = deps.items.findIndex((item) => item.message.id === deps.firstUnreadId);
        if (idx >= 0) {
          atBottomRef.current = false;
          pendingUnreadJumpRef.current = !deps.unreadCovered;
          return {
            mode: "unread" as const,
            targetId: deps.firstUnreadId,
            expectedTop: deps.alignIndexSync(idx, "start"),
          };
        }
      }
      atBottomRef.current = true;
      deps.scrollToBottom("auto");
      return { mode: "bottom" as const, targetId: null as string | null, expectedTop: null };
    };

    const isConverged = (state: ReturnType<typeof alignTargetNow>): boolean => {
      if (element.clientHeight === 0) {
        return true;
      }
      const expectedTotal = virtualizer.getTotalSize();
      const listEl = listRef.current;
      if (listEl && Math.abs(listEl.offsetHeight - expectedTotal) > 2) {
        return false;
      }
      if (element.scrollHeight <= element.clientHeight + 1) {
        return true;
      }
      if (state.mode === "bottom") {
        const maxTop = Math.max(0, element.scrollHeight - element.clientHeight);
        if (Math.abs(element.scrollTop - maxTop) > 2) {
          return false;
        }
      } else if (
        state.expectedTop !== null &&
        Math.abs(element.scrollTop - state.expectedTop) > 2
      ) {
        return false;
      }
      const virtualByIndex = new Map<number, number>();
      for (const row of virtualizer.getVirtualItems()) {
        virtualByIndex.set(row.index, row.size);
      }
      const rendered = element.querySelectorAll<HTMLElement>("[data-index]");
      if (rendered.length === 0) return false;
      for (const row of rendered) {
        const idx = Number(row.dataset.index);
        const size = virtualByIndex.get(idx);
        const rectHeight = row.getBoundingClientRect().height;
        if (size === undefined || Math.abs(rectHeight - size) > 1.5) {
          return false;
        }
      }
      if (state.targetId) {
        const targetRow = Array.from(
          element.querySelectorAll<HTMLElement>("[data-message-id]"),
        ).find((entry) => entry.dataset.messageId === state.targetId);
        if (!targetRow) return false;
        const sr = element.getBoundingClientRect();
        const tr = targetRow.getBoundingClientRect();
        if (tr.bottom <= sr.top + 1 || tr.top >= sr.bottom - 1) {
          return false;
        }
      }
      return true;
    };

    const initialState = alignTargetNow();
    if (isConverged(initialState)) {
      finishSettling();
      return;
    }

    let frame = 0;
    let frames = 0;
    let stopped = false;
    const cancelEvents = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
    const stop = () => {
      if (stopped) return;
      stopped = true;
      cancelAnimationFrame(frame);
      clearTimeout(fallbackTimer);
      for (const ev of cancelEvents) element.removeEventListener(ev, stop);
      finishSettling();
    };
    realignWhileSettlingRef.current = () => {
      if (!settlingRef.current || stopped) return;
      const state = alignTargetNow();
      if (isConverged(state)) {
        stop();
      }
    };
    for (const ev of cancelEvents) element.addEventListener(ev, stop, { passive: true });
    const fallbackTimer = setTimeout(stop, SETTLE_FALLBACK_TIMEOUT_MS);

    const step = () => {
      if (stopped) return;
      frames += 1;
      const beforeTop = element.scrollTop;
      const state = alignTargetNow();
      const scrolledThisFrame = Math.abs(element.scrollTop - beforeTop) > 2;
      if ((!scrolledThisFrame && isConverged(state)) || frames >= MAX_SETTLE_FRAMES) {
        stop();
        return;
      }
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);

    return () => {
      stopped = true;
      realignWhileSettlingRef.current = null;
      cancelAnimationFrame(frame);
      clearTimeout(fallbackTimer);
      for (const ev of cancelEvents) element.removeEventListener(ev, stop);
    };
  }, [virtualizer, invalidateWidthGeneration, cancelUnreadJump]);

  useLayoutEffect(() => {
    if (readStatePending || initialUnreadResolvedRef.current) return;
    initialUnreadResolvedRef.current = true;
    if (userInteractedRef.current) return;
    const hasTarget = Boolean(
      targetMessageId && items.some((item) => item.message.id === targetMessageId),
    );
    if (hasTarget || (targetMessageId && scrolledTarget.current === targetMessageId)) {
      return;
    }
    if (!unreadCovered) {
      atBottomRef.current = false;
      pendingUnreadJumpRef.current = true;
      if (firstUnreadId) {
        const idx = items.findIndex((item) => item.message.id === firstUnreadId);
        if (idx >= 0) {
          alignIndexSync(idx, "start");
        }
      }
      return;
    }
    if (!firstUnreadId) return;
    const index = items.findIndex((item) => item.message.id === firstUnreadId);
    if (index >= 0) {
      scheduleUnreadJump(index);
    }
  }, [
    readStatePending,
    firstUnreadId,
    unreadCovered,
    targetMessageId,
    items,
    alignIndexSync,
    scheduleUnreadJump,
  ]);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    if (element.clientWidth > 0 && element.clientHeight > 0) {
      latestMetricsRef.current = { width: element.clientWidth, height: element.clientHeight };
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
    const awaitingUnread = pendingUnreadJumpRef.current && change === "prepend";
    const hasTargetInItems = Boolean(
      targetMessageId && items.some((item) => item.message.id === targetMessageId),
    );
    const preserveReadingOnReplace =
      change === "replace" &&
      !atBottomRef.current &&
      initialUnreadResolvedRef.current &&
      !hasTargetInItems &&
      !pendingUnreadJumpRef.current &&
      unreadJumpFrameRef.current === null;
    if (change === "initial") {
      // 初回表示の位置合わせはマウント直後の useLayoutEffect が担当する。
    } else if (
      !awaitingUnread &&
      (change === "prepend" || (slid && !atBottomRef.current) || preserveReadingOnReplace)
    ) {
      // Estimates for unmounted rows must not decide which message the reader sees.
      const anchor = scrollAnchorRef.current;
      const index = anchor ? items.findIndex((item) => item.message.id === anchor.id) : -1;
      // アンカーが押し出された (index < 0) 場合は、消えた行の実測高さが取れないので
      // scrollTop を推測で動かさない。描画後の captureScrollAnchor() が次の位置を記録する。
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
            const delta = getAnchorScrollDelta(
              row.getBoundingClientRect().top,
              element.getBoundingClientRect().top,
              anchor.offset,
            );
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
            const offset =
              typeof virtualizer.getOffsetForIndex === "function"
                ? virtualizer.getOffsetForIndex(index, "start")
                : undefined;
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
        if (lastItem) onLiveAnnouncementChange(newMessageAnnouncement(lastItem.message, t));
      }
    } else if (change === "replace" && hasTargetInItems && targetMessageId) {
      cancelUnreadJump();
      atBottomRef.current = false;
      scrolledTarget.current = undefined;
      pendingUnreadJumpRef.current = false;
    } else if (
      change === "replace" &&
      (pendingUnreadJumpRef.current || unreadJumpFrameRef.current !== null)
    ) {
      // 保留中・予約済みの未読ジャンプが完了するまで末尾追従しない。
    } else if (change === "replace" && !initialUnreadResolvedRef.current && firstUnreadId) {
      const index = items.findIndex((item) => item.message.id === firstUnreadId);
      if (index >= 0) {
        atBottomRef.current = false;
        pendingUnreadJumpRef.current = !unreadCovered;
        alignIndexSync(index, "start");
      }
    } else if (awaitingUnread) {
      // 読み足しの途中: 位置は動かさない (終わったら、下で未読の先頭へ合わせる)。
    } else if (shouldStickToBottom(change, atBottomRef.current)) {
      if (change === "replace") {
        if (atBottomRef.current) scrollToBottom("auto");
      } else {
        requestAnimationFrame(() => scrollToBottom());
      }
    } else if (change === "append") {
      // 自分の送信は、上にスクロールしていても最下部へ追従する。
      const appended = items[items.length - 1];
      if (appended && appended.message.user_id === currentUserId) {
        requestAnimationFrame(() => scrollToBottom());
      } else {
        setHasNewBelow(true);
        if (appended) {
          onLiveAnnouncementChange(newMessageAnnouncement(appended.message, t));
        }
      }
    }

    if (pendingUnreadJumpRef.current && change !== "initial" && unreadCovered && !isLoadingOlder) {
      pendingUnreadJumpRef.current = false;
      if (!hasTargetInItems && (!targetMessageId || scrolledTarget.current !== targetMessageId)) {
        const index = firstUnreadId
          ? items.findIndex((item) => item.message.id === firstUnreadId)
          : -1;
        if (index >= 0) {
          scheduleUnreadJump(index);
        }
      }
    }

    snapshotRef.current = next;
    return () => stopRestoring?.();
  }, [
    items,
    messages,
    unreadCovered,
    hasOlder,
    isLoadingOlder,
    currentUserId,
    scrollToBottom,
    alignIndexSync,
    cancelUnreadJump,
    scheduleUnreadJump,
    t,
    firstUnreadId,
    targetMessageId,
    virtualizer,
    captureScrollAnchor,
    onLiveAnnouncementChange,
  ]);

  // Decide the initial unread position before following resized rows.
  useLayoutEffect(() => {
    if (settlingRef.current) {
      realignWhileSettlingRef.current?.();
      return;
    }
    // ResizeObserver can request a bottom adjustment before the sizer grows,
    // which the browser clamps. Reapply after committing the measured height.
    if (atBottomRef.current && editingId === null) scrollToBottom();
  }, [totalSize, editingId, scrollToBottom]);

  // Also capture after measurements commit, not just when the user scrolls.
  useLayoutEffect(() => {
    captureScrollAnchor();
  });

  // チャンネルを離れるとき（アンマウント時）に計測済みの行高を保存し、次回切替で再利用する。
  useLayoutEffect(() => {
    return () => {
      if (!cacheKey || cacheGenerationRef.current !== getChannelMeasurementCacheGeneration()) {
        return;
      }
      const el = scrollRef.current;
      const width = el && el.clientWidth > 0 ? el.clientWidth : latestMetricsRef.current.width;
      const height = el && el.clientHeight > 0 ? el.clientHeight : latestMetricsRef.current.height;
      if (width <= 0 || height <= 0) return;
      if (measuredWidthRef.current !== null && Math.abs(width - measuredWidthRef.current) > 2) {
        return;
      }
      const snapshotItems =
        typeof virtualizer.takeSnapshot === "function"
          ? virtualizer
              .takeSnapshot()
              .filter(
                (item) =>
                  typeof item.key === "string" && measuredKeysForWidthRef.current.has(item.key),
              )
          : [];
      saveChannelMeasurementCache({
        key: cacheKey,
        containerWidth: width,
        viewportHeight: height,
        snapshotItems,
        previousSizes: widthInvalidatedRef.current ? undefined : initialPlan.cachedSizes,
        fallbackRowHeight: widthInvalidatedRef.current
          ? ESTIMATED_ROW_HEIGHT
          : initialPlan.estimatedRowHeight,
      });
    };
  }, [cacheKey, virtualizer, initialPlan]);

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

  useEffect(() => {
    const update = () => {
      const element = scrollRef.current;
      if (
        element &&
        !settlingRef.current &&
        !readStatePending &&
        !pendingUnreadJumpRef.current &&
        unreadJumpFrameRef.current === null &&
        isNearBottom(element.scrollTop, element.scrollHeight, element.clientHeight)
      ) {
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
  }, [markRead, readStatePending]);

  // 表示中の対象メッセージへのリンクをもう一度押したとき。URL が変わらず、ルーターは何も変えないので、
  // スクロール済みの印を消して、下の効果をもう一度走らせる。
  const jumpRequest = useMessageJumpRequest(targetMessageId, scrolledTarget);
  // 対象の行へスクロールしたときに、その行の強調（点滅 → 点灯 → フェードアウト）を再生する。
  const { play: playTargetFlash, runFor: targetFlashRun } = useMessageTargetFlash();

  useLayoutEffect(() => {
    if (!targetMessageId) {
      scrolledTarget.current = undefined;
      return;
    }
    if (scrolledTarget.current === targetMessageId) return;
    const index = items.findIndex((item) => item.message.id === targetMessageId);
    if (index < 0) return;
    cancelUnreadJump();
    atBottomRef.current = false;
    pendingUnreadJumpRef.current = false;
    alignIndexSync(index, "center");
    const frame = requestAnimationFrame(() => {
      atBottomRef.current = false;
      scrolledTarget.current = targetMessageId;
      virtualizer.scrollToIndex(index, { align: "center" });
      playTargetFlash(targetMessageId);
    });
    return () => cancelAnimationFrame(frame);
  }, [
    targetMessageId,
    items,
    virtualizer,
    alignIndexSync,
    cancelUnreadJump,
    jumpRequest,
    playTargetFlash,
  ]);

  // 各行の measureElement / ResizeObserver が編集・保存・折り返しによる
  // 高さの変化を追跡する。measure() で全行の実測値を消すと、変化していない
  // 行まで推定高へ戻り、次の ResizeObserver 通知まで配置がずれてしまう。

  useEffect(() => {
    if (!hasNewBelow) {
      onLiveAnnouncementChange("");
    }
  }, [hasNewBelow, onLiveAnnouncementChange]);

  return (
    <>
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        data-testid="message-scroll"
        data-scroll-restoration-id={channelId ? `message-scroll:${channelId}` : "message-scroll"}
        className="h-full overflow-y-auto overscroll-contain px-2 py-3 [overflow-anchor:none]"
      >
        <div
          role="list"
          ref={listRef}
          className="relative h-(--message-list-height) w-full"
          style={{ "--message-list-height": `${totalSize}px` } as CSSProperties}
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
                data-message-id={item.message.id}
                role="listitem"
                className="absolute left-0 top-0 w-full translate-y-(--message-row-offset)"
                style={{ "--message-row-offset": `${virtualRow.start}px` } as CSSProperties}
              >
                {virtualRow.index > 0 && virtualRow.index === firstEncryptedIndex ? (
                  // 暗号化に切り替える前と後の境目（#367）。上は平文、下は暗号化したメッセージ。
                  <div
                    role="separator"
                    aria-label={t("e2ee.legacy.divider")}
                    className="flex items-center gap-2 py-2 text-xs font-medium text-muted-foreground"
                  >
                    <span className="h-px flex-1 bg-border" />
                    <LockKeyholeIcon className="size-3" aria-hidden="true" />
                    <span>{t("e2ee.legacy.divider")}</span>
                    <span className="h-px w-6 bg-border" />
                  </div>
                ) : null}
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
                  highlightRun={targetFlashRun(item.message.id)}
                  mentionUserId={viewerId === undefined ? currentUserId : viewerId}
                  reactions={
                    item.message.display_data
                      ? (groupReactions(item.message.display_data.reactions, reactionUserId).get(
                          item.message.id,
                        ) ?? [])
                      : (reactions.get(item.message.id) ?? [])
                  }
                  kitchenCatalog={kitchenCatalog}
                  canReact={canReact && !item.message.thread_archive_at}
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
                  onOpenThread={onOpenThread ? () => onOpenThread(item.message.id) : undefined}
                  attachments={
                    item.message.display_data?.attachments ??
                    attachments?.get(item.message.id) ??
                    []
                  }
                  onForward={
                    onForward
                      ? () =>
                          onForward(
                            item.message,
                            item.message.display_data?.attachments ??
                              attachments?.get(item.message.id) ??
                              [],
                          )
                      : undefined
                  }
                  unencryptedAfterEncrypted={
                    firstEncryptedIndex >= 0 && virtualRow.index > firstEncryptedIndex
                  }
                />
              </div>
            );
          })}
        </div>
      </div>

      {hasNewBelow ? (
        <div className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full shadow-md">
          <Button shape="circle" size="sm" onClick={() => scrollToBottom("smooth")}>
            <ArrowDownIcon />
            {t("messages.new")}
          </Button>
        </div>
      ) : null}
    </>
  );
}
