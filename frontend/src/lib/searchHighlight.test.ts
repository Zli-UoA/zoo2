import { describe, expect, it } from "vite-plus/test";

import { highlightSegments, searchTerms } from "./searchHighlight";

describe("searchTerms", () => {
  it("splits on whitespace like the server, without duplicates, up to 5 terms", () => {
    expect(searchTerms("  検索　テスト 検索\tfoo ")).toEqual(["検索", "テスト", "foo"]);
    expect(searchTerms("a b c d e f g")).toEqual(["a", "b", "c", "d", "e"]);
    expect(searchTerms("   ")).toEqual([]);
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
