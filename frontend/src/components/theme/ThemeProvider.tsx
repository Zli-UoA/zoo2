import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import {
  DEFAULT_THEME,
  THEME_STORAGE_KEY,
  applyThemeClass,
  readStoredTheme,
  resolveTheme,
  type ResolvedTheme,
  type Theme,
} from "./theme";

export interface ThemeContextValue {
  /** ユーザーが選んだテーマ（`system` を含む）。 */
  theme: Theme;
  /** `system` を OS 設定に解決した実効テーマ。 */
  resolvedTheme: ResolvedTheme;
  setTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function prefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/**
 * ライト/ダーク/システムの切替を localStorage に永続化する Provider。
 * 初回描画は SSR と一致させ、マウント後に保存値と OS 設定へ同期する
 * （実際のクラス適用は `__root.tsx` の FOUC 防止スクリプトが先に行う）。
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(DEFAULT_THEME);
  const [systemPrefersDark, setSystemPrefersDark] = useState(false);
  const [hydrated, setHydrated] = useState(false);

  const resolvedTheme = resolveTheme(theme, systemPrefersDark);

  useEffect(() => {
    setThemeState(readStoredTheme(window.localStorage));
    setSystemPrefersDark(prefersDark());
    setHydrated(true);

    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = (event: MediaQueryListEvent) => {
      setSystemPrefersDark(event.matches);
    };
    const handleStorage = (event: StorageEvent) => {
      if (event.key === THEME_STORAGE_KEY) {
        setThemeState(readStoredTheme(window.localStorage));
      }
    };
    query.addEventListener("change", handleChange);
    window.addEventListener("storage", handleStorage);
    return () => {
      query.removeEventListener("change", handleChange);
      window.removeEventListener("storage", handleStorage);
    };
  }, []);

  useEffect(() => {
    if (!hydrated) {
      return;
    }
    applyThemeClass(document.documentElement, resolvedTheme);
  }, [hydrated, resolvedTheme]);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      // localStorage が使えない環境ではセッション内のみ反映する。
    }
  }, []);

  const value = useMemo<ThemeContextValue>(
    () => ({ theme, resolvedTheme, setTheme }),
    [theme, resolvedTheme, setTheme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error("useTheme は ThemeProvider の内側で使ってください。");
  }
  return context;
}
