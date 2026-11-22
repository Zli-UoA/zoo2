import { useEffect, useMemo, useRef, type MutableRefObject } from "react";

import { parseUtcTimestamp } from "../../lib/timestamps";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useMessageDecryptions } from "../../hooks/useMessageDecryptions";
import { useEncryptedDisplay } from "../../hooks/useEncryptedDisplay";
import {
  MessageDecryptionsProvider,
  firstEncryptedAt,
  withUnencryptedRows,
} from "../../lib/e2ee/messageDecryption";
import { useKitchenCatalog } from "../../hooks/useKitchenCatalog";
import { useMessageAttachments } from "../../hooks/useAttachments";
import { useMessageFeed, useMessageReactions, useUsers } from "../../hooks/useRealtime";
import { groupAttachmentsByMessage } from "../../lib/attachments";
import { groupReactions, reactionUserLabel } from "../../lib/emoji/reactions";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { MessageList } from "./messages/MessageList";
import type { FeedMessage } from "./messages/messageFormat";

/** How many pages back we look for the unread divider before leaving it to scrolling. */
const MAX_UNREAD_PAGES = 5;

/**
 * One conversation's unread history, rendered with the ordinary message list
 * but without composer, thread pane or automatic read receipts.
 */
export function CatchUpMessagePage({
  channelId,
  channelName,
  encrypted = false,
  firstUnreadAt,
  onBoundaryMessage,
  onRangeReady,
  loadOlderRef,
}: {
  channelId: string;
  channelName?: string;
  encrypted?: boolean;
  firstUnreadAt: string | null;
  onBoundaryMessage: (messageId: string | null, createdAt: number | null) => void;
  onRangeReady?: (ready: boolean, exhausted: boolean) => void;
  loadOlderRef?: MutableRefObject<(() => void) | null>;
}) {
  const feed = useMessageFeed(channelId);
  const { currentUserId } = useCurrentUser();
  const messages = feed.data as FeedMessage[];
  const usersQuery = useUsers();
  const reactionsQuery = useMessageReactions(channelId);
  const { isOnline } = useOfflineStatus();
  const reactionRows = reactionsQuery.data ?? [];
  const { catalog } = useKitchenCatalog(
    reactionRows.some((row) => row.emoji.startsWith("kitchen:")),
  );
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
  const reactionGroups = useMemo(() => {
    const names = Object.fromEntries(
      (usersQuery.data ?? []).map((user) => [user.id, reactionUserLabel(user)]),
    );
    const groups = groupReactions(reactionRows, null);
    for (const reactions of groups.values()) {
      for (const reaction of reactions) {
        reaction.userNames = { ...reaction.userNames, ...names };
        reaction.usersLoading = usersQuery.isLoading;
      }
    }
    return groups;
  }, [reactionRows, usersQuery.data, usersQuery.isLoading]);

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
  useEffect(() => {
    if (feed.isInitialSyncLoading || messages.length === 0) return;
    if (!boundaryRef.current) {
      const newest = messages[messages.length - 1];
      const at = parseUtcTimestamp(newest.created_at).getTime();
      boundaryRef.current = { id: newest.id, at };
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
  const rangeReadyRef = useRef<string | null>(null);
  useEffect(() => {
    const key = `${rangeReady}:${exhausted}`;
    if (rangeReadyRef.current === key) return;
    rangeReadyRef.current = key;
    onRangeReady?.(rangeReady, exhausted);
  }, [onRangeReady, rangeReady, exhausted]);

  return (
    <MessageDecryptionsProvider
      decryptions={decryptions}
      mentionUserNames={mentionUserNames}
      encrypted={treatAsEncrypted}
      encryptedSince={encryptedSince}
    >
      <MessageList
        key={channelId}
        channelId={undefined}
        messages={messages}
        isLoading={feed.isLoading}
        isError={feed.isError}
        currentUserId={null}
        editingId={null}
        editingText=""
        onEditingTextChange={() => {}}
        onStartEdit={() => {}}
        onCancelEdit={() => {}}
        onSaveEdit={() => {}}
        onRequestDelete={() => {}}
        reactions={reactionGroups}
        attachments={attachments}
        kitchenCatalog={catalog}
        canReact={false}
        onReaction={() => {}}
        onPickReaction={() => {}}
        canCompose={false}
        isOffline={!isOnline}
        unreadBoundaryId={firstUnreadId}
        unreadBoundaryAt={firstUnreadAt ? parseUtcTimestamp(firstUnreadAt).getTime() : null}
        hasOlder={feed.hasOlder}
        isLoadingOlder={feed.isLoadingOlder}
        onLoadOlder={() => void feed.loadOlder()}
        onRetry={() => window.location.reload()}
      />
    </MessageDecryptionsProvider>
  );
}
