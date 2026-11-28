import { describe, expect, it } from "vite-plus/test";
import { recentReactionValue } from "./recents";
describe("recent emojis", () => {
  it("keeps the image ID with an image-based reaction", () => {
    const hash = "a".repeat(64);
    expect(recentReactionValue("party_parrot")).toBe("party_parrot");
    // ReactionEmoji の形式は apub:<image_id>:<:name:> なので区切りと名前にコロンが並ぶ。
    expect(recentReactionValue("party_parrot", hash)).toBe(`apub:${hash}::party_parrot:`);
    expect(recentReactionValue(":party_parrot:", hash)).toBe(`apub:${hash}::party_parrot:`);
    expect(recentReactionValue("party_parrot", hash, true)).toBe(":party_parrot:");
  });
});
