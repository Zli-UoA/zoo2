import { createInstance } from "i18next";

import ja from "./locales/ja.json";
import en from "./locales/en.json";
import zhHant from "./locales/zh-Hant.json";
import zhHans from "./locales/zh-Hans.json";
import yue from "./locales/yue.json";
import { DEFAULT_LOCALE, LOCALES, type Locale } from "./locale";

export const resources = {
  ja: { translation: ja },
  en: { translation: en },
  "zh-Hant": { translation: zhHant },
  "zh-Hans": { translation: zhHans },
  yue: { translation: yue },
};

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: typeof ja };
  }
}

/** A separate instance for each React tree keeps SSR requests isolated. */
export function createI18n(locale: Locale = DEFAULT_LOCALE) {
  const instance = createInstance();
  void instance.init({
    resources,
    lng: locale,
    fallbackLng: DEFAULT_LOCALE,
    supportedLngs: [...LOCALES],
    load: "currentOnly",
    initAsync: false,
    interpolation: { escapeValue: false },
    react: { useSuspense: false },
  });
  return instance;
}

// Read-only fixed translators for pure formatting helpers; never change its language.
const formatterI18n = createI18n();
export function getTranslator(locale: Locale) {
  return formatterI18n.getFixedT(locale);
}
