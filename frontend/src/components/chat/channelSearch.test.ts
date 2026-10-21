import { describe, expect, it } from "vite-plus/test";

import type { ChannelRow } from "../../lib/electric/types";
import { filterChannels } from "./channelSearch";

function channel(id: string, channel_name: string, is_private = false): ChannelRow {
  return {
    id,
    channel_name,
    description: null,
    created_user_id: "user-1",
    is_private,
    created_at: "2026-01-02 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
  };
}

const channels = [
  channel("c1", "general"),
  channel("c2", "Random"),
  channel("c3", "dev-frontend", true),
  channel("c4", "design"),
];

describe("filterChannels", () => {
  it("returns every channel in the original order when the query is empty", () => {
    expect(filterChannels(channels, "")).toBe(channels);
    expect(filterChannels(channels, "   ")).toBe(channels);
  });

  it("matches channel names partially, ignoring case", () => {
    expect(filterChannels(channels, "GEN").map((entry) => entry.id)).toEqual(["c1"]);
    expect(filterChannels(channels, "front").map((entry) => entry.id)).toEqual(["c3"]);
    expect(filterChannels(channels, "e").map((entry) => entry.id)).toEqual(["c1", "c3", "c4"]);
  });

  it("trims padding around the query", () => {
    expect(filterChannels(channels, "  random  ").map((entry) => entry.id)).toEqual(["c2"]);
  });

  it("returns an empty list when nothing matches", () => {
    expect(filterChannels(channels, "zzz")).toEqual([]);
  });

  it("does not change the input array", () => {
    const input = [...channels];
    filterChannels(input, "dev");
    expect(input).toEqual(channels);
  });
});
