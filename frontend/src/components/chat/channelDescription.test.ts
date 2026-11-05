import { describe, expect, it } from "vite-plus/test";

import {
  channelDescriptionLength,
  isChannelDescriptionTooLong,
  MAX_CHANNEL_DESCRIPTION_LENGTH,
  normalizeChannelDescription,
} from "./channelDescription";

describe("channel description helpers", () => {
  it("counts code points like the backend", () => {
    expect(channelDescriptionLength("あいう")).toBe(3);
    expect(channelDescriptionLength("𠮷野家")).toBe(3);
  });

  it("allows exactly 250 characters and rejects more", () => {
    expect(isChannelDescriptionTooLong("あ".repeat(MAX_CHANNEL_DESCRIPTION_LENGTH))).toBe(false);
    expect(isChannelDescriptionTooLong("あ".repeat(MAX_CHANNEL_DESCRIPTION_LENGTH + 1))).toBe(true);
  });

  it("trims descriptions and treats null as empty", () => {
    expect(normalizeChannelDescription("  説明  ")).toBe("説明");
    expect(normalizeChannelDescription(null)).toBe("");
    expect(normalizeChannelDescription(undefined)).toBe("");
  });
});
