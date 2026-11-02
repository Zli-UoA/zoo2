import { afterEach, expect, it, vi } from "vite-plus/test";
import {
  homePostSettingsKey,
  loadHomePostSettings,
  saveHomePostSettings,
  replyVisibility,
  homeTimelineKey,
  loadHomeTimeline,
  saveHomeTimeline,
} from "./homePreferences";

afterEach(() => vi.unstubAllGlobals());
function storage() {
  const values = new Map<string, string>();
  vi.stubGlobal("window", {
    localStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    },
  });
  return values;
}
it("persists only posting controls and isolates accounts and servers", () => {
  const values = storage();
  const key = homePostSettingsKey("/api", "alice");
  const settings = {
    visibility: "public" as const,
    apub: false,
    channel_id: "general",
    content: "not a saved draft",
    in_reply_to: "private reply",
  };
  saveHomePostSettings(key, settings);
  expect(loadHomePostSettings(key)).toEqual({
    visibility: "public",
    apub: false,
    channel_id: "general",
  });
  expect(JSON.parse(values.get(key)!)).not.toHaveProperty("content");
  expect(JSON.parse(values.get(key)!)).not.toHaveProperty("in_reply_to");
  for (const other of [
    homePostSettingsKey("/api", "bob"),
    homePostSettingsKey("https://other.example", "alice"),
  ]) {
    expect(loadHomePostSettings(other)).toEqual({
      visibility: "unlisted",
      apub: true,
      channel_id: null,
    });
  }
});
it("ignores malformed settings and incompatible followers-only channel links", () => {
  const values = storage();
  for (const value of [
    "{",
    "null",
    "[]",
    '"public"',
    '{"visibility":"invalid","apub":"true","channel_id":42}',
  ]) {
    values.set("key", value);
    expect(loadHomePostSettings("key")).toEqual({
      visibility: "unlisted",
      apub: true,
      channel_id: null,
    });
  }
  values.set(
    "key",
    JSON.stringify({ visibility: "followers", apub: false, channel_id: "general" }),
  );
  expect(loadHomePostSettings("key")).toEqual({
    visibility: "followers",
    apub: false,
    channel_id: null,
  });
});
it("restores the selected TL without sharing it with other accounts", () => {
  storage();
  const key = homeTimelineKey("/api", "alice");
  for (const timeline of ["local", "social", "home"] as const) {
    saveHomeTimeline(key, timeline);
    expect(loadHomeTimeline(key)).toBe(timeline);
    expect(loadHomeTimeline(homeTimelineKey("/api", "bob"))).toBe("home");
  }
});
it("falls back without blocking posting when local storage is broken or unavailable", () => {
  const values = storage();
  values.set("timeline", "not-a-timeline");
  expect(loadHomeTimeline("timeline")).toBe("home");
  vi.stubGlobal("window", {
    get localStorage() {
      throw new Error("blocked");
    },
  });
  expect(loadHomePostSettings("key")).toEqual({
    visibility: "unlisted",
    apub: true,
    channel_id: null,
  });
  expect(() =>
    saveHomePostSettings("key", { visibility: "followers", apub: false, channel_id: null }),
  ).not.toThrow();
  expect(loadHomeTimeline("key")).toBe("home");
  expect(() => saveHomeTimeline("key", "local")).not.toThrow();
});
it("never widens a saved audience when opening a reply", () => {
  expect(replyVisibility("public", "followers")).toBe("followers");
  expect(replyVisibility("public", "unlisted")).toBe("unlisted");
  expect(replyVisibility("followers", "public")).toBe("followers");
  expect(replyVisibility("unlisted", "public")).toBe("unlisted");
});
