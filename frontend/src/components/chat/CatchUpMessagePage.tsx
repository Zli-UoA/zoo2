import { useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import { useTranslation } from "react-i18next";

import { parseUtcTimestamp } from "../../lib/timestamps";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useMessageDecryptions } from "../../hooks/useMessageDecryptions";
import { useEncryptedDisplay } from "../../hooks/useEncryptedDisplay";
import {
  MessageDecryptionsProvider,
  firstEncryptedAt,
  withUnencryptedRows,
} from "../../lib/e2ee/messageDecryption";
import { useMessageAttachments } from "../../hooks/useAttachments";
import { ForwardedMessagesProvider, useForwardedMessages } from "../../hooks/useForwardedMessages";
import { useMessageFeed, useUsers } from "../../hooks/useRealtime";
import { groupAttachmentsByMessage } from "../../lib/attachments";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { MessageList } from "./messages/MessageList";
import { ReactionPicker } from "./messages/ReactionPicker";
import { useChannelReactions } from "./messages/useChannelReactions";
import type { FeedMessage } from "./messages/messageFormat";

/** How many pages back we look for the unread divider before leaving it to scrolling. */
const MAX_UNREAD_PAGES = 5;
/** 未読の範囲を少ない往復で覆うため、通常の投稿一覧より大きいページを使う。 */
const CATCHUP_PAGE_SIZE = 150;

/**
 * One conversation's unread history, rendered with the ordinary message list
 * but without composer, thread pane or automatic read receipts.
 */
