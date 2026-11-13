import { describe, expect, it } from "vite-plus/test";

import type { ChannelRow } from "../../lib/electric/types";
import {
  filterDiscoverableChannels,
  filterJoinedChannels,
  isChannelJoinable,
} from "./channelSearch";

function channel(
  id: string,
  channel_name: string,
  options: { isPrivate?: boolean; archived?: boolean } = {},
): ChannelRow {
  return {
    id,
    channel_name,
    description: null,
    created_user_id: "user-1",
    is_private: options.isPrivate ?? false,
    is_dm: false,
    created_at: "2026-01-02 00:00:00.000",
    updated_at: null,
    archive_at: options.archived ? "2026-01-03 00:00:00.000" : null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
  };
}

const channels = [
  channel("c1", "general"),
  channel("c2", "Random"),
  channel("c3", "dev-frontend", { isPrivate: true }),
  channel("c4", "design"),
  channel("c5", "old-town", { archived: true }),
];

describe("filterJoinedChannels", () => {
  it("keeps only joined channels in the original order", () => {
    expect(filterJoinedChannels(channels, new Set(["c4", "c1"])).map((entry) => entry.id)).toEqual([
      "c1",
      "c4",
    ]);
  });

  it("includes private channels but hides archived ones even when joined", () => {
    expect(filterJoinedChannels(channels, new Set(["c3", "c5"])).map((entry) => entry.id)).toEqual([
      "c3",
    ]);
  });

  it("returns an empty list when nothing is joined", () => {
    expect(filterJoinedChannels(channels, new Set())).toEqual([]);
  });
});

describe("isChannelJoinable", () => {
  it("allows public channels that are not archived", () => {
    expect(isChannelJoinable(channel("c1", "general"))).toBe(true);
  });

  it("rejects private and archived channels", () => {
    expect(isChannelJoinable(channel("c1", "general", { isPrivate: true }))).toBe(false);
    expect(isChannelJoinable(channel("c1", "general", { archived: true }))).toBe(false);
  });
});

describe("filterDiscoverableChannels", () => {
  it("excludes joined and private channels but keeps archived ones", () => {
    expect(
      filterDiscoverableChannels(channels, new Set(["c1"]), "").map((entry) => entry.id),
    ).toEqual(["c2", "c4", "c5"]);
  });

  it("keeps joined private archived channels but not unjoined private ones", () => {
    const privateArchived = channel("c6", "secret-old", { isPrivate: true, archived: true });
    const unjoined = channel("c7", "secret-other", { isPrivate: true, archived: true });
    const all = [...channels, privateArchived, unjoined];
    expect(filterJoinedChannels(all, new Set(["c6"]))).toEqual([]);
    expect(
      filterDiscoverableChannels(all, new Set(["c6"]), "secret").map((entry) => entry.id),
    ).toEqual(["c6"]);
  });

  it("keeps joined archived channels because the sidebar hides them", () => {
    expect(
      filterDiscoverableChannels(channels, new Set(["c1", "c5"]), "").map((entry) => entry.id),
    ).toEqual(["c2", "c4", "c5"]);
  });

  it("matches channel names partially, ignoring case", () => {
    expect(filterDiscoverableChannels(channels, new Set(), "DES").map((entry) => entry.id)).toEqual(
      ["c4"],
    );
    expect(filterDiscoverableChannels(channels, new Set(), "e").map((entry) => entry.id)).toEqual([
      "c1",
      "c4",
    ]);
  });

  it("trims padding around the query", () => {
    expect(
      filterDiscoverableChannels(channels, new Set(), "  random  ").map((entry) => entry.id),
    ).toEqual(["c2"]);
  });

  it("keeps archived channels so they can be shown as unavailable", () => {
    const result = filterDiscoverableChannels(channels, new Set(), "old");
    expect(result.map((entry) => entry.id)).toEqual(["c5"]);
    expect(isChannelJoinable(result[0])).toBe(false);
  });

  it("returns an empty list when nothing matches", () => {
    expect(filterDiscoverableChannels(channels, new Set(), "zzz")).toEqual([]);
  });

  it("does not change the input array", () => {
    const input = [...channels];
    filterDiscoverableChannels(input, new Set(), "dev");
    expect(input).toEqual(channels);
  });
});
