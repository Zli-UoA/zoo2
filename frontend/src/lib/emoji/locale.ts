import type { Locale } from "../i18n/locale";

/** Locales available under `public/emojibase-data`. */
export const EMOJI_DATA_LOCALES = ["en", "ja", "zh-hant"] as const;
export type EmojiDataLocale = (typeof EMOJI_DATA_LOCALES)[number];

export const DEFAULT_EMOJI_DATA_LOCALE: EmojiDataLocale = "en";

/**
 * Maps every app locale to the self-hosted Emojibase dataset. Simplified
 * Chinese and Cantonese use the traditional dataset because Emojibase does not
 * ship dedicated data for them.
 */
export const EMOJI_DATA_LOCALE_BY_LOCALE: Record<Locale, EmojiDataLocale> = {
  ja: "ja",
  en: "en",
  "zh-Hant": "zh-hant",
  "zh-Hans": "zh-hant",
  yue: "zh-hant",
};

export function resolveEmojiDataLocale(locale: string): EmojiDataLocale {
  return EMOJI_DATA_LOCALE_BY_LOCALE[locale as Locale] ?? DEFAULT_EMOJI_DATA_LOCALE;
}
