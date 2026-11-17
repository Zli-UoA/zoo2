import { describe, expect, it } from "vite-plus/test";
import {
  countNewTimelineEntries,
  mergeTimelineWindow,
  reachedTimelineTail,
  timelinePosition,
  visibleTimelineTail,
} from "./homeTimeline";

describe("timeline viewport for prefetch", () => {
  const items = Array.from({ length: 50 }, (_, index) => ({
    index,
    start: 100 + index * 240,
    end: 100 + (index + 1) * 240,
  }));

  it("does not use a tail range left over after a timeline resets its scroll position", () => {
    expect(visibleTimelineTail(items.slice(40), 0, 900)).toBe(-1);
  });

  it("uses visible rows rather than overscan to identify the viewport tail", () => {
    expect(visibleTimelineTail(items, 0, 900)).toBe(3);
    expect(visibleTimelineTail(items.slice(30), 10000, 900)).toBe(44);
  });

  it("excludes rows touching a viewport boundary and hidden scroll containers", () => {
    expect(visibleTimelineTail(items, 340, 240)).toBe(1);
    expect(visibleTimelineTail(items, 10000, 0)).toBe(-1);
  });
});

describe("bounded timeline refresh", () => {
  const rows = Array.from({ length: 1200 }, (_, i) => ({ id: `post-${i}`, content: `body ${i}` }));
  it("preserves unloaded-from-DOM history and updates/removes checked rows", () => {
    const edited = { ...rows[750], content: "edited" };
    const next = mergeTimelineWindow(
      rows,
      rows.slice(0, 50),
      [rows[750].id, rows[751].id],
      [edited],
      false,
    );
    expect(next).toHaveLength(1199);
    expect(next[750]).toEqual(edited);
    expect(next.some((row) => row.id === rows[751].id)).toBe(false);
    expect(next.at(-1)).toBe(rows.at(-1));
    expect(next[0]).toBe(rows[0]);
  });
  it("prepends new entries, removes deleted head entries and deduplicates the overlap", () => {
    const head = [{ id: "new", content: "new" }, ...rows.slice(1, 50)];
    const next = mergeTimelineWindow(rows, head, [], [], false);
    expect(next).toEqual([...head, ...rows.slice(50)]);
    expect(new Set(next.map((row) => row.id)).size).toBe(next.length);
  });
  it("keeps the reading window when a disconnected head has no overlap", () => {
    const head = [{ id: "new", content: "new" }];
    expect(mergeTimelineWindow(rows, head, [], [], false)).toEqual(rows);
    // Explicitly returning to the top replaces the disconnected window, avoiding a gap.
    expect(mergeTimelineWindow(rows, head, [], [], true)).toEqual(head);
  });
  it("keeps repost entries separate from the original post", () => {
    const current = [
      { id: "p", entry_id: "p" },
      { id: "p", entry_id: "repost:r" },
    ];
    expect(mergeTimelineWindow(current, current.slice(0, 1), ["repost:r"], [], false)).toEqual([
      current[0],
    ]);
  });
});
describe("retaining the loaded timeline tail", () => {
  it("does not stop at a newer microsecond in the same millisecond", () => {
    const tail = timelinePosition({ id: "z-tail", sort_key: "1767225600000001" });
    expect(reachedTimelineTail([{ id: "a-newer", sort_key: "1767225600000002" }], tail)).toBe(
      false,
    );
    expect(reachedTimelineTail([{ id: "z-tail", sort_key: "1767225600000001" }], tail)).toBe(true);
  });
  it("does not use ID collation to stop within an equal timestamp", () => {
    const tail = timelinePosition({ id: "z-tail", sort_key: "1767225600000001" });
    expect(reachedTimelineTail([{ id: "a-same-time", sort_key: "1767225600000001" }], tail)).toBe(
      false,
    );
    expect(
      reachedTimelineTail([{ id: "past-deleted-tail", sort_key: "1767225600000000" }], tail),
    ).toBe(true);
  });
  it("keeps exact ordering beyond Number precision and for repost entry IDs", () => {
    const tail = timelinePosition({
      id: "original",
      entry_id: "repost:tail",
      sort_key: "9007199254740993",
    });
    expect(
      reachedTimelineTail(
        [{ id: "original", entry_id: "repost:other", sort_key: "9007199254740994" }],
        tail,
      ),
    ).toBe(false);
    expect(
      reachedTimelineTail(
        [{ id: "original", entry_id: "repost:tail", sort_key: "9007199254740993" }],
        tail,
      ),
    ).toBe(true);
  });
});

it("does not count the same disconnected head repeatedly as new arrivals", () => {
  const old = [{ id: "old" }],
    head = [{ id: "new-2" }, { id: "new-1" }];
  expect(countNewTimelineEntries(head, old)).toBe(2);
  expect(countNewTimelineEntries(head, head)).toBe(0);
  expect(countNewTimelineEntries([{ id: "new-3" }, ...head], head)).toBe(1);
  expect(
    countNewTimelineEntries([{ id: "p", entry_id: "repost:r" }, { id: "p" }], [{ id: "p" }]),
  ).toBe(1);
});
