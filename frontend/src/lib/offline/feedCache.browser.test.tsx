import { afterEach, describe, expect, it } from "vite-plus/test";

import {
  blockFeedCache,
  clearFeedCache,
  deleteFeedCache,
  enableFeedCache,
  loadFeedCache,
  saveFeedCache,
} from "./feedCache";

const NAMESPACE = "user-feed-cache-test";
const OTHER = "user-feed-cache-other";

afterEach(async () => {
  await deleteFeedCache(NAMESPACE);
  await deleteFeedCache(OTHER);
});

describe("feed cache", () => {
  it("returns the newest rows that were saved for a channel", async () => {
    const rows = Array.from({ length: 5 }, (_, index) => ({ id: `m${index}` }));
    await saveFeedCache(NAMESPACE, "general", rows, 3);
    const entry = await loadFeedCache<{ id: string }>(NAMESPACE, "general");
    expect(entry?.rows.map((row) => row.id)).toEqual(["m2", "m3", "m4"]);
    expect(await loadFeedCache(NAMESPACE, "random")).toBeNull();
  });

  it("keeps each user's cache separate and removes it on logout", async () => {
    await saveFeedCache(NAMESPACE, "general", [{ id: "a" }], 50);
    expect(await loadFeedCache(OTHER, "general")).toBeNull();
    await deleteFeedCache(NAMESPACE);
    expect(await loadFeedCache(NAMESPACE, "general")).toBeNull();
  });

  it("drops a channel's cache when it is cleared (all messages deleted)", async () => {
    await saveFeedCache(NAMESPACE, "general", [{ id: "a" }], 50);
    await saveFeedCache(NAMESPACE, "random", [{ id: "b" }], 50);
    await clearFeedCache(NAMESPACE, "general");
    expect(await loadFeedCache(NAMESPACE, "general")).toBeNull();
    expect((await loadFeedCache<{ id: string }>(NAMESPACE, "random"))?.rows).toEqual([{ id: "b" }]);
  });

  it("does not recreate a deleted cache when a pending save runs after logout", async () => {
    blockFeedCache(NAMESPACE);
    await deleteFeedCache(NAMESPACE);
    await saveFeedCache(NAMESPACE, "general", [{ id: "late" }], 50);
    expect(await loadFeedCache(NAMESPACE, "general")).toBeNull();
    enableFeedCache(NAMESPACE);
    await saveFeedCache(NAMESPACE, "general", [{ id: "again" }], 50);
    expect(await loadFeedCache(NAMESPACE, "general")).not.toBeNull();
  });

  it("does not store an empty page", async () => {
    await saveFeedCache(NAMESPACE, "general", [], 50);
    expect(await loadFeedCache(NAMESPACE, "general")).toBeNull();
  });
});
