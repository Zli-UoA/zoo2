import { describe, expect, it } from "vite-plus/test";
import { composerQuery, insertSuggestion } from "./composerSuggestions";

describe("composer completion", () => {
  it("finds channel references in text and Unicode prefixes", () => {
    expect(composerQuery("こんにちは #開発", 9, "channel")).toEqual({
      start: 6,
      end: 9,
      query: "開発",
    });
    expect(composerQuery("(#GEN", 5, "channel")?.query).toBe("gen");
    expect(composerQuery("#", 1, "channel")?.query).toBe("");
  });
  it.each([
    "# Heading",
    "## Heading",
    "text#general",
    "https://example.com/#general",
    "`#general",
    "```md\n#general",
    "\\#general",
  ])("does not complete protected/literal hash: %s", (value) => {
    expect(composerQuery(value, value.length, "channel")).toBeNull();
  });
  it("only completes slash commands at the start of a message, before arguments", () => {
    expect(composerQuery("/", 1, "command")?.query).toBe("");
    expect(composerQuery("/GiT", 4, "command")?.query).toBe("git");
    for (const value of [
      "text /echo",
      "/echo argument",
      "https://example.com/echo",
      "`/echo",
      "line\n/echo",
    ]) {
      expect(composerQuery(value, value.length, "command")).toBeNull();
    }
  });
  it("replaces the token at the caret and preserves suffix text and caret position", () => {
    const value = "See (#genral) next";
    expect(insertSuggestion(value, composerQuery(value, 9, "channel")!, "#general")).toEqual({
      value: "See (#general ) next",
      caret: 14,
    });
    expect(insertSuggestion("/ec args", composerQuery("/ec args", 3, "command")!, "/echo")).toEqual(
      { value: "/echo args", caret: 6 },
    );
  });
});
