import { describe, expect, it } from "vite-plus/test";

import { rankCustomEmojis } from "./EmojiPicker";

const e = (name: string, aliases: string[] = [], category = "") => ({
  name,
  url: "",
  category,
  aliases,
});

describe("rankCustomEmojis", () => {
  it("puts exact name matches first, then prefix, then substring, shorter first", () => {
    const options = [
      e("kusa_hayasu"),
      e("big_kusa"),
      e("kusa"),
      e("kusaa"),
      e("w", ["kusa"]),
      e("other"),
    ];
    expect(rankCustomEmojis(options, "kusa").map((x) => x.name)).toEqual([
      "kusa",
      "kusaa",
      "kusa_hayasu",
      "big_kusa",
      "w",
    ]);
  });

  it("returns every emoji in registration order for an empty query", () => {
    const options = [e("longer_name"), e("a"), e("mid")];
    expect(rankCustomEmojis(options, "").map((x) => x.name)).toEqual(["longer_name", "a", "mid"]);
  });

  it("ranks alias and category matches equally, so shorter names come first", () => {
    const options = [e("longname", ["x"]), e("a", [], "x")];
    expect(rankCustomEmojis(options, "x").map((x) => x.name)).toEqual(["a", "longname"]);
  });

  it("keeps the original order for equal rank and length", () => {
    expect(rankCustomEmojis([e("ab1"), e("ab2")], "ab").map((x) => x.name)).toEqual(["ab1", "ab2"]);
  });
});
