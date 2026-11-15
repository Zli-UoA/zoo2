import { expect, it } from "vite-plus/test";
import { createHomeTimelineCache, type HomeTimelineSnapshot } from "./homeTimelineCache";

function snapshot(): HomeTimelineSnapshot<{ id: string; entry_id?: string; parent?: string }> {
  return {
    posts: [{ id: "p" }, { id: "p", entry_id: "repost:r" }, { id: "q", parent: "p" }],
    hasMore: true,
    scrollOffset: 250,
    listOffset: 50,
    measurements: ["p", "repost:r", "q"].map((key, index) => ({
      key,
      index,
      size: 100,
      start: 50 + index * 100,
      end: 150 + index * 100,
      lane: 0,
    })),
  };
}
it("removes all entries for the deleted post, clears references and adjusts the reading position within the same account", () => {
  const cache = createHomeTimelineCache<ReturnType<typeof snapshot>["posts"][number]>();
  cache.save("account-a:home", snapshot());
  cache.save("account-b:home", snapshot());
  cache.remove("account-a:", "p", (post) => ({ ...post, parent: undefined }));
  const saved = cache.get("account-a:home")!;
  expect(saved.posts).toEqual([{ id: "q", parent: undefined }]);
  expect(saved.scrollOffset).toBe(50);
  expect(saved.measurements).toEqual([
    { key: "q", index: 0, size: 100, start: 50, end: 150, lane: 0 },
  ]);
  expect(cache.get("account-b:home")!.posts).toHaveLength(3);
});
it("keeps the next row at the top when the partially visible row was deleted", () => {
  const cache = createHomeTimelineCache<{ id: string }>();
  const state = snapshot();
  state.scrollOffset = 75;
  cache.save("a:home", state);
  cache.remove("a:", "p");
  expect(cache.get("a:home")!.scrollOffset).toBe(50);
});
it("bounds retained history entries and refreshes their eviction order", () => {
  const cache = createHomeTimelineCache<{ id: string }>(2);
  cache.save("a", snapshot());
  cache.save("b", snapshot());
  cache.save("a", snapshot());
  cache.save("c", snapshot());
  expect(cache.get("b")).toBeUndefined();
  expect(cache.get("a")).toBeDefined();
  expect(cache.get("c")).toBeDefined();
});
it("preserves sparse measured rows after deleting unmeasured entries above them", () => {
  const cache = createHomeTimelineCache<{ id: string }>();
  const state = snapshot();
  state.measurements = [{ key: "q", index: 2, start: 530, end: 630, size: 100, lane: 0 }];
  state.scrollOffset = 550;
  state.anchor = { id: "q", offset: -20 };
  cache.save("a:home", state);
  cache.remove("a:", "p");
  const next = cache.get("a:home")!;
  expect(next.scrollOffset).toBe(70);
  expect(next.measurements[0]).toMatchObject({ key: "q", index: 0, start: 50, end: 150 });
  expect(next.anchor).toEqual({ id: "q", offset: -20 });
});
