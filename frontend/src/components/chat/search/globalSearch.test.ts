import { describe, expect, it } from "vite-plus/test";

import {
  appendSearchItems,
  normalizeSearchQuery,
  parseSearchRouteParams,
  beforeFromEndDay,
  endDayFromBefore,
  localMidnight,
  periodAfter,
  periodOf,
  searchItemKey,
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
    expect(parseSearchRouteParams({ q: "x", type: "messages" }).type).toBe("messages");
    expect(parseSearchRouteParams({ q: "x", type: "all" }).type).toBeUndefined();
  });
  it("drops dates that do not exist", () => {
    expect(parseSearchRouteParams({ q: "x", before: "2026-13-01", after: "2026-02-30" })).toEqual({
      q: "x",
    });
    expect(parseSearchRouteParams({ q: "x", after: "2024-02-29" }).after).toBe("2024-02-29");
  });
  it("keeps the filters and drops malformed dates", () => {
    expect(
      parseSearchRouteParams({
        q: "x",
        from: "u1",
        in: "c1",
        after: "2026-11-01",
        before: "11/30",
      }),
    ).toEqual({ q: "x", from: "u1", in: "c1", after: "2026-11-01" });
  });
  it("restores numeric queries the router parsed as JSON", () => {
    expect(parseSearchRouteParams({ q: 2026 }).q).toBe("2026");
    expect(parseSearchRouteParams({ q: { nested: true } }).q).toBeUndefined();
  });
  it("defaults the kind to messages and files together", () => {
    expect(searchKindOf({})).toBe("all");
    expect(searchKindOf({ type: "files" })).toBe("files");
  });
});

describe("end day", () => {
  it("includes the chosen end day by sending the next day as before", () => {
    expect(beforeFromEndDay("2026-11-30")).toBe("2026-12-01");
    expect(endDayFromBefore("2026-12-01")).toBe("2026-11-30");
    expect(beforeFromEndDay(undefined)).toBeUndefined();
  });
});

describe("searchItemKey", () => {
  it("keeps a message and an attachment with the same id apart", () => {
    expect(searchItemKey(message)).not.toBe(searchItemKey({ ...file, id: message.id }));
  });
});

/** 端末のタイムゾーンを一時的に替える（Node は `TZ` の変更をすぐ反映する）。 */
function inTimeZone(zone: string, run: () => void) {
  const original = process.env.TZ;
  process.env.TZ = zone;
  try {
    run();
  } finally {
    if (original === undefined) delete process.env.TZ;
    else process.env.TZ = original;
  }
}

describe("periods", () => {
  // 端末の暦の 11/21 正午（UTC+12 以降の端末でも日付がずれない）。
  const now = new Date(2026, 10, 21, 12);
  it("uses the terminal's calendar day", () => {
    // JST では 11/22 の 8 時なので、今日は 11/22。
    inTimeZone("Asia/Tokyo", () =>
      expect(periodAfter("last7", new Date("2026-11-21T23:00:00Z"))).toBe("2026-11-16"),
    );
    inTimeZone("UTC", () =>
      expect(periodAfter("last7", new Date("2026-11-21T23:00:00Z"))).toBe("2026-11-15"),
    );
  });
  it("sends the terminal's midnight with its offset", () => {
    inTimeZone("Asia/Tokyo", () =>
      expect(localMidnight("2026-11-22")).toBe("2026-11-22T00:00:00+09:00"),
    );
    // 変換済みの時刻は、タイムゾーンが変わってもそのまま。
    inTimeZone("UTC", () =>
      expect(localMidnight("2026-11-22T00:00:00+09:00")).toBe("2026-11-22T00:00:00+09:00"),
    );
    inTimeZone("America/New_York", () => {
      expect(localMidnight("2026-11-22")).toBe("2026-11-22T00:00:00-05:00");
      expect(localMidnight("2026-07-01")).toBe("2026-07-01T00:00:00-04:00");
    });
    // 0 時がない日（サンパウロの 2018-11-04 は 0 時に 1 時へ進んだ）は、その日の最初の時刻。
    inTimeZone("America/Sao_Paulo", () =>
      expect(localMidnight("2018-11-04")).toBe("2018-11-04T01:00:00-02:00"),
    );
    inTimeZone("UTC", () => expect(localMidnight("0002-01-01")).toBe("0002-01-01T00:00:00+00:00"));
  });
  it("counts today as the first of the last N days", () => {
    expect(periodAfter("last7", now)).toBe("2026-11-15");
    expect(periodOf({ after: "2026-11-15" }, now)).toBe("last7");
    expect(periodOf({}, now)).toBe("anytime");
    expect(periodOf({ after: "2026-11-15", before: "2026-11-20" }, now)).toBe("custom");
  });
});

describe("searchRequestPath", () => {
  it("sends the filters as the API parameters", () => {
    inTimeZone("Asia/Tokyo", () =>
      expect(
        searchRequestPath("会議", "all", null, {
          from: "u1",
          in: "c1",
          after: "2026-11-01",
          before: "2026-11-22",
        }),
      ).toBe(
        "/api/search?q=%E4%BC%9A%E8%AD%B0&type=all&user_id=u1&channel_id=c1&after=2026-11-01T00%3A00%3A00%2B09%3A00&before=2026-11-22T00%3A00%3A00%2B09%3A00",
      ),
    );
  });
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
