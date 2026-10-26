import { describe, expect, it } from "vite-plus/test";

import {
  MESSAGE_WINDOW_MS,
  expandWindowSince,
  formatWindowTimestamp,
  messageWindowStart,
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
