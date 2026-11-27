import { describe, expect, it } from "vite-plus/test";
import {
  decideUnreadLoad,
  isUnreadCovered,
  MAX_UNREAD_PAGES,
  resolveUnreadBoundaryId,
} from "./messageUnreadBoundary";

describe("resolveUnreadBoundaryId", () => {
  it("渡された境界は null でもそのまま使い、既読状態より優先する", () => {
    expect(resolveUnreadBoundaryId("provided", "server", true, false)).toBe("provided");
    expect(resolveUnreadBoundaryId(null, "server", false, false)).toBeNull();
  });

  it("境界が渡されないときは、取得中・失敗中は区切りを出さない", () => {
    expect(resolveUnreadBoundaryId(undefined, "server", false, false)).toBe("server");
    expect(resolveUnreadBoundaryId(undefined, "server", true, false)).toBeNull();
    expect(resolveUnreadBoundaryId(undefined, "server", false, true)).toBeNull();
    expect(resolveUnreadBoundaryId(undefined, null, false, false)).toBeNull();
  });
});

describe("isUnreadCovered", () => {
  const covered = (overrides: Partial<Parameters<typeof isUnreadCovered>[0]> = {}) =>
    isUnreadCovered({
      lastRead: 1_000,
      oldestCreatedAt: 1_100,
      hasOlder: true,
      loadedPages: 0,
      ...overrides,
    });

  it("既読位置が無い・先頭より新しいときは読み込み済みとみなす", () => {
    expect(covered({ lastRead: null })).toBe(true);
    expect(covered({ lastRead: 0 })).toBe(true);
    expect(covered({ oldestCreatedAt: 900, lastRead: 1_000 })).toBe(true);
  });

  it("時刻が読めないときや、これ以上古いページが無いときも読み込み済みとみなす", () => {
    expect(covered({ oldestCreatedAt: Number.NaN })).toBe(true);
    expect(covered({ hasOlder: false, oldestCreatedAt: 100 })).toBe(true);
  });

  it("上限まで読み足しても届かないときは読み込み済みとみなす", () => {
    expect(covered({ loadedPages: MAX_UNREAD_PAGES })).toBe(true);
    expect(covered({ loadedPages: MAX_UNREAD_PAGES - 1 })).toBe(false);
  });

  it("既読位置より古い行が残っていれば未読は範囲外", () => {
    expect(covered({ oldestCreatedAt: 1_100 })).toBe(false);
  });
});

describe("decideUnreadLoad", () => {
  const decide = (overrides: Partial<Parameters<typeof decideUnreadLoad>[0]> = {}) =>
    decideUnreadLoad({
      readBoundary: 1_000,
      oldestCreatedAt: 1_100,
      hasOlder: true,
      isLoadingOlder: false,
      canLoadOlder: true,
      loadedPages: 0,
      ...overrides,
    });

  it("既読位置が範囲外のときだけ次のページを読む", () => {
    expect(decide()).toBe("load");
    expect(decide({ loadedPages: MAX_UNREAD_PAGES - 1 })).toBe("load");
    expect(decide({ oldestCreatedAt: 1_000 })).toBe("none");
    expect(decide({ oldestCreatedAt: Number.NaN })).toBe("none");
  });

  it("既読位置や取得要求が無いときは読まない", () => {
    expect(decide({ readBoundary: null })).toBe("none");
    expect(decide({ readBoundary: 0 })).toBe("none");
    expect(decide({ hasOlder: false })).toBe("none");
    expect(decide({ isLoadingOlder: true })).toBe("none");
    expect(decide({ canLoadOlder: false })).toBe("none");
  });

  it("上限に達したときだけ too-old を返す", () => {
    expect(decide({ loadedPages: MAX_UNREAD_PAGES })).toBe("too-old");
    // 取得中は、上限に達していても再要求も通知もしない。
    expect(decide({ loadedPages: MAX_UNREAD_PAGES, isLoadingOlder: true })).toBe("none");
  });
});
