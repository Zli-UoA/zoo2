import type { VirtualItem } from "@tanstack/react-virtual";
import { timelineEntryId, type TimelineEntry } from "./homeTimeline";

export interface HomeTimelineSnapshot<T> {
  posts: T[];
  hasMore: boolean;
  scrollOffset: number;
  listOffset: number;
  measurements: VirtualItem[];
  anchor?: { id: string; offset: number };
}

/** Tab-memory cache for back navigation, scoped by workspace, account and history entry. */
export function createHomeTimelineCache<T extends TimelineEntry>(capacity = 8) {
  const snapshots = new Map<string, HomeTimelineSnapshot<T>>();
  return {
    get: (key: string) => snapshots.get(key),
    save(key: string, snapshot: HomeTimelineSnapshot<T>) {
      snapshots.delete(key);
      snapshots.set(key, snapshot);
      while (snapshots.size > capacity) snapshots.delete(snapshots.keys().next().value!);
    },
    remove(scope: string, postId: string, update: (post: T) => T = (post) => post) {
      for (const [key, snapshot] of snapshots) {
        if (!key.startsWith(scope)) continue;
        const removed = new Set(
          snapshot.posts.filter((post) => post.id === postId).map(timelineEntryId),
        );
        const measured = new Map(snapshot.measurements.map((row) => [String(row.key), row]));
        const measurements: VirtualItem[] = [];
        let oldStart = snapshot.listOffset,
          start = snapshot.listOffset,
          delta = 0,
          index = 0;
        for (const post of snapshot.posts) {
          const id = timelineEntryId(post),
            row = measured.get(id),
            size = row?.size ?? 240;
          if (removed.has(id)) {
            if (oldStart < snapshot.scrollOffset)
              delta += Math.min(size, snapshot.scrollOffset - oldStart);
          } else {
            if (row) measurements.push({ ...row, index, start, end: start + size });
            index++;
            start += size;
          }
          oldStart += size;
        }
        const anchorIndex = snapshot.posts.findIndex(
          (post) => timelineEntryId(post) === snapshot.anchor?.id,
        );
        const nextAnchor = snapshot.posts
          .slice(anchorIndex + 1)
          .find((post) => !removed.has(timelineEntryId(post)));
        const anchor =
          snapshot.anchor && removed.has(snapshot.anchor.id)
            ? nextAnchor
              ? { id: timelineEntryId(nextAnchor), offset: 0 }
              : undefined
            : snapshot.anchor;
        snapshots.set(key, {
          ...snapshot,
          posts: snapshot.posts.filter((post) => post.id !== postId).map(update),
          measurements,
          anchor,
          scrollOffset: Math.max(0, snapshot.scrollOffset - delta),
        });
      }
    },
  };
}
