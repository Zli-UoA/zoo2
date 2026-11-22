import { useEffect, useMemo, useRef } from "react";

import { parseUtcTimestamp } from "../../lib/timestamps";
import { useCurrentUser } from "../../hooks/useCurrentUser";
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
  firstUnreadAt,
  onBoundaryMessage,
}: {
  channelId: string;
  firstUnreadAt: string | null;
  onBoundaryMessage: (messageId: string | null) => void;
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

  // Read far enough back to show the unread divider when it is close by.
  const loadedPages = useRef(0);
  useEffect(() => {
    const hasBoundary = !firstUnreadId || messages.some((m) => m.id === firstUnreadId);
    if (hasBoundary || loadedPages.current >= MAX_UNREAD_PAGES) return;
    if (!feed.hasOlder || feed.isLoadingOlder || feed.isLoading) return;
    loadedPages.current += 1;
    void feed.loadOlder();
  }, [feed.hasOlder, feed.isLoading, feed.isLoadingOlder, feed.loadOlder, firstUnreadId, messages]);

  return (
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
  );
}
