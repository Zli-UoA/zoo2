import { eq, useLiveQuery } from "@tanstack/react-db";
import { getCollections } from "../lib/electric/collections";
import { useCurrentUser } from "./useCurrentUser";

export function useNotifications() {
  const { currentUserId } = useCurrentUser();
  const collection = currentUserId ? getCollections().notifications(currentUserId) : null;
  const query = useLiveQuery(
    (q) =>
      collection && currentUserId
        ? q
            .from({ notifications: collection })
            .where(({ notifications }) => eq(notifications.user_id, currentUserId))
            .orderBy(({ notifications }) => notifications.created_at, "desc")
            .orderBy(({ notifications }) => notifications.id, "desc")
        : undefined,
    [collection, currentUserId],
  );
  const notifications = query.data ?? [];
  const unread = notifications.filter((notification) => !notification.is_read);
  async function markRead(id?: string) {
    if (!collection) return;
    const ids = unread.filter((row) => id === undefined || row.id === id).map((row) => row.id);
    if (ids.length === 0) return;
    const transaction = collection.update(ids, (rows) => {
      for (const row of rows) row.is_read = true;
    });
    await transaction.isPersisted.promise;
  }
  return { ...query, notifications, unreadCount: unread.length, markRead };
}
