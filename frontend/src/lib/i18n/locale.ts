import { readStorageValue } from "../storage";

export const DEFAULT_LOCALE = "ja";
export const LOCALE_STORAGE_KEY = "zoo-locale";
export const LOCALES = ["ja", "en", "zh-Hant", "zh-Hans", "yue"] as const;
export type Locale = (typeof LOCALES)[number];

export const LANGUAGE_NAMES: Record<Locale, string> = {
  ja: "日本語",
  en: "English",
  "zh-Hant": "繁體中文",
  "zh-Hans": "简体中文",
  yue: "廣東話",
};

export const INTL_LOCALES: Record<Locale, string> = {
  ja: "ja-JP",
  en: "en-US",
  "zh-Hant": "zh-TW",
  "zh-Hans": "zh-CN",
  yue: "yue-Hant-HK",
};

export function isLocale(value: unknown): value is Locale {
  return LOCALES.some((locale) => locale === value);
}

/** Explicit script wins over region; bare Chinese defaults to simplified. */
export function matchLocale(value: string): Locale | null {
  const tag = value.toLowerCase().replaceAll("_", "-");
  const [language, ...subtags] = tag.split("-");
  if (language === "ja" || language === "en" || language === "yue") return language;
  if (language !== "zh") return null;
  if (subtags.includes("yue")) return "yue";
  if (subtags.includes("hant")) return "zh-Hant";
  if (subtags.includes("hans")) return "zh-Hans";
  return subtags.some((part) => ["tw", "hk", "mo"].includes(part)) ? "zh-Hant" : "zh-Hans";
}

export function resolveLocale(stored: unknown, languages: readonly string[]): Locale {
  if (isLocale(stored)) return stored;
  for (const language of languages) {
    const locale = matchLocale(language);
    if (locale) return locale;
  }
  return DEFAULT_LOCALE;
}

export function readBrowserLocale(): Locale {
  const stored = readStorageValue(LOCALE_STORAGE_KEY);
  return resolveLocale(
    stored,
    navigator.languages.length ? navigator.languages : [navigator.language],
  );
}

/** Runs before hydration. matchLocale is self-contained so its bundled source is safe to serialize. */
export const localeInitScript = `!function(){var stored=null;try{stored=localStorage.getItem(${JSON.stringify(LOCALE_STORAGE_KEY)})}catch(e){}var languages=navigator.languages.length?navigator.languages:[navigator.language];document.documentElement.lang=${JSON.stringify(LOCALES)}.includes(stored)?stored:languages.map(${matchLocale.toString()}).find(Boolean)||${JSON.stringify(DEFAULT_LOCALE)}}();`;
