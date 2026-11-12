import { afterEach, describe, expect, it, vi } from "vite-plus/test";

vi.mock("frimousse", () => ({
  defaultEmojiDataResolver: vi.fn(async (locale: string) => ({
    locale,
    emojis: [{ emoji: "😀", category: 0, label: "にっこり", version: 1, tags: ["笑顔"] }],
    categories: [],
    skinTones: {},
  })),
}));

const index = { "😀": ["にっこり", "笑顔", "smile", "웃음", "xiao lian", "xiaolian"] };

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("resolveMultilingualEmojiData", () => {
  it("adds keywords from other languages and pinyin without duplicating its own", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => Response.json(index)),
    );
    const { resolveMultilingualEmojiData: resolve } = await import("./resolver");
    const result = await resolve("ja", { emojibaseUrl: "/emojibase-data" });
    expect(result.emojis[0]?.label).toBe("にっこり");
    expect(result.emojis[0]?.tags).toEqual(["笑顔", "smile", "웃음", "xiao lian", "xiaolian"]);
  });

  it("falls back to the localized data when the index cannot be loaded", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("", { status: 404 })),
    );
    const { resolveMultilingualEmojiData: resolve } = await import("./resolver");
    const result = await resolve("ja", { emojibaseUrl: "/emojibase-data" });
    expect(result.emojis[0]?.tags).toEqual(["笑顔"]);
  });
});
