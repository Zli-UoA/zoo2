import { afterEach, describe, expect, it } from "vite-plus/test";
import type { VirtualItem } from "@tanstack/react-virtual";

import {
  buildChannelScrollKey,
  clearChannelMeasurementCache,
  computeInitialScrollPlan,
  getChannelMeasurementCache,
  recordLastViewportRect,
  saveChannelMeasurementCache,
} from "./messageScroll";

function snapshotItem(key: string, index: number, size: number, start = 0): VirtualItem {
  return {
    key,
    index,
    size,
    start,
    end: start + size,
    lane: 0,
  };
}

describe("channel scroll measurement cache & initial plan", () => {
  afterEach(() => {
    clearChannelMeasurementCache();
  });

  it("computes bottom initialOffset even on an uncached channel", () => {
    const itemIds = Array.from({ length: 50 }, (_, index) => `m-${index}`);
    const plan = computeInitialScrollPlan({
      itemIds,
      defaultRowHeight: 64,
      defaultViewportHeight: 600,
      hasTargetMessageId: false,
      targetIndex: -1,
      unreadIndex: -1,
    });

    expect(plan.initialMeasurementsCache).toBeUndefined();
    expect(plan.estimatedRowHeight).toBe(64);
    expect(plan.initialRect).toEqual({ width: 0, height: 600 });
    expect(plan.initialOffset).toBe(50 * 64 - 600);

    recordLastViewportRect(720, 480);
    const afterRect = computeInitialScrollPlan({
      itemIds,
      defaultRowHeight: 64,
      defaultViewportHeight: 600,
      hasTargetMessageId: false,
      targetIndex: -1,
      unreadIndex: -1,
    });
    expect(afterRect.initialRect).toEqual({ width: 720, height: 480 });
    expect(afterRect.initialOffset).toBe(50 * 64 - 480);
  });

  it("centers targetIndex or aligns unreadIndex when specified", () => {
    const itemIds = Array.from({ length: 50 }, (_, index) => `m-${index}`);
    const targetPlan = computeInitialScrollPlan({
      itemIds,
      defaultRowHeight: 64,
      defaultViewportHeight: 600,
      hasTargetMessageId: true,
      targetIndex: 20,
      unreadIndex: 35,
    });
    expect(targetPlan.initialOffset).toBe(20 * 64 - (600 - 64) / 2);

    const pendingTargetPlan = computeInitialScrollPlan({
      itemIds,
      defaultRowHeight: 64,
      defaultViewportHeight: 600,
      hasTargetMessageId: true,
      targetIndex: -1,
      unreadIndex: 35,
    });
    expect(pendingTargetPlan.initialOffset).toBe(0);

    const unreadPlan = computeInitialScrollPlan({
      itemIds,
      defaultRowHeight: 64,
      defaultViewportHeight: 600,
      hasTargetMessageId: false,
      targetIndex: -1,
      unreadIndex: 35,
    });
    expect(unreadPlan.initialOffset).toBe(35 * 64);
  });

  it("stores measured heights per channel/user, reuses them for initial plan, and invalidates on width change", () => {
    const key = buildChannelScrollKey("general", "u1");
    saveChannelMeasurementCache({
      key,
      containerWidth: 800,
      viewportHeight: 650,
      snapshotItems: [snapshotItem("m-0", 0, 100, 0), snapshotItem("m-1", 1, 140, 100)],
      fallbackRowHeight: 64,
    });

    const cached = getChannelMeasurementCache(key);
    expect(cached).toBeDefined();
    expect(cached?.estimatedRowHeight).toBe(120);

    const plan = computeInitialScrollPlan({
      itemIds: ["m-0", "m-1", "m-2"],
      cached,
      defaultRowHeight: 64,
      defaultViewportHeight: 600,
      hasTargetMessageId: false,
      targetIndex: -1,
      unreadIndex: -1,
    });
    expect(plan.initialMeasurementsCache).toEqual([
      snapshotItem("m-0", 0, 100, 0),
      snapshotItem("m-1", 1, 140, 100),
    ]);
    // 2 cached rows (240px) + 1 unseen row (learned mean 120px) = 360px (< viewport 650) -> offset 0
    expect(plan.initialOffset).toBe(0);
    expect(plan.estimatedRowHeight).toBe(120);

    // Resaving with a different container width discards stale wrapped heights cleanly.
    saveChannelMeasurementCache({
      key,
      containerWidth: 390,
      viewportHeight: 650,
      snapshotItems: [snapshotItem("m-1", 1, 180, 0)],
      fallbackRowHeight: 64,
    });
    const resized = getChannelMeasurementCache(key);
    expect(resized?.sizes.has("m-0")).toBe(false);
    expect(resized?.sizes.get("m-1")).toBe(180);
  });

  it("bounds cached row sizes per channel and channel count via LRU", () => {
    for (let c = 0; c < 40; c += 1) {
      const k = buildChannelScrollKey(`c-${c}`, "u1");
      saveChannelMeasurementCache({
        key: k,
        containerWidth: 700,
        viewportHeight: 500,
        snapshotItems: Array.from({ length: 220 }, (_, i) => snapshotItem(`m-${i}`, i, 80, i * 80)),
        fallbackRowHeight: 64,
      });
    }

    // Oldest channels evicted past MAX_CHANNEL_SCROLL_CACHE (32).
    expect(getChannelMeasurementCache(buildChannelScrollKey("c-0", "u1"))).toBeUndefined();
    const newest = getChannelMeasurementCache(buildChannelScrollKey("c-39", "u1"));
    expect(newest?.sizes.size).toBe(200);
    // Most recent rows retained (indices 20..219).
    expect(newest?.sizes.has("m-0")).toBe(false);
    expect(newest?.sizes.has("m-219")).toBe(true);
  });
});
