import { useWorkspaceEmojis } from "../lib/emoji/workspace";
import { useMemo, useSyncExternalStore } from "react";
import { useCollections } from "../lib/electric/collections-context";
import { displayChanges, optimisticDisplay } from "../lib/electric/optimisticDisplay";
import type { MessageDisplayData } from "../lib/electric/types";

/**
 * コレクションが変わるたびに増える版。再描画のきっかけにだけ使う。
 *
 * `from(collection)` の live query を作ると、on-demand のコレクションでは、全行を取る要求になる。
 * そこで、行を取らずに変更だけを購読する (`includeInitialState: false`)。
 */
function useCollectionVersion(collection: {
  subscribeChanges: (
    callback: () => void,
    options?: { includeInitialState?: boolean },
  ) => {
    unsubscribe: () => void;
  };
}): number {
  const subscription = useMemo(() => {
    let version = 0;
    const listeners = new Set<() => void>();
    return {
      subscribe: (listener: () => void) => {
        listeners.add(listener);
        const handle = collection.subscribeChanges(
          () => {
            version += 1;
            for (const notify of listeners) notify();
          },
          { includeInitialState: false },
        );
        return () => {
          listeners.delete(listener);
          handle.unsubscribe();
        };
      },
      getSnapshot: () => version,
    };
  }, [collection]);
  return useSyncExternalStore(subscription.subscribe, subscription.getSnapshot, () => 0);
}

/** Share the same local overlay between the timeline, thread root and replies. */
export function useOptimisticDisplay<T extends { id: string; display_data?: MessageDisplayData }>(
  channelId: string | null,
  data: T[] | undefined,
): T[] | undefined {
  const { messages, reactions } = useCollections();
  const messageTable = channelId ? messages(channelId) : null;
  const reactionTable = channelId ? reactions(channelId) : null;
  const messageVersion = useCollectionVersion(
    messageTable ?? { subscribeChanges: () => ({ unsubscribe: () => {} }) },
  );
  const reactionVersion = useCollectionVersion(
    reactionTable ?? { subscribeChanges: () => ({ unsubscribe: () => {} }) },
  );
  const changes = useMemo(() => {
    if (!messageTable || !reactionTable) return undefined;
    // `base` is TanStack DB's public authoritative state, before optimistic writes.
    // Read the virtual tables directly, so an unfinished initial live query never
    // looks like an optimistic deletion of all the base rows.
    const localMessages = new Map(messageTable.entries());
    const localReactions = new Map(reactionTable.entries());
    return displayChanges(messageTable.base, localMessages, reactionTable.base, localReactions);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [messageTable, reactionTable, messageVersion, reactionVersion]);
  const pendingCustom = [...(changes?.reactions.values() ?? [])].some((rows) =>
    [...rows.values()].some((row) => row?.emoji.startsWith(":")),
  );
  const { emojis } = useWorkspaceEmojis(pendingCustom);
  return useMemo(() => {
    if (!data || !changes) return data;
    return data.map((row) =>
      row.display_data
        ? {
            ...row,
            display_data: optimisticDisplay(row.display_data, row.id, changes, emojis),
          }
        : row,
    );
  }, [data, changes, emojis]);
}
