import { describe, expect, it } from "vite-plus/test";

import {
  authorLabel,
  formatClock,
  formatRelativeDay,
  formatTime,
  isSameLocalDay,
  localDayKey,
  messageTextSegments,
  newMessageAnnouncement,
  type FeedMessage,
  type MessageTextSegment,
} from "./messageFormat";

import { createI18n } from "../../../lib/i18n";

const t = createI18n().t;

function message(overrides: Partial<FeedMessage> = {}): FeedMessage {
  return {
    id: "message-1",
    user_id: "user-1",
    content: "hello",
    created_at: "2026-01-02 00:00:00.000",
    parent_id: null,
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
    expect(formatRelativeDay(new Date(2026, 5, 15, 9, 0), now, t, "ja")).toBe("今日");
    expect(formatRelativeDay(new Date(2026, 5, 14, 9, 0), now, t, "ja")).toBe("昨日");
  });

  it("formats older days with the weekday", () => {
    expect(formatRelativeDay(new Date(2026, 5, 10), now, t, "ja")).toBe("2026年6月10日(水)");
  });
});

function text(value: string): MessageTextSegment {
  return { kind: "text", value };
}

function link(value: string): MessageTextSegment {
  return { kind: "link", value };
}

describe("messageTextSegments", () => {
  it("returns plain content as a single text segment", () => {
    expect(messageTextSegments("こんにちは\n改行もそのまま")).toEqual([
      text("こんにちは\n改行もそのまま"),
    ]);
    expect(messageTextSegments("")).toEqual([text("")]);
  });

  it("splits a URL out of the surrounding text", () => {
    expect(messageTextSegments("see https://example.com/a?b=1#c now")).toEqual([
      text("see "),
      link("https://example.com/a?b=1#c"),
      text(" now"),
    ]);
  });

  it("linkifies http and https, but not other schemes", () => {
    expect(messageTextSegments("http://example.com")).toEqual([link("http://example.com")]);
    expect(messageTextSegments("javascript:alert(1)")).toEqual([text("javascript:alert(1)")]);
    expect(messageTextSegments("foohttps://example.com")).toEqual([text("foohttps://example.com")]);
  });

  it("keeps trailing sentence punctuation out of the link", () => {
    expect(messageTextSegments("詳細は https://example.com/docs。")).toEqual([
      text("詳細は "),
      link("https://example.com/docs"),
      text("。"),
    ]);
    expect(messageTextSegments("https://example.com/a, and more")).toEqual([
      link("https://example.com/a"),
      text(", and more"),
    ]);
  });

  it("keeps balanced parentheses in the URL", () => {
    expect(messageTextSegments("https://example.com/wiki/Foo_(bar)")).toEqual([
      link("https://example.com/wiki/Foo_(bar)"),
    ]);
    expect(messageTextSegments("(https://example.com/a)")).toEqual([
      text("("),
      link("https://example.com/a"),
      text(")"),
    ]);
    expect(messageTextSegments("（https://example.com/a）")).toEqual([
      text("（"),
      link("https://example.com/a"),
      text("）"),
    ]);
  });

  it("handles multiple URLs", () => {
    expect(messageTextSegments("https://a.example と https://b.example")).toEqual([
      link("https://a.example"),
      text(" と "),
      link("https://b.example"),
    ]);
  });

  it("stops the URL at whitespace and angle brackets", () => {
    expect(messageTextSegments("https://example.com/a,b")).toEqual([
      link("https://example.com/a,b"),
    ]);
    expect(messageTextSegments("<https://example.com>")).toEqual([
      text("<"),
      link("https://example.com"),
      text(">"),
    ]);
  });

  it("requires a host part after the scheme", () => {
    expect(messageTextSegments("https://.")).toEqual([text("https://.")]);
    expect(messageTextSegments("https://?q")).toEqual([text("https://?q")]);
    expect(messageTextSegments("https:///path")).toEqual([text("https:///path")]);
    expect(messageTextSegments("https://)")).toEqual([text("https://)")]);
    expect(messageTextSegments("詳細: https://。")).toEqual([text("詳細: https://。")]);
    // ホスト部があるものは今までどおりリンクにする。
    expect(messageTextSegments("https://[::1]:3000/a")).toEqual([link("https://[::1]:3000/a")]);
    expect(messageTextSegments("https://localhost:3000/")).toEqual([
      link("https://localhost:3000/"),
    ]);
  });
});

describe("newMessageAnnouncement", () => {
  it("summarizes the author and the collapsed content", () => {
    expect(newMessageAnnouncement(message({ content: " line1\n\nline2 " }), t)).toBe(
      "新着メッセージ: Alice「line1 line2」",
    );
  });

  it("truncates long content by code points", () => {
    const announcement = newMessageAnnouncement(message({ content: "あ".repeat(10) }), t, 4);
    expect(announcement).toBe("新着メッセージ: Alice「ああああ…」");
  });

  it("falls back to the author only when there is no content", () => {
    expect(newMessageAnnouncement(message({ content: "   " }), t)).toBe("新着メッセージ: Alice");
  });
});

describe("localized dates", () => {
  const now = new Date(2026, 5, 15, 20);
  it("localizes relative labels and older dates", () => {
    expect(formatRelativeDay(now, now, createI18n("en").t, "en")).toBe("Today");
    expect(formatRelativeDay(new Date(2026, 5, 14), now, createI18n("yue").t, "yue")).toBe("尋日");
    expect(formatRelativeDay(new Date(2026, 5, 10), now, createI18n("en").t, "en")).toBe(
      "Wed, June 10, 2026",
    );
    expect(formatRelativeDay(new Date(2026, 5, 10), now, createI18n("zh-Hans").t, "zh-Hans")).toBe(
      "2026年6月10日周三",
    );
  });
});

it("localizes new message announcements without translating user content", () => {
  expect(newMessageAnnouncement(message({ content: "hello" }), createI18n("en").t)).toBe(
    "New message from Alice: “hello”",
  );
  expect(newMessageAnnouncement(message({ content: "" }), createI18n("yue").t)).toBe(
    "Alice 傳咗新訊息",
  );
});
