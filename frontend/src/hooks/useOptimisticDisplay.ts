import { useLiveQuery } from "@tanstack/react-db";
import { useMemo } from "react";
import { useCollections } from "../lib/electric/collections-context";
import { displayChanges, optimisticDisplay } from "../lib/electric/optimisticDisplay";
import type { MessageDisplayData } from "../lib/electric/types";

/** Share the same local overlay between the timeline, thread root and replies. */
export function useOptimisticDisplay<T extends { id: string; display_data?: MessageDisplayData }>(
  channelId: string | null,
  data: T[] | undefined,
): T[] | undefined {
  const { messages, reactions } = useCollections();
  const messageRows = useLiveQuery((q) =>
    channelId ? q.from({ row: messages(channelId) }) : undefined,
  );
  const reactionRows = useLiveQuery((q) =>
    channelId ? q.from({ row: reactions(channelId) }) : undefined,
  );
  return useMemo(() => {
    if (!channelId || !data) return data;
    const messageTable = messages(channelId);
    const reactionTable = reactions(channelId);
    // `base` is TanStack DB's public authoritative state, before optimistic writes.
    // Read the virtual tables directly, so an unfinished initial live query never
    // looks like an optimistic deletion of all the base rows.
    const localMessages = new Map(messageTable.entries());
    const localReactions = new Map(reactionTable.entries());
    const changes = displayChanges(
      messageTable.base,
      localMessages,
      reactionTable.base,
      localReactions,
    );
    return data.map((row) =>
      row.display_data
        ? {
            ...row,
            display_data: optimisticDisplay(row.display_data, row.id, changes),
          }
        : row,
    );
  }, [channelId, data, messages, reactions, messageRows.data, reactionRows.data]);
}
