export const SEARCH_ENGINES = {
  google: { label: "Google", action: "https://www.google.com/search", parameter: "q" },
  duckduckgo: { label: "DuckDuckGo", action: "https://duckduckgo.com/", parameter: "q" },
  yandex: { label: "Yandex", action: "https://yandex.com/search/", parameter: "text" },
  baidu: { label: "Baidu", action: "https://www.baidu.com/s", parameter: "wd" },
} as const;
export type SearchEngine = keyof typeof SEARCH_ENGINES;
export function isSearchEngine(value: unknown): value is SearchEngine {
  return typeof value === "string" && Object.hasOwn(SEARCH_ENGINES, value);
}
export function searchUrl(engine: SearchEngine, query: string): string {
  const provider = SEARCH_ENGINES[engine];
  const url = new URL(provider.action);
  url.searchParams.set(provider.parameter, query);
  return url.href;
}
