import { describe, expect, it } from "vite-plus/test";

import { encryptedMentions } from "./mentions";

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
