import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { I18nextProvider } from "react-i18next";

import { createI18n } from "../../lib/i18n";
import {
  DEFAULT_LOCALE,
  LOCALE_STORAGE_KEY,
  isLocale,
  readBrowserLocale,
  type Locale,
} from "../../lib/i18n/locale";
import { subscribeStorage, writeStorageValue } from "../../lib/storage";

const LocaleContext = createContext<{ locale: Locale; setLocale: (locale: Locale) => void } | null>(
  null,
);

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [instance] = useState(() =>
    createI18n(typeof window === "undefined" ? DEFAULT_LOCALE : readBrowserLocale()),
  );
  // i18next is the sole source for translations and Intl formatting alike.
  const subscribe = useCallback(
    (onChange: () => void) => {
      instance.on("languageChanged", onChange);
      return () => {
        instance.off("languageChanged", onChange);
      };
    },
    [instance],
  );
  const getLocale = useCallback(
    () => (isLocale(instance.language) ? instance.language : DEFAULT_LOCALE),
    [instance],
  );
  const locale = useSyncExternalStore(subscribe, getLocale, getLocale);

  const apply = useCallback(
    (next: Locale) => {
      // All resources are bundled, so changeLanguage completes synchronously.
      document.documentElement.lang = next;
      void instance.changeLanguage(next);
    },
    [instance],
  );

  useEffect(() => {
    const refresh = () => apply(readBrowserLocale());
    const unsubscribe = subscribeStorage(LOCALE_STORAGE_KEY, refresh);
    window.addEventListener("languagechange", refresh);
    return () => {
      unsubscribe();
      window.removeEventListener("languagechange", refresh);
    };
  }, [apply]);

  const setLocale = useCallback(
    (next: Locale) => {
      apply(next);
      writeStorageValue(LOCALE_STORAGE_KEY, next);
    },
    [apply],
  );
  const value = useMemo(() => ({ locale, setLocale }), [locale, setLocale]);

  return (
    <LocaleContext.Provider value={value}>
      <I18nextProvider i18n={instance}>{children}</I18nextProvider>
    </LocaleContext.Provider>
  );
}

export function useLocale() {
  const context = useContext(LocaleContext);
  if (!context) throw new Error("useLocale requires LocaleProvider");
  return context;
}
