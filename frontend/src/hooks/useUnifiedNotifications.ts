import { useCallback, useMemo } from "react";
import { useNotifications } from "./useNotifications";
import { useActivityPubNotifications } from "./useActivityPubNotifications";
import { unifiedNotificationFeed } from "../components/chat/notifications/groupNotifications";

/** Read controls and ordering for the shared notifications screen and rail badge. */
export function useUnifiedNotifications() {
  const channel = useNotifications();
  const activitypub = useActivityPubNotifications();
  const feed = useMemo(
    () =>
      unifiedNotificationFeed(channel.notifications, activitypub.items, !!activitypub.next_cursor),
    [channel.notifications, activitypub.items, activitypub.next_cursor],
  );
  const markRead = useCallback(
    async (ids?: readonly string[]) => {
      if (ids === undefined) {
        await Promise.all([channel.markRead(), activitypub.markRead()]);
        return;
      }
      const local = ids.filter((id) => !id.startsWith("activitypub:"));
      const remote = new Set(
        ids.filter((id) => id.startsWith("activitypub:")).map((id) => id.slice(12)),
      );
      for (const mirror of feed.mirrors)
        if (local.includes(mirror.channelId)) remote.add(mirror.item.id);
      await Promise.all([
        channel.markRead(local),
        remote.size ? activitypub.markRead([...remote]) : Promise.resolve(),
      ]);
    },
    [channel.markRead, activitypub.markRead, feed.mirrors],
  );
  return {
    channel,
    activitypub,
    entries: feed.entries,
    markRead,
    unreadCount: channel.unreadCount + activitypub.unread_count - feed.duplicateUnreadCount,
  };
}
