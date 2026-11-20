/** バックエンドの `MAX_CHANNEL_NAME_LENGTH` と揃える。 */
export const MAX_CHANNEL_NAME_LENGTH = 80;

/** バックエンドの `RESERVED_CHANNEL_NAMES` と揃える（大小文字を区別する）。 */
const RESERVED_CHANNEL_NAMES = new Set(["all", "channel", "everyone", "here", "general"]);

/** バックエンドの `CHANNEL_NAME_PATTERN` と揃える（文字・結合文字・数字・-・_）。 */
const CHANNEL_NAME_PATTERN = /^[\p{L}\p{M}\p{N}_-]+$/u;

export type ChannelNameError = "empty" | "tooLong" | "reserved" | "invalidCharacters";

/** Rust の `chars().count()` と同じくコードポイント単位で数える。 */
export function channelNameLength(value: string): number {
  return Array.from(value).length;
}

/**
 * バックエンドの `validate_channel_name` と同じ規則で検証する。
 * 違反があれば最初の理由を返し、問題なければ `null` を返す。
 */
export function validateChannelName(name: string): ChannelNameError | null {
  if (name.trim().length === 0) {
    return "empty";
  }
  if (channelNameLength(name) > MAX_CHANNEL_NAME_LENGTH) {
    return "tooLong";
  }
  if (RESERVED_CHANNEL_NAMES.has(name)) {
    return "reserved";
  }
  if (!CHANNEL_NAME_PATTERN.test(name)) {
    return "invalidCharacters";
  }
  return null;
}
