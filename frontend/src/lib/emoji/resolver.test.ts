import { describe, expect, it, vi } from "vitest";

const data = (locale: string, label: string, tags: string[]) => ({
  locale,
  emojis: [{ emoji: "😀", category: 0, label, version: 1, tags }],
  categories: [],
  skinTones: {},
});

vi.mock("frimousse", () => ({
  defaultEmojiDataResolver: vi.fn(async (locale: string) =>
    locale === "ja" ? data("ja", "にっこり", ["笑顔"]) : data("en", "grinning face", ["smile"]),
  ),
}));

import { resolveMultilingualEmojiData } from "./resolver";

describe("resolveMultilingualEmojiData", () => {
  it("adds English label and tags to localized emojis", async () => {
    const result = await resolveMultilingualEmojiData("ja", {});
    expect(result.emojis[0]?.label).toBe("にっこり");
    expect(result.emojis[0]?.tags).toEqual(["笑顔", "grinning face", "smile"]);
  });

  it("leaves English data untouched", async () => {
    const result = await resolveMultilingualEmojiData("en", {});
    expect(result.emojis[0]?.tags).toEqual(["smile"]);
  });
});
