import { describe, expect, it } from "vite-plus/test";

import {
  appendSearchItems,
  normalizeSearchQuery,
  parseSearchRouteParams,
  searchKindOf,
  searchRequestPath,
  searchResultLink,
  type SearchFileItem,
  type SearchMessageItem,
} from "./globalSearch";

const message: SearchMessageItem = {
  id: "m1",
  channel_id: "c1",
  channel_name: "general",
  is_dm: false,
  parent_id: null,
  user_id: "u1",
  author: null,
  snippet: "会議の資料",
  created_at: "2026-11-21T01:02:03.456",
};
const file: SearchFileItem = {
  ...message,
  id: "a1",
  message_id: "m9",
  file_name: "資料.pdf",
  content_type: "application/pdf",
  file_size: 1024,
  width: null,
  height: null,
  is_sensitive: false,
};

describe("parseSearchRouteParams", () => {
  it("keeps only the supported kinds and omits defaults", () => {
    expect(parseSearchRouteParams({})).toEqual({ q: undefined, type: undefined });
    expect(parseSearchRouteParams({ q: "  会議   資料 ", type: "files" })).toEqual({
      q: "会議 資料",
      type: "files",
    });
    expect(parseSearchRouteParams({ q: "x", type: "notes" })).toEqual({ q: "x", type: undefined });
    expect(parseSearchRouteParams({ q: "x", type: "messages" }).type).toBeUndefined();
  });
  it("restores numeric queries the router parsed as JSON", () => {
    expect(parseSearchRouteParams({ q: 2026 }).q).toBe("2026");
    expect(parseSearchRouteParams({ q: { nested: true } }).q).toBeUndefined();
  });
  it("defaults the kind to messages", () => {
    expect(searchKindOf({})).toBe("messages");
    expect(searchKindOf({ type: "files" })).toBe("files");
  });
});

describe("searchRequestPath", () => {
  it("encodes the query, kind and cursor", () => {
    expect(searchRequestPath("a&b %", "messages", null)).toBe(
      "/api/search?q=a%26b+%25&type=messages",
    );
    const cursor = '{"m":24323,"at":null,"id":null}';
    const url = new URL(searchRequestPath("会議", "files", cursor), "https://zoo.test");
    expect(url.searchParams.get("q")).toBe("会議");
    expect(url.searchParams.get("type")).toBe("files");
    expect(url.searchParams.get("cursor")).toBe(cursor);
  });
  it("normalizes whitespace like the server", () => {
    expect(normalizeSearchQuery("\t会議　 資料\n")).toBe("会議 資料");
  });
});

describe("searchResultLink", () => {
  it("opens top-level messages directly", () => {
    expect(searchResultLink(message)).toEqual({ channelId: "c1", search: { message: "m1" } });
  });
  it("opens the thread for replies and focuses the reply", () => {
    expect(searchResultLink({ ...message, parent_id: "root" })).toEqual({
      channelId: "c1",
      search: { thread: "root", message: "m1" },
    });
  });
  it("opens the message that holds a file", () => {
    expect(searchResultLink(file)).toEqual({ channelId: "c1", search: { message: "m9" } });
    expect(searchResultLink({ ...file, parent_id: "root" }).search).toEqual({
      thread: "root",
      message: "m9",
    });
  });
});

it("appends pages without duplicating overlapping items", () => {
  const second = { ...message, id: "m2" };
  expect(appendSearchItems([message], [message, second]).map((item) => item.id)).toEqual([
    "m1",
    "m2",
  ]);
});
