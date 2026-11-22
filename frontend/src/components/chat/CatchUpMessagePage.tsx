import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";

import { parseUtcTimestamp } from "../../lib/timestamps";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useMessageDecryptions } from "../../hooks/useMessageDecryptions";
import {
  encryptedChannelsVersion,
  noteEncryptedChannel,
  subscribeEncryptedChannels,
} from "../../lib/e2ee/encryptionGuard";
import {
  MessageDecryptionsProvider,
  firstEncryptedAt,
  useEncryptedChannel,
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
}: {
  channelId: string;
  channelName?: string;
  encrypted?: boolean;
  firstUnreadAt: string | null;
  onBoundaryMessage: (messageId: string | null) => void;
  onRangeReady?: (ready: boolean) => void;
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
  // 暗号化の判定は MessagePane と同じ基準を使う（ciphertext の有無だけでは
  // epoch だけの行や edm: の暗号化DMを取りこぼす）。
  const isEncryptedDm = !!channelName?.startsWith("edm:");
  if (encrypted || isEncryptedDm) noteEncryptedChannel(channelId);
  useSyncExternalStore(subscribeEncryptedChannels, encryptedChannelsVersion);
  const notedEncrypted = useEncryptedChannel();
  const treatAsEncrypted =
    encrypted || isEncryptedDm || notedEncrypted || messages.some((row) => row.ciphertext != null);
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

  const boundaryRef = useRef<string | null>(null);
  useEffect(() => {
    if (boundaryRef.current !== null || feed.isInitialSyncLoading || messages.length === 0) return;
    const newest = messages[messages.length - 1].id;
    boundaryRef.current = newest;
    onBoundaryMessage(newest);
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

  // 「既読にする」は最初の未読が読み込み範囲内にあるときだけ有効にする。
  const rangeReady =
    messages.length > 0 &&
    (!firstUnreadAt ||
      (!!messages[0] &&
        parseUtcTimestamp(messages[0].created_at).getTime() <=
          parseUtcTimestamp(firstUnreadAt).getTime()));
  const rangeReadyRef = useRef<boolean | null>(null);
  useEffect(() => {
    if (rangeReadyRef.current === rangeReady) return;
    rangeReadyRef.current = rangeReady;
    onRangeReady?.(rangeReady);
  }, [onRangeReady, rangeReady]);

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
        onOpenThread={() => {}}
        canCompose={false}
        isOffline={!isOnline}
        unreadBoundaryId={firstUnreadId}
        hasOlder={feed.hasOlder}
        isLoadingOlder={feed.isLoadingOlder}
        onLoadOlder={() => void feed.loadOlder()}
        onRetry={() => window.location.reload()}
      />
    </MessageDecryptionsProvider>
  );
}
