import { describe, expect, it } from "vite-plus/test";

import type { ThreadParticipant } from "../../../hooks/useThreadMembership";
import { filterThreadParticipants, sortThreadParticipants } from "./threadParticipants";

function participant(id: string, user_name: string, display_name: string | null = null) {
  return { id, user_name, display_name, avatar_url: null } satisfies ThreadParticipant;
}

const people = [
  participant("u3", "carol", "Carol"),
  participant("u1", "alice", "Alice"),
  participant("u2", "bob"),
  participant("u4", "alice2", "Alice"),
];

describe("sortThreadParticipants", () => {
  it("puts the author of the parent message first, then sorts by label", () => {
    const sorted = sortThreadParticipants(people, "u3");
    expect(sorted.map((p) => p.id)).toEqual(["u3", "u1", "u4", "u2"]);
  });

  it("sorts by label when the author is not among the loaded participants", () => {
    expect(sortThreadParticipants(people, "missing").map((p) => p.id)).toEqual([
      "u1",
      "u4",
      "u2",
      "u3",
    ]);
    expect(sortThreadParticipants(people, null).map((p) => p.id)).toEqual(["u1", "u4", "u2", "u3"]);
  });

  it("does not mutate its input", () => {
    const before = people.map((p) => p.id);
    sortThreadParticipants(people, "u3");
    expect(people.map((p) => p.id)).toEqual(before);
  });
});

describe("filterThreadParticipants", () => {
  it("matches the display name or the user name, ignoring case", () => {
    expect(filterThreadParticipants(people, "ALI").map((p) => p.id)).toEqual(["u1", "u4"]);
    expect(filterThreadParticipants(people, "bo").map((p) => p.id)).toEqual(["u2"]);
    expect(filterThreadParticipants(people, "alice2").map((p) => p.id)).toEqual(["u4"]);
  });

  it("returns everyone for a blank query and nobody for no match", () => {
    expect(filterThreadParticipants(people, "  ")).toHaveLength(4);
    expect(filterThreadParticipants(people, "zzz")).toEqual([]);
  });
});
