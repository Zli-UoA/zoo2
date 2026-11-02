import { readStorageValue, writeStorageValue } from "./storage";

export const HOME_POST_SETTINGS_PREFIX = "zoo-home-post-settings:v1:";
export interface HomePostSettings {
  visibility: "public" | "unlisted" | "followers";
  apub: boolean;
  channel_id: string | null;
}
const defaults: HomePostSettings = { visibility: "unlisted", apub: true, channel_id: null };

export function homePostSettingsKey(apiBaseUrl: string, userId: string): string {
  return HOME_POST_SETTINGS_PREFIX + JSON.stringify([apiBaseUrl, userId]);
}
export function loadHomePostSettings(key: string | null): HomePostSettings {
  try {
    const value: unknown = key ? JSON.parse(readStorageValue(key) ?? "null") : null;
    if (!value || typeof value !== "object" || Array.isArray(value)) return { ...defaults };
    const stored = value as Record<string, unknown>;
    const visibility =
      stored.visibility === "public" ||
      stored.visibility === "followers" ||
      stored.visibility === "unlisted"
        ? stored.visibility
        : defaults.visibility;
    return {
      visibility,
      apub: typeof stored.apub === "boolean" ? stored.apub : defaults.apub,
      channel_id:
        visibility !== "followers" &&
        typeof stored.channel_id === "string" &&
        stored.channel_id.length <= 200
          ? stored.channel_id || null
          : null,
    };
  } catch {
    return { ...defaults };
  }
}
export function saveHomePostSettings(key: string | null, settings: HomePostSettings): void {
  if (!key) return;
  // Persist only posting controls, never the post body or reply target.
  writeStorageValue(
    key,
    JSON.stringify({
      visibility: settings.visibility,
      apub: settings.apub,
      channel_id: settings.channel_id,
    }),
  );
}
export function replyVisibility(
  visibility: HomePostSettings["visibility"],
  parent?: string,
): HomePostSettings["visibility"] {
  if (parent === "followers") return "followers";
  return parent === "unlisted" && visibility === "public" ? "unlisted" : visibility;
}

export const HOME_TIMELINE_PREFIX = "zoo-home-timeline:v1:";
export type HomeTimeline = "home" | "local" | "social";
export function homeTimelineKey(apiBaseUrl: string, userId: string): string {
  return HOME_TIMELINE_PREFIX + JSON.stringify([apiBaseUrl, userId]);
}
export function loadHomeTimeline(key: string | null): HomeTimeline {
  const value = key ? readStorageValue(key) : null;
  return value === "local" || value === "social" ? value : "home";
}
export function saveHomeTimeline(key: string | null, value: HomeTimeline): void {
  if (key) writeStorageValue(key, value);
}
