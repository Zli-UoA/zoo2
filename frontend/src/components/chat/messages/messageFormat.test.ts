import { describe, expect, it } from "vite-plus/test";

import {
  authorLabel,
  formatClock,
  formatRelativeDay,
  formatTime,
  isSameLocalDay,
  localDayKey,
  type FeedMessage,
} from "./messageFormat";

function message(overrides: Partial<FeedMessage> = {}): FeedMessage {
  return {
    id: "message-1",
    user_id: "user-1",
    content: "hello",
    created_at: "2026-01-02 00:00:00.000",
    user_name: "alice",
    display_name: "Alice",
    ...overrides,
  };
}

describe("authorLabel", () => {
  it("prefers display name, then user name, then id", () => {
    expect(authorLabel(message())).toBe("Alice");
    expect(authorLabel(message({ display_name: null }))).toBe("alice");
    expect(authorLabel(message({ display_name: null, user_name: null }))).toBe("user-1");
  });
});

describe("formatClock", () => {
  it("zero-pads hours and minutes", () => {
    expect(formatClock(new Date(2026, 0, 2, 3, 4))).toBe("03:04");
    expect(formatClock(new Date(2026, 0, 2, 23, 59))).toBe("23:59");
  });
});

describe("formatTime", () => {
  it("renders a HH:mm clock for a naive UTC timestamp", () => {
    expect(formatTime("2026-01-02 03:04:05.000")).toMatch(/^\d{2}:\d{2}$/);
  });
});

describe("day helpers", () => {
  it("compares local calendar days", () => {
    expect(isSameLocalDay(new Date(2026, 5, 15, 1), new Date(2026, 5, 15, 23))).toBe(true);
    expect(isSameLocalDay(new Date(2026, 5, 15), new Date(2026, 5, 16))).toBe(false);
    expect(localDayKey(new Date(2026, 5, 5))).toBe("2026-06-05");
  });
});

describe("formatRelativeDay", () => {
  const now = new Date(2026, 5, 15, 20, 0);

  it("labels today and yesterday", () => {
    expect(formatRelativeDay(new Date(2026, 5, 15, 9, 0), now)).toBe("今日");
    expect(formatRelativeDay(new Date(2026, 5, 14, 9, 0), now)).toBe("昨日");
  });

  it("formats older days with the weekday", () => {
    expect(formatRelativeDay(new Date(2026, 5, 10), now)).toBe("2026年6月10日(水)");
  });
});

describe("localized dates", () => {
  const now = new Date(2026, 5, 15, 20);
  it("localizes relative labels and older dates", () => {
    expect(formatRelativeDay(now, now, "en")).toBe("Today");
    expect(formatRelativeDay(new Date(2026, 5, 14), now, "yue")).toBe("尋日");
    expect(formatRelativeDay(new Date(2026, 5, 10), now, "en")).toBe("Wed, June 10, 2026");
    expect(formatRelativeDay(new Date(2026, 5, 10), now, "zh-Hans")).toBe("2026年6月10日周三");
  });
});
