export interface TimelineEntry {
  id: string;
  entry_id?: string;
  /** Exact PostgreSQL epoch microseconds, serialized as text. */
  sort_key?: string;
}
export interface TimelinePosition {
  id: string;
  sortKey: bigint | null;
}
export function timelinePosition(entry: TimelineEntry): TimelinePosition {
  return {
    id: entry.entry_id ?? entry.id,
    sortKey: entry.sort_key && /^-?\d+$/.test(entry.sort_key) ? BigInt(entry.sort_key) : null,
  };
}
export function reachedTimelineTail(
  entries: readonly TimelineEntry[],
  tail: TimelinePosition | null,
): boolean {
  if (!tail || entries.some((entry) => (entry.entry_id ?? entry.id) === tail.id)) return true;
  const last = entries.at(-1);
  const key = last ? timelinePosition(last).sortKey : null;
  // Do not compare IDs using JavaScript collation or round timestamps to milliseconds.
  // If the tail was deleted, continue through ties until crossing its exact timestamp.
  return key !== null && tail.sortKey !== null && key < tail.sortKey;
}
