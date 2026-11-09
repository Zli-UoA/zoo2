import { describe, expect, it } from "vite-plus/test";
import { insertMention, mentionOptions, mentionQuery } from "./mentions";
import { messageTextSegments } from "./messageFormat";
import type { UserRow } from "../../../lib/electric/types";

const user = (id: string, name: string, display = name): UserRow => ({
  id,
  user_name: name,
  display_name: display,
  created_at: "2026-10-24T00:00:00Z",
  updated_at: null,
});

describe("mentions", () => {
  it("matches backend boundaries and excludes email addresses and URLs", () => {
    const content =
      "こんにちは。@bob、(@alice) @channel! @HERE. foo@example.com https://example.com/@here";
    expect(
      messageTextSegments(content)
        .filter((part) => part.kind === "mention")
        .map((part) => part.value),
    ).toEqual(["@bob", "@alice", "@channel", "@HERE"]);
    expect(
      messageTextSegments(content)
        .map((part) => part.value)
        .join(""),
    ).toBe(content);
    expect(messageTextSegments(content).filter((part) => part.kind === "link")).toEqual([
      { kind: "link", value: "https://example.com/@here" },
    ]);
  });
  it("offers only channel members and special mentions, with display name search", () => {
    const users = [user("a", "alice", "アリス"), user("b", "bob"), user("c", "outsider")];
    const ids = new Set(["a", "b"]);
    expect(mentionOptions(users, ids, "").map((option) => option.name)).toEqual([
      "channel",
      "here",
      "alice",
      "bob",
    ]);
    expect(mentionOptions(users, ids, "アリ")).toEqual([{ name: "alice", label: "アリス" }]);
    expect(mentionOptions(users, ids, "BO")[0]?.name).toBe("bob");
  });
  it("leaves deactivated users such as imported Slack placeholders out of the candidates", () => {
    const placeholder = { ...user("u3", "alice_slack", "Alice (Slack)"), is_deactivated: true };
    const users = [user("u1", "alice"), placeholder];
    expect(
      mentionOptions(users, new Set(["u1", "u3"]), "alice").map((option) => option.name),
    ).toEqual(["alice"]);
  });
  it("replaces only the token at the caret and preserves following text", () => {
    const text = "Hi (@al) next";
    const range = mentionQuery(text, 7)!;
    expect(range).toEqual({ start: 4, end: 7, query: "al" });
    expect(insertMention(text, range, "alice")).toEqual({ value: "Hi (@alice ) next", caret: 11 });
    expect(mentionQuery("email@example", 13)).toBeNull();
    expect(mentionQuery("https://site/@bob", 17)).toBeNull();
    expect(mentionQuery("@", 1)?.query).toBe("");
  });
});

it("accepts Japanese display-name queries only after an ASCII @", () => {
  expect(mentionQuery("hi @太", 5)).toEqual({ start: 3, end: 5, query: "太" });
  expect(mentionQuery("＠太", 2)).toBeNull();
});
