/** Base URL for the self-hosted Emojibase data copied into `public/`. */
export const EMOJIBASE_DATA_URL = "/emojibase-data";

/**
 * Highest emoji version offered by the picker. The backend `emojis` crate
 * (0.6.x) validates up to Unicode 16.0, so newer emojis would be rejected.
 * Keep this in sync when upgrading either side.
 */
export const SUPPORTED_EMOJI_VERSION = 16;
