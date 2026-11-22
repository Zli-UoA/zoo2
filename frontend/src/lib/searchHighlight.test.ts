import { describe, expect, it } from "vite-plus/test";

import {
  focusSnippet,
  highlightSegments,
  searchQueryProblem,
  searchTerms,
} from "./searchHighlight";

describe("searchTerms", () => {
  it("splits on whitespace like the server, without duplicates, up to 5 terms", () => {
    expect(searchTerms("  検索　テスト 検索\tfoo ")).toEqual(["検索", "テスト", "foo"]);
    expect(searchTerms("a b c d e f g")).toEqual(["a", "b", "c", "d", "e"]);
    expect(searchTerms("   ")).toEqual([]);
  });
});

describe("searchQueryProblem", () => {
  it("rejects what the server's parse_terms rejects", () => {
    expect(searchQueryProblem("a b c d e")).toBeNull();
    expect(searchQueryProblem("a b c d e a")).toBeNull();
    expect(searchQueryProblem("how to set up dev server")).toBe("tooManyTerms");
    expect(searchQueryProblem("x".repeat(100))).toBeNull();
    expect(searchQueryProblem(`https://example.com/${"x".repeat(81)}`)).toBe("termTooLong");
    // 文字数はコードポイントで数える（サーバーの chars().count() と同じ）。
    expect(searchQueryProblem("🐘".repeat(100))).toBeNull();
    expect(searchQueryProblem("")).toBeNull();
  });
});

describe("highlightSegments", () => {
  it("marks every hit case-insensitively", () => {
    expect(highlightSegments("Rust と rust と RUST", ["rust"])).toEqual([
      { text: "Rust", match: true },
      { text: " と ", match: false },
      { text: "rust", match: true },
      { text: " と ", match: false },
      { text: "RUST", match: true },
    ]);
  });

  it("prefers the longer term where terms overlap", () => {
    expect(highlightSegments("全文検索の検索", ["検索", "全文検索"])).toEqual([
      { text: "全文検索", match: true },
      { text: "の", match: false },
      { text: "検索", match: true },
    ]);
  });

  it("treats regular expression characters literally", () => {
    expect(highlightSegments("a.b axb (c)", ["a.b", "(c)"])).toEqual([
      { text: "a.b", match: true },
      { text: " axb ", match: false },
      { text: "(c)", match: true },
    ]);
  });

  it("returns the text unchanged without terms or hits", () => {
    expect(highlightSegments("本文", [])).toEqual([{ text: "本文", match: false }]);
    expect(highlightSegments("本文", ["なし"])).toEqual([{ text: "本文", match: false }]);
  });

  it("keeps surrogate pairs whole", () => {
    expect(highlightSegments("🐘と🐘", ["🐘"])).toEqual([
      { text: "🐘", match: true },
      { text: "と", match: false },
      { text: "🐘", match: true },
    ]);
  });
});

describe("focusSnippet", () => {
  it("shortens what comes before the first hit", () => {
    const before = "あ".repeat(40);
    expect(focusSnippet(`…${before}会議の資料`, ["会議"], 4)).toBe("…ああああ会議の資料");
    // 短ければ変えない。一致がなくても変えない。
    expect(focusSnippet("明日の会議", ["会議"], 4)).toBe("明日の会議");
    expect(focusSnippet(`${before}資料`, ["会議"], 4)).toBe(`${before}資料`);
    expect(focusSnippet(`${before}資料`, [], 4)).toBe(`${before}資料`);
  });
});
