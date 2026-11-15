import { describe, expect, it } from "vite-plus/test";

import { decryptedMentionAllowlist, encryptedMentions, isAllowedMention } from "./mentions";

const users = [
  { id: "u-alice", user_name: "alice" },
  { id: "u-bob", user_name: "bob.smith" },
];

describe("encryptedMentions", () => {
  it("サーバーと同じ規則でユーザー、@channel、@here を拾う", () => {
    expect(encryptedMentions("hi @alice, see (@bob.smith) and @channel.", users)).toEqual({
      user_ids: ["u-alice", "u-bob"],
      channel: true,
      here: false,
    });
    expect(encryptedMentions("@HERE ok", users)).toEqual({
      user_ids: [],
      channel: false,
      here: true,
    });
  });

  it("境界のない @ や知らない名前は拾わない", () => {
    expect(encryptedMentions("mail@alice.example @unknown", users)).toEqual({
      user_ids: [],
      channel: false,
      here: false,
    });
  });
});

describe("decryptedMentionAllowlist / isAllowedMention", () => {
  const userNames = new Map([
    ["u-alice", "alice"],
    ["u-bob", "bob.smith"],
  ]);

  it("復号した mentions のユーザーと @channel だけをメンションとして扱う", () => {
    const allowlist = decryptedMentionAllowlist(
      { user_ids: ["u-alice", "u-unknown"], channel: true, here: false },
      userNames,
    );
    expect([...allowlist.userNames]).toEqual(["alice"]);
    expect(isAllowedMention("@alice", allowlist)).toBe(true);
    expect(isAllowedMention("@CHANNEL", allowlist)).toBe(true);
    // 本文に @name があっても、復号した mentions になければメンションにしない。
    expect(isAllowedMention("@bob.smith", allowlist)).toBe(false);
    expect(isAllowedMention("@here", allowlist)).toBe(false);
    expect(isAllowedMention("@unknown", allowlist)).toBe(false);
  });

  it("送信側と同じく、ユーザー名は語のまま、なければ末尾の句読点を外して引く", () => {
    const allowlist = decryptedMentionAllowlist(
      { user_ids: ["u-bob"], channel: false, here: true },
      userNames,
    );
    expect(isAllowedMention("@bob.smith", allowlist)).toBe(true);
    expect(isAllowedMention("@bob.smith.", allowlist)).toBe(true);
    expect(isAllowedMention("@here!", allowlist)).toBe(true);
  });
});
