/** バックエンドの `MAX_CHANNEL_DESCRIPTION_LENGTH` と揃える。 */
export const MAX_CHANNEL_DESCRIPTION_LENGTH = 250;

/** Rust の `chars().count()` と同じくコードポイント単位で数える。 */
export function channelDescriptionLength(value: string): number {
  return Array.from(value).length;
}

export function isChannelDescriptionTooLong(value: string): boolean {
  return channelDescriptionLength(value) > MAX_CHANNEL_DESCRIPTION_LENGTH;
}

export function normalizeChannelDescription(value: string | null | undefined): string {
  return value?.trim() ?? "";
}
