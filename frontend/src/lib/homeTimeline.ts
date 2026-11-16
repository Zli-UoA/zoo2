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

export function timelineEntryId(entry: TimelineEntry): string {
  return entry.entry_id ?? entry.id;
}

/** Count the new prefix against the last fetched head, even across a disconnected history gap. */
export function countNewTimelineEntries(
  head: readonly TimelineEntry[],
  previous: readonly TimelineEntry[],
): number {
  const known = new Set(previous.map(timelineEntryId));
  const overlap = head.findIndex((entry) => known.has(timelineEntryId(entry)));
  return overlap < 0 ? head.length : overlap;
}

/** Keep history without refetching it; only an authoritative window may remove rows. */
export function mergeTimelineWindow<T extends TimelineEntry>(
  current: readonly T[],
  head: readonly T[],
  checkedIds: readonly string[],
  checked: readonly T[],
  atTop: boolean,
): T[] {
  const headIds = new Set(head.map(timelineEntryId));
  let lastOverlap = -1;
  for (let index = current.length - 1; index >= 0; index--) {
    if (headIds.has(timelineEntryId(current[index]))) {
      lastOverlap = index;
      break;
    }
  }
  const replaceHead = atTop || lastOverlap >= 0;
  const history = replaceHead ? (lastOverlap >= 0 ? current.slice(lastOverlap + 1) : []) : current;
  const checkedSet = new Set(checkedIds);
  const updates = new Map([...head, ...checked].map((entry) => [timelineEntryId(entry), entry]));
  const previous = new Map(current.map((entry) => [timelineEntryId(entry), entry]));
  const seen = new Set<string>();
  return [...(replaceHead ? head : []), ...history].flatMap((entry) => {
    const id = timelineEntryId(entry);
    if (seen.has(id) || (checkedSet.has(id) && !updates.has(id))) return [];
    seen.add(id);
    const next = updates.get(id) ?? entry;
    const old = previous.get(id);
    // Preserve row props for memoized rich content when nothing changed.
    return [old && (old === next || JSON.stringify(old) === JSON.stringify(next)) ? old : next];
  });
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

/** Ignore stale virtual ranges and overscan when deciding whether the viewport is near the tail. */
export function visibleTimelineTail(
  items: readonly { index: number; start: number; end: number }[],
  scrollTop: number,
  viewportHeight: number,
): number {
  if (viewportHeight <= 0) return -1;
  const bottom = scrollTop + viewportHeight;
  for (let i = items.length - 1; i >= 0; i--)
    if (items[i].start < bottom && items[i].end > scrollTop) return items[i].index;
  return -1;
}
