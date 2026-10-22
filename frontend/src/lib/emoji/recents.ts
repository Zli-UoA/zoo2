import { readStorageValue } from "../storage";

export const RECENT_EMOJIS_STORAGE_KEY = "zoo-recent-emoji";
export const MAX_RECENT_EMOJIS = 16;

type RecentsStorage = Pick<Storage, "getItem" | "setItem">;

/** Reads recents newest-first, dropping malformed entries and duplicates. */
export function loadRecentEmojis(storage?: Pick<Storage, "getItem"> | null): string[] {
  const raw = readStorageValue(RECENT_EMOJIS_STORAGE_KEY, storage);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const seen = new Set<string>();
    const recents: string[] = [];
    for (const value of parsed) {
      if (typeof value !== "string" || value === "" || seen.has(value)) continue;
      seen.add(value);
      recents.push(value);
      if (recents.length === MAX_RECENT_EMOJIS) break;
    }
    return recents;
  } catch {
    return [];
  }
}

/** Moves `emoji` to the front, deduplicates and caps the list, then persists it. */
export function addRecentEmoji(emoji: string, storage?: RecentsStorage | null): string[] {
  const recents = [emoji, ...loadRecentEmojis(storage).filter((value) => value !== emoji)].slice(
    0,
    MAX_RECENT_EMOJIS,
  );
  try {
    const target = storage === undefined ? window.localStorage : storage;
    target?.setItem(RECENT_EMOJIS_STORAGE_KEY, JSON.stringify(recents));
  } catch {
    // Recents are best-effort when storage is unavailable.
  }
  return recents;
}
