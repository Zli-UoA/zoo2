import { describe, expect, it } from "vite-plus/test";

import type { FeedMessage } from "../components/chat/messages/messageFormat";
import { isCacheCoveredByLive, mergeCachedAndLive } from "./useRealtime";

function row(id: string): FeedMessage {
  return {
    id,
    user_id: "user-1",
    content: id,
    created_at: "2026-01-01 00:00:00.000",
    parent_id: null,
  };
}

describe("mergeCachedAndLive", () => {
  it("keeps cached rows older than the live window", () => {
    const cached = [row("a"), row("b"), row("c"), row("d")];
    const live = [row("c"), row("d")];
    expect(mergeCachedAndLive(cached, live).map((r) => r.id)).toEqual(["a", "b", "c", "d"]);
  });

  it("uses live rows inside the overlapping range", () => {
    const cached = [row("a"), row("b")];
    const live = [row("b"), { ...row("c"), content: "fresh" }];
    const merged = mergeCachedAndLive(cached, live);
    expect(merged.map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(merged[2].content).toBe("fresh");
  });

  it("returns the other side when either list is empty or disjoint", () => {
    expect(mergeCachedAndLive([], [row("a")]).map((r) => r.id)).toEqual(["a"]);
    expect(mergeCachedAndLive([row("a")], []).map((r) => r.id)).toEqual(["a"]);
    expect(mergeCachedAndLive([row("a")], [row("b")]).map((r) => r.id)).toEqual(["b"]);
  });
});

describe("isCacheCoveredByLive", () => {
  it("reports the cache as covered once its newest row appears in live", () => {
    const cached = [row("a"), row("b"), row("c")];
    expect(isCacheCoveredByLive(cached, [row("c")])).toBe(true);
    expect(isCacheCoveredByLive(cached, [row("b"), row("c")])).toBe(true);
  });

  it("keeps merging while the live window has not reached the cached newest row", () => {
    const cached = [row("a"), row("b"), row("c")];
    expect(isCacheCoveredByLive(cached, [row("b")])).toBe(false);
    expect(isCacheCoveredByLive(cached, [])).toBe(false);
    expect(isCacheCoveredByLive(null, [row("b")])).toBe(true);
  });
});
