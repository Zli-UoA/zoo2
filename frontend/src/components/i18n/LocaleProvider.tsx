import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { I18nextProvider } from "react-i18next";

import { createI18n } from "../../lib/i18n";
import {
  DEFAULT_LOCALE,
  LOCALE_STORAGE_KEY,
  readBrowserLocale,
  type Locale,
} from "../../lib/i18n/locale";

const LocaleContext = createContext<{ locale: Locale; setLocale: (locale: Locale) => void } | null>(
  null,
);

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [instance] = useState(createI18n);
  const [locale, setLocaleState] = useState<Locale>(DEFAULT_LOCALE);

  useEffect(() => {
    const apply = () => setLocaleState(readBrowserLocale());
    apply();
    const onStorage = (event: StorageEvent) => {
      if (event.key === LOCALE_STORAGE_KEY || event.key === null) apply();
    };
    window.addEventListener("storage", onStorage);
    window.addEventListener("languagechange", apply);
    return () => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("languagechange", apply);
    };
  }, []);

  useEffect(() => {
    void instance.changeLanguage(locale);
    document.documentElement.lang = locale;
  }, [instance, locale]);

  const setLocale = (next: Locale) => {
    setLocaleState(next);
    try {
      window.localStorage.setItem(LOCALE_STORAGE_KEY, next);
    } catch {
      // Keep the selection for this session when storage is blocked.
    }
  };

  return (
    <LocaleContext.Provider value={{ locale, setLocale }}>
      <I18nextProvider i18n={instance}>{children}</I18nextProvider>
    </LocaleContext.Provider>
  );
}

export function useLocale() {
  const context = useContext(LocaleContext);
  if (!context) throw new Error("useLocale requires LocaleProvider");
  return context;
}
