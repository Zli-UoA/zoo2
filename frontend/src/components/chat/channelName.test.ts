import { describe, expect, it } from "vite-plus/test";

import { MAX_CHANNEL_NAME_LENGTH, validateChannelName } from "./channelName";

describe("channel name validation", () => {
  it.each(["new-channel", "a_b", "ざつだん", "General", "équipe", "123", "あ".repeat(80)])(
    "accepts %s",
    (name) => {
      expect(validateChannelName(name)).toBeNull();
    },
  );

  it.each(["", "   ", "\u3000"])("rejects blank name %j", (name) => {
    expect(validateChannelName(name)).toBe("empty");
  });

  it("counts code points like the backend", () => {
    expect(validateChannelName("あ".repeat(MAX_CHANNEL_NAME_LENGTH))).toBeNull();
    expect(validateChannelName("あ".repeat(MAX_CHANNEL_NAME_LENGTH + 1))).toBe("tooLong");
  });

  it.each(["all", "channel", "everyone", "here", "general"])("rejects reserved name %s", (name) => {
    expect(validateChannelName(name)).toBe("reserved");
  });

  it("keeps reserved names case-sensitive", () => {
    expect(validateChannelName("General")).toBeNull();
    expect(validateChannelName("ALL")).toBeNull();
  });

  it.each(["Invalid Name", "invalid!", "名前 です", "emoji😀", "a/b", "a.b"])(
    "rejects unsupported characters in %s",
    (name) => {
      expect(validateChannelName(name)).toBe("invalidCharacters");
    },
  );
});
