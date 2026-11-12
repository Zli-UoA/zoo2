import { defaultEmojiDataResolver, type EmojiDataResolver } from "frimousse";

import { DEFAULT_EMOJI_DATA_LOCALE } from "./locale";

/**
 * Resolves the active locale's Emojibase data and adds the English label and
 * tags to each emoji's search tags, so e.g. "smile" also finds emojis while
 * the app language is Japanese. Displayed labels stay in the active locale.
 * If the English data cannot be loaded, the localized data is used as-is.
 */
export const resolveMultilingualEmojiData: EmojiDataResolver = async (locale, options) => {
  if (locale === DEFAULT_EMOJI_DATA_LOCALE) return defaultEmojiDataResolver(locale, options);

  const [localized, english] = await Promise.all([
    defaultEmojiDataResolver(locale, options),
    Promise.resolve(defaultEmojiDataResolver(DEFAULT_EMOJI_DATA_LOCALE, options)).catch(() => null),
  ]);
  if (!english) return localized;

  const englishByEmoji = new Map(english.emojis.map((entry) => [entry.emoji, entry]));
  return {
    ...localized,
    emojis: localized.emojis.map((entry) => {
      const en = englishByEmoji.get(entry.emoji);
      if (!en) return entry;
      return { ...entry, tags: [...new Set([...entry.tags, en.label, ...en.tags])] };
    }),
  };
};