export function CatchUpMessagePage({
  channelId,
  channelName,
  encrypted = false,
  readOnly = false,
  firstUnreadAt,
  onBoundaryMessage,
  onRangeReady,
  loadOlderRef,
}: {
  channelId: string;
  channelName?: string;
  encrypted?: boolean;
  readOnly?: boolean;
  firstUnreadAt: string | null;
  onBoundaryMessage: (messageId: string | null, createdAt: number | null) => void;
  onRangeReady?: (ready: boolean, exhausted: boolean, boundaryTooOld: boolean) => void;
  loadOlderRef?: MutableRefObject<(() => void) | null>;
}) {
  const { t } = useTranslation();
  const feed = useMessageFeed(channelId, { pageSize: CATCHUP_PAGE_SIZE });
  const { currentUserId } = useCurrentUser();
  const messages = feed.data as FeedMessage[];
  const usersQuery = useUsers();
  const { isOnline } = useOfflineStatus();
  const {
    reactionGroups,
    kitchenCatalog,
    canReact,
    isError: reactionsError,
    reactionMessageId,
    openReactionPicker,
    closeReactionPicker,
    handleReaction,
  } = useChannelReactions({
    channelId,
    currentUserId,
    users: usersQuery.data,
    usersLoading: usersQuery.isLoading,
    readOnly: readOnly || !isOnline || feed.isLoading || feed.isError,
  });
  const attachmentsQuery = useMessageAttachments(
    channelId,
    messages.some((row) => !row.display_data),
  );
  const attachments = useMemo(
    () => groupAttachmentsByMessage(attachmentsQuery.data ?? []),
    [attachmentsQuery.data],
  );
  // 暗号化の判定は MessagePane と同じ共通hookを使う（ciphertext の有無だけでは
  // epoch だけの行、edm: の暗号化DM、格下げを取りこぼす）。
  const { treatAsEncrypted } = useEncryptedDisplay({
    channelId,
    channelName,
    encrypted,
    hasCiphertext: messages.some((row) => row.ciphertext != null),
  });
  const decryptionRows = useMemo(
    () => (treatAsEncrypted ? messages : []),
    [treatAsEncrypted, messages],
  );
  const workerDecryptions = useMessageDecryptions(channelId, decryptionRows);
  const decryptions = useMemo(
    () => withUnencryptedRows(decryptionRows, workerDecryptions),
    [decryptionRows, workerDecryptions],
  );
  const encryptedSince = useMemo(() => firstEncryptedAt(messages), [messages]);
  const mentionUserNames = useMemo(
    () => new Map((usersQuery.data ?? []).map((user) => [user.id, user.user_name])),
    [usersQuery.data],
  );
  const forwardedIds = useMemo(
    () => (treatAsEncrypted ? [] : messages.flatMap((row) => row.forwarded_message_ids ?? [])),
    [messages, treatAsEncrypted],
  );
  const forwardedMessages = useForwardedMessages(forwardedIds, !treatAsEncrypted);

  // The boundary is the newest message visible when the page opens. Messages
  // arriving while reading stay unread for the next catch-up.
  const firstUnreadId = useMemo(() => {
    if (!firstUnreadAt) return null;
    const threshold = parseUtcTimestamp(firstUnreadAt).getTime();
    return (
      messages.find(
        (message) =>
          message.user_id !== currentUserId &&
          parseUtcTimestamp(message.created_at).getTime() >= threshold,
      )?.id ?? null
    );
  }, [messages, firstUnreadAt, currentUserId]);

  const boundaryRef = useRef<{ id: string; at: number } | null>(null);
  const [boundaryState, setBoundaryState] = useState<{ id: string; at: number } | null>(null);
  useEffect(() => {
    if (feed.isInitialSyncLoading || messages.length === 0) return;
    if (!boundaryRef.current) {
      const newest = messages[messages.length - 1];
      const at = parseUtcTimestamp(newest.created_at).getTime();
      boundaryRef.current = { id: newest.id, at };
      setBoundaryState(boundaryRef.current);
      onBoundaryMessage(newest.id, at);
      return;
    }
    // The boundary message can be deleted while reading; fall back to the
    // newest surviving message at or before the original boundary timestamp.
    if (messages.some((row) => row.id === boundaryRef.current!.id)) return;
    const fallback = [...messages]
      .reverse()
      .find((row) => parseUtcTimestamp(row.created_at).getTime() <= boundaryRef.current!.at);
    if (fallback) {
      boundaryRef.current = { id: fallback.id, at: boundaryRef.current.at };
      setBoundaryState(boundaryRef.current);
      onBoundaryMessage(fallback.id, boundaryRef.current.at);
    }
  }, [feed.isInitialSyncLoading, messages, onBoundaryMessage]);

  // Read far enough back that the oldest unread message is inside the loaded
  // range; `firstUnreadId` alone cannot tell us whether older rows are missing.
  const loadedPages = useRef(0);
  useEffect(() => {
    const oldest = messages[0]?.created_at;
    if (!firstUnreadAt || !oldest) return;
    if (parseUtcTimestamp(oldest).getTime() <= parseUtcTimestamp(firstUnreadAt).getTime()) return;
    if (loadedPages.current >= MAX_UNREAD_PAGES) return;
    if (!feed.hasOlder || feed.isLoadingOlder || feed.isLoading) return;
    loadedPages.current += 1;
    void feed.loadOlder();
  }, [feed.hasOlder, feed.isLoading, feed.isLoadingOlder, feed.loadOlder, firstUnreadAt, messages]);

  useEffect(() => {
    if (!loadOlderRef) return;
    loadOlderRef.current = () => void feed.loadOlder();
    return () => {
      loadOlderRef.current = null;
    };
  }, [feed.loadOlder, loadOlderRef]);

  // 「既読にする」は最初の未読が読み込み範囲の内側にあるときだけ有効にする。
  // 下限だけでなく上限も確認しないと、未読より古い末尾を既読にしてしまう。
  const boundaryTime = firstUnreadAt ? parseUtcTimestamp(firstUnreadAt).getTime() : null;
  const oldestTime = messages[0] ? parseUtcTimestamp(messages[0].created_at).getTime() : NaN;
  const newestTime = messages.at(-1)
    ? parseUtcTimestamp(messages[messages.length - 1].created_at).getTime()
    : NaN;
  const rangeReady =
    messages.length > 0 &&
    (boundaryTime === null || (oldestTime <= boundaryTime && newestTime >= boundaryTime));
  // 救済が必要なのは「古い履歴を読み切っても最初の未読に届かない」場合だけ。
  // 最初の未読が表示末尾より新しい場合は、読み足しても境界は動かない。
  const exhausted =
    !rangeReady &&
    messages.length > 0 &&
    boundaryTime !== null &&
    oldestTime > boundaryTime &&
    (loadedPages.current >= MAX_UNREAD_PAGES || (!feed.hasOlder && !feed.isLoadingOlder));
  // ページを開いた後に最初の未読が進んだ場合、古い境界をPOSTすると新しい未読を
  // 飛ばしてしまう。この会話では既読操作を許可しない（未読のまま次へは使える）。
  const boundaryTooOld =
    boundaryTime !== null && boundaryState !== null && boundaryTime > boundaryState.at;
  const rangeReadyRef = useRef<string | null>(null);
  useEffect(() => {
    const key = `${rangeReady}:${exhausted}:${boundaryTooOld}`;
    if (rangeReadyRef.current === key) return;
    rangeReadyRef.current = key;
    onRangeReady?.(rangeReady, exhausted, boundaryTooOld);
  }, [onRangeReady, rangeReady, exhausted, boundaryTooOld]);

  return (
    <MessageDecryptionsProvider
      decryptions={decryptions}
      mentionUserNames={mentionUserNames}
      encrypted={treatAsEncrypted}
      encryptedSince={encryptedSince}
    >
      <ForwardedMessagesProvider value={forwardedMessages}>
        {reactionsError ? (
          <div
            role="status"
            className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b px-4 py-2"
          >
            <span className="text-xs text-destructive">{t("reactions.fetchError")}</span>
          </div>
        ) : null}
        <MessageList
          key={channelId}
          channelId={undefined}
          messages={messages}
          isLoading={feed.isLoading}
          isError={feed.isError}
          currentUserId={null}
          viewerId={currentUserId}
          editingId={null}
          editingText=""
          onEditingTextChange={() => {}}
          onStartEdit={() => {}}
          onCancelEdit={() => {}}
          onSaveEdit={() => {}}
          onRequestDelete={() => {}}
          reactions={reactionGroups}
          attachments={attachments}
          kitchenCatalog={kitchenCatalog}
          canReact={canReact}
          onReaction={handleReaction}
          onPickReaction={openReactionPicker}
          canCompose={false}
          isOffline={!isOnline}
          unreadBoundaryId={firstUnreadId}
          unreadBoundaryAt={firstUnreadAt ? parseUtcTimestamp(firstUnreadAt).getTime() : null}
          hasOlder={feed.hasOlder}
          isLoadingOlder={feed.isLoadingOlder}
          onLoadOlder={() => void feed.loadOlder()}
          onRetry={() => window.location.reload()}
        />
        <ReactionPicker
          open={reactionMessageId !== null}
          onOpenChange={(open) => {
            if (!open) {
              closeReactionPicker();
            }
          }}
          onSelect={(emoji) => {
            if (reactionMessageId) {
              void handleReaction(reactionMessageId, emoji);
            }
          }}
        />
      </ForwardedMessagesProvider>
    </MessageDecryptionsProvider>
  );
}
