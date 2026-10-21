import { readFileSync } from "node:fs";
import { describe, expect, it } from "vite-plus/test";

import {
  filterKitchenEmojis,
  kitchenEmojis,
  kitchenImageUrl,
  kitchenLabel,
  kitchenParts,
  matchesKitchenQuery,
  reactionKey,
  type KitchenCatalog,
} from "./kitchen";

const catalog = JSON.parse(
  readFileSync(new URL("./emoji-kitchen.json", import.meta.url), "utf8"),
) as KitchenCatalog;

describe("Emoji Kitchen", () => {
  it("uses the published image orientation and per-pair date", () => {
    const expected =
      "https://www.gstatic.com/android/keyboard/emojikitchen/20230301/u1f600/u1f600_u1f431.png";
    expect(kitchenImageUrl(catalog, "kitchen:1f431_1f600")).toBe(expected);
    expect(kitchenImageUrl(catalog, "kitchen:😀_🐱")).toBe(expected);
    expect(kitchenImageUrl(catalog, "kitchen:1f602_1f60d")).toBe(
      "https://www.gstatic.com/android/keyboard/emojikitchen/20201001/u1f602/u1f602_u1f60d.png",
    );
  });

  it("prefixes every codepoint in multi-codepoint image paths", () => {
    expect(kitchenImageUrl(catalog, "kitchen:263a-fe0f_2615")).toBe(
      "https://www.gstatic.com/android/keyboard/emojikitchen/20230301/u263a-ufe0f/u263a-ufe0f_u2615.png",
    );
  });

  it("groups reversed and variation-selector spellings", () => {
    expect(reactionKey("kitchen:2764-fe0f_1f600")).toBe(reactionKey("kitchen:😀_❤️"));
    expect(kitchenImageUrl(catalog, "kitchen:2764-fe0f_1f600")).toBe(
      kitchenImageUrl(catalog, "kitchen:1f600_2764"),
    );
    expect(kitchenImageUrl(catalog, "kitchen:1f600_2764")).not.toBeNull();
    expect(reactionKey("👍")).toBe("👍");
  });

  it("does not invent image URLs for unavailable or malformed pairs", () => {
    expect(kitchenImageUrl(catalog, "kitchen:1f602_1f94d")).toBeNull();
    expect(kitchenImageUrl(catalog, "kitchen:1f600_10ffff")).toBeNull();
    for (const value of [
      "kitchen:1f600_",
      "kitchen:a_b_c",
      "kitchen:110000_1f600",
      "https://example.com",
      "kitchen:",
    ]) {
      expect(kitchenImageUrl(catalog, value)).toBeNull();
      expect(() => kitchenLabel(value)).not.toThrow();
    }
    expect(kitchenParts("👍")).toBeNull();
    expect(kitchenLabel("kitchen:1f600_1f431")).toBe("😀 + 🐱");
  });

  it("filters emojis by name, emoji and codepoint without case or underscore sensitivity", () => {
    const emojis = kitchenEmojis(catalog);
    expect(filterKitchenEmojis(emojis, "cat").map((emoji) => emoji.codepoint)).toEqual(["1f431"]);
    expect(filterKitchenEmojis(emojis, "CAT").map((emoji) => emoji.codepoint)).toEqual(["1f431"]);
    expect(filterKitchenEmojis(emojis, "heart eyes").map((emoji) => emoji.codepoint)).toContain(
      "1f60d",
    );
    expect(filterKitchenEmojis(emojis, "🐱").map((emoji) => emoji.codepoint)).toEqual(["1f431"]);
    expect(filterKitchenEmojis(emojis, "1f602").map((emoji) => emoji.codepoint)).toEqual(["1f602"]);
    expect(filterKitchenEmojis(emojis, "grinning cat")).toEqual([]);
    expect(filterKitchenEmojis(emojis, "  ")).toBe(emojis);
    expect(filterKitchenEmojis([], "cat")).toEqual([]);
    expect(matchesKitchenQuery(emojis[0], "")).toBe(true);
  });
});
