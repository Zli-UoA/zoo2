import { describe, expect, it } from "vite-plus/test";

import {
  MAX_RECENT_EMOJIS,
  RECENT_EMOJIS_STORAGE_KEY,
  addRecentEmoji,
  loadRecentEmojis,
} from "./recents";

function fakeStorage(initial: string | null = null) {
  let value = initial;
  return {
    getItem(key: string) {
      return key === RECENT_EMOJIS_STORAGE_KEY ? value : null;
    },
    setItem(key: string, next: string) {
      if (key === RECENT_EMOJIS_STORAGE_KEY) value = next;
    },
    get value() {
      return value;
    },
  };
}

describe("recent emojis", () => {
  it("returns an empty list when storage is empty, malformed or not an array", () => {
    expect(loadRecentEmojis(fakeStorage())).toEqual([]);
    expect(loadRecentEmojis(fakeStorage("not json"))).toEqual([]);
    expect(loadRecentEmojis(fakeStorage('{"emoji":"👍"}'))).toEqual([]);
    expect(loadRecentEmojis(null)).toEqual([]);
  });

  it("keeps only string entries and removes duplicates without reordering", () => {
    const storage = fakeStorage(JSON.stringify(["👍", 42, null, "🎉", "👍", "", "🔥"]));
    expect(loadRecentEmojis(storage)).toEqual(["👍", "🎉", "🔥"]);
  });

  it("moves a selected emoji to the front and persists it", () => {
    const storage = fakeStorage(JSON.stringify(["👍", "🎉"]));
    expect(addRecentEmoji("🎉", storage)).toEqual(["🎉", "👍"]);
    expect(storage.value).toBe(JSON.stringify(["🎉", "👍"]));
    expect(loadRecentEmojis(storage)).toEqual(["🎉", "👍"]);
  });

  it("caps the list at 16 entries, newest first", () => {
    const all = Array.from({ length: 20 }, (_, index) => `emoji-${index}`);
    const storage = fakeStorage(JSON.stringify(all));
    expect(loadRecentEmojis(storage)).toHaveLength(MAX_RECENT_EMOJIS);
    expect(loadRecentEmojis(storage)[0]).toBe("emoji-0");

    const next = addRecentEmoji("newest", storage);
    expect(next).toHaveLength(MAX_RECENT_EMOJIS);
    expect(next[0]).toBe("newest");
    expect(next.at(-1)).toBe("emoji-14");
  });

  it("keeps the in-memory list when writing storage fails", () => {
    const storage = {
      getItem() {
        return JSON.stringify(["👍"]);
      },
      setItem() {
        throw new Error("quota exceeded");
      },
    };
    expect(addRecentEmoji("🎉", storage)).toEqual(["🎉", "👍"]);
  });
});
