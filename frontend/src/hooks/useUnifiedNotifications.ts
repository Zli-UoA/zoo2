import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useMemo,
  type ReactNode,
} from "react";
import { useCurrentUser } from "./useCurrentUser";
import { useCollections } from "../lib/electric/collections-context";
import { useNotifications } from "./useNotifications";
import { useActivityPubNotifications } from "./useActivityPubNotifications";
import { unifiedNotificationFeed } from "../components/chat/notifications/groupNotifications";

function useNotificationFeed() {
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

const NotificationsContext = createContext<ReturnType<typeof useNotificationFeed> | null>(null);

function NotificationFeedProvider({ children }: { children: ReactNode }) {
  const value = useNotificationFeed();
  return createElement(NotificationsContext.Provider, { value }, children);
}

/** One feed and live subscription for the rail and screen, scoped to the active account. */
export function NotificationsProvider({ children }: { children: ReactNode }) {
  const { currentUserId } = useCurrentUser();
  const { config } = useCollections();
  return createElement(NotificationFeedProvider, {
    key: JSON.stringify([config.apiBaseUrl, config.devUserId, currentUserId]),
    children,
  });
}

export function useUnifiedNotifications() {
  const notifications = useContext(NotificationsContext);
  if (!notifications) throw new Error("NotificationsProvider is required");
  return notifications;
}
