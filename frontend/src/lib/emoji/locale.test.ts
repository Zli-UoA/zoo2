import { describe, expect, it } from "vite-plus/test";

import { LOCALES } from "../i18n/locale";
import {
  DEFAULT_EMOJI_DATA_LOCALE,
  EMOJI_DATA_LOCALE_BY_LOCALE,
  EMOJI_DATA_LOCALES,
  resolveEmojiDataLocale,
} from "./locale";

describe("emoji data locale", () => {
  it("maps every app locale to a self-hosted dataset", () => {
    expect(Object.keys(EMOJI_DATA_LOCALE_BY_LOCALE).sort()).toEqual([...LOCALES].sort());
    for (const locale of LOCALES) {
      expect(EMOJI_DATA_LOCALES).toContain(EMOJI_DATA_LOCALE_BY_LOCALE[locale]);
    }
  });

  it.each([
    ["ja", "ja"],
    ["en", "en"],
    ["zh-Hant", "zh-hant"],
    ["zh-Hans", "zh-hant"],
    ["yue", "zh-hant"],
  ])("resolves %s to %s", (locale, expected) => {
    expect(resolveEmojiDataLocale(locale)).toBe(expected);
  });

  it("falls back to English for unsupported locales", () => {
    expect(resolveEmojiDataLocale("fr")).toBe(DEFAULT_EMOJI_DATA_LOCALE);
    expect(resolveEmojiDataLocale("")).toBe("en");
  });
});
