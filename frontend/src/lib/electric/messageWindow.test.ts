import { describe, expect, it } from "vite-plus/test";

import {
  MESSAGE_WINDOW_MS,
  expandWindowSince,
  formatWindowTimestamp,
  messageWindowStart,
  olderWindowSince,
} from "./messageWindow";

const NOW = Date.UTC(2026, 9, 27, 12, 0, 0);

describe("messageWindowStart", () => {
  it("returns the Electric timestamp format for 30 days before now", () => {
    expect(messageWindowStart(NOW)).toBe("2026-09-27 12:00:00.000");
    expect(NOW - MESSAGE_WINDOW_MS).toBe(Date.UTC(2026, 8, 27, 12, 0, 0));
  });
});

describe("expandWindowSince", () => {
  it("moves since to one millisecond before the oldest message", () => {
    expect(expandWindowSince("2026-10-01 00:00:00.000", "2026-09-20 12:00:00.000")).toBe(
      "2026-09-20 11:59:59.999",
    );
  });

  it("keeps the current window when it already covers the oldest message", () => {
    const since = "2026-10-01 00:00:00.000";
    expect(expandWindowSince(since, "2026-10-02 00:00:00.000")).toBe(since);
    expect(expandWindowSince(since, since)).toBe(since);
  });

  it("handles an invalid oldest timestamp without shrinking the window", () => {
    const since = "2026-10-01 00:00:00.000";
    expect(expandWindowSince(since, "not-a-timestamp")).toBe(since);
  });

  it("compares ISO timestamps with Z against Electric timestamps", () => {
    expect(expandWindowSince("2026-10-01T00:00:00.000Z", "2026-09-15T00:00:00.000Z")).toBe(
      "2026-09-14 23:59:59.999",
    );
  });
});

describe("formatWindowTimestamp", () => {
  it("always emits millisecond precision in UTC", () => {
    expect(formatWindowTimestamp(Date.UTC(2026, 0, 2, 3, 4, 5))).toBe("2026-01-02 03:04:05.000");
  });
});

describe("olderWindowSince", () => {
  const since = "2026-10-12 00:00:00.000";
  // 窓の外 (since より前) の行: 10/11 から 1 日ずつ古くなる
  const day = (n: number) => `2026-10-${String(n).padStart(2, "0")} 09:00:00.000`;
  const outside = [11, 10, 9, 8, 7, 6, 5, 4, 3, 2].map(day);

  it("widens by one page of older rows, not by the oldest displayed row", () => {
    // 窓の中の行だけでは広がらない (従来の不具合)
    expect(olderWindowSince(since, ["2026-10-12 01:00:00.000", "2026-11-01 00:00:00.000"])).toBe(
      since,
    );
    // 古い方へ 3 件分 (10/11・10/10・10/9) 広げ、3 件目の 1 ミリ秒前まで
    expect(olderWindowSince(since, outside, 3)).toBe("2026-10-09 08:59:59.999");
  });

  it("goes to the oldest row when fewer older rows remain than a page", () => {
    expect(olderWindowSince(since, outside, 100)).toBe("2026-10-02 08:59:59.999");
  });

  it("returns the same window when nothing is older", () => {
    expect(olderWindowSince(since, [])).toBe(since);
    expect(olderWindowSince(since, ["2026-10-12 00:00:00.000", "2026-11-01 00:00:00.000"])).toBe(
      since,
    );
  });

  it("ignores invalid timestamps and accepts both timestamp formats", () => {
    expect(
      olderWindowSince(since, ["not-a-timestamp", "2026-10-05T09:00:00.000Z", day(9)], 5),
    ).toBe("2026-10-05 08:59:59.999");
  });

  it("keeps widening page after page until the history is exhausted", () => {
    let current = since;
    const seen: string[] = [];
    for (let i = 0; i < 6; i++) {
      const next = olderWindowSince(current, outside, 4);
      seen.push(next);
      if (next === current) break;
      current = next;
    }
    // 4 件ずつ: 10/8 → 10/4 → 10/2 (残りの 2 件で終わり) → 10/2 のまま
    expect(seen).toEqual([
      "2026-10-08 08:59:59.999",
      "2026-10-04 08:59:59.999",
      "2026-10-02 08:59:59.999",
      "2026-10-02 08:59:59.999",
    ]);
  });
});
