import { useCallback } from "react";
import { eq, useLiveQuery } from "@tanstack/react-db";
import { useCollections } from "../lib/electric/collections-context";
import { useCurrentUser } from "./useCurrentUser";

export function useNotifications() {
  const { currentUserId } = useCurrentUser();
  const collections = useCollections();
  const collection = currentUserId ? collections.notifications(currentUserId) : null;
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
  const markRead = useCallback(
    async (id?: string | readonly string[]) => {
      if (!collection) return;
      const requested = Array.isArray(id) ? new Set(id) : null;
      const ids = unread
        .filter((row) => id === undefined || (requested ? requested.has(row.id) : row.id === id))
        .map((row) => row.id);
      if (ids.length === 0) return;
      if (requested) {
        // One transaction per visible ID: the multi-row writer calls read-all, which would also
        // acknowledge notifications outside the viewport or arriving during this request.
        await Promise.all(
          ids.map(
            (key) =>
              collection.update(key, (row) => {
                row.is_read = true;
              }).isPersisted.promise,
          ),
        );
        return;
      }
      const transaction = collection.update(ids, (rows) => {
        for (const row of rows) row.is_read = true;
      });
      await transaction.isPersisted.promise;
    },
    [collection, notifications],
  );
  return { ...query, notifications, unreadCount: unread.length, markRead };
}
