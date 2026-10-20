import { readStorageValue } from "../../lib/storage";

/**
 * テーマの保存・解決ロジック。React に依存しない純粋な部分をここへ置き、
 * `ThemeProvider` と FOUC 防止スクリプトの双方から同じ定義を使う。
 */
export type Theme = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "zoo-theme";
export const DEFAULT_THEME: Theme = "system";

export function isTheme(value: unknown): value is Theme {
  return value === "light" || value === "dark" || value === "system";
}

/**
 * localStorage から保存済みテーマを読む。未保存・不正値・storage 不可の場合は既定値。
 */
export function readStoredTheme(storage?: Pick<Storage, "getItem"> | null): Theme {
  const raw = readStorageValue(THEME_STORAGE_KEY, storage);
  return isTheme(raw) ? raw : DEFAULT_THEME;
}

/**
 * `system` は OS 設定に追従する。それ以外はそのまま解決する。
 */
export function resolveTheme(theme: Theme, prefersDark: boolean): ResolvedTheme {
  if (theme === "system") {
    return prefersDark ? "dark" : "light";
  }
  return theme;
}

export interface ThemeRoot {
  classList: { toggle: (token: string, force?: boolean) => void };
  style: { colorScheme: string };
}

/**
 * `<html>` にテーマクラスと `color-scheme` を反映する。
 */
export function applyThemeClass(root: ThemeRoot, resolved: ResolvedTheme): void {
  const isDark = resolved === "dark";
  root.classList.toggle("dark", isDark);
  root.style.colorScheme = resolved;
}

/**
 * `<head>` で同期実行する FOUC 防止スクリプト。React のマウント前に
 * localStorage を読み、`<html class>` を確定させる。
 */
export const themeInitScript = `!function(){try{var e=localStorage.getItem(${JSON.stringify(
  THEME_STORAGE_KEY,
)});var t=e==="light"||e==="dark"||e==="system"?e:"system";var d=t==="dark"||t==="system"&&window.matchMedia("(prefers-color-scheme: dark)").matches;var o=document.documentElement;o.classList.toggle("dark",d);o.style.colorScheme=d?"dark":"light"}catch(e){}}();`;
