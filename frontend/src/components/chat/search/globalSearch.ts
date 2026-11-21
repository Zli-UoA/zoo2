import type { DisplayEmoji, MessageDisplayData } from "../../../lib/electric/types";

/**
 * クライアントで表示する種類。`all`（既定）はメッセージとファイルをまとめて探す。
 * ノート（`type=notes`）は画面がないので扱わない。
 */
export const SEARCH_KINDS = ["all", "messages", "files"] as const;
export type SearchKind = (typeof SEARCH_KINDS)[number];

/** 絞り込み。送信者（`user_id`）・チャンネル（`channel_id`）・期間（UTC の `YYYY-MM-DD`）。 */
export interface SearchFilters {
  from?: string;
  in?: string;
  /** この日を含む。 */
  after?: string;
  /** この日を含まない。 */
  before?: string;
}

export interface SearchRouteParams extends SearchFilters {
  q?: string;
  type?: Exclude<SearchKind, "all">;
}

type SearchAuthor = NonNullable<MessageDisplayData["author"]>;

interface SearchItemBase {
  id: string;
  kind?: "message" | "file";
  channel_id: string;
  channel_name: string;
  is_dm: boolean;
  /** スレッドの返信なら親の ID。 */
  parent_id: string | null;
  user_id: string | null;
  author?: SearchAuthor | null;
  snippet: string;
  created_at: string;
}

export interface SearchMessageItem extends SearchItemBase {
  emojis?: Record<string, DisplayEmoji>;
}

export interface SearchFileItem extends SearchItemBase {
  message_id: string;
  file_name: string;
  content_type: string;
  file_size: number | string;
  width: number | null;
  height: number | null;
  is_sensitive: boolean;
}

export type SearchItem = SearchMessageItem | SearchFileItem;

export interface SearchPage {
  items: SearchItem[];
  next_cursor: string | null;
  /** どの月まで見たか（`YYYY-MM`）。見えるチャンネルがなければ null。 */
  searched_until: string | null;
}

/** `/search` の URL の検索パラメーター。既定値（空の語・all・絞り込みなし）は URL に出さない。 */
export function parseSearchRouteParams(search: Record<string, unknown>): SearchRouteParams {
  // TanStack Router は `?q=123` を数値として読むので文字列へ戻す。
  const raw =
    typeof search.q === "string"
      ? search.q
      : typeof search.q === "number" || typeof search.q === "boolean"
        ? String(search.q)
        : "";
  const q = normalizeSearchQuery(raw);
  const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);
  const day = (value: unknown) =>
    typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/u.test(value) ? value : undefined;
  return {
    q: q || undefined,
    type: search.type === "files" || search.type === "messages" ? search.type : undefined,
    from: text(search.from),
    in: text(search.in),
    after: day(search.after),
    before: day(search.before),
  };
}

export function searchFiltersOf(params: SearchRouteParams): SearchFilters {
  return { from: params.from, in: params.in, after: params.after, before: params.before };
}

export function hasSearchFilters(filters: SearchFilters): boolean {
  return !!(filters.from || filters.in || filters.after || filters.before);
}

/** 期間の選択肢。`custom` は日付を指定する。 */
export const SEARCH_PERIODS = ["anytime", "last7", "last30", "last365", "custom"] as const;
export type SearchPeriod = (typeof SEARCH_PERIODS)[number];
const PERIOD_DAYS: Record<"last7" | "last30" | "last365", number> = {
  last7: 7,
  last30: 30,
  last365: 365,
};

/** UTC の日付（`YYYY-MM-DD`）。 */
export function utcDay(date: Date): string {
  return date.toISOString().slice(0, 10);
}

/** 「過去 N 日間」の開始日（今日を含めて N 日）。 */
export function periodAfter(period: "last7" | "last30" | "last365", now: Date): string {
  return utcDay(new Date(now.getTime() - (PERIOD_DAYS[period] - 1) * 86_400_000));
}

/** 今の絞り込みが、どの期間の選択肢に当たるか。 */
export function periodOf(filters: SearchFilters, now: Date): SearchPeriod {
  if (!filters.after && !filters.before) return "anytime";
  if (!filters.before) {
    for (const period of ["last7", "last30", "last365"] as const) {
      if (filters.after === periodAfter(period, now)) return period;
    }
  }
  return "custom";
}

/** 前後の空白を除き、連続する空白を 1 つにする。サーバーも空白で語を区切る。 */
export function normalizeSearchQuery(value: string): string {
  return value.trim().replace(/\s+/gu, " ");
}

export function searchKindOf(params: SearchRouteParams): SearchKind {
  return params.type ?? "all";
}

export function searchRequestPath(
  query: string,
  kind: SearchKind,
  cursor: string | null,
  filters: SearchFilters = {},
): string {
  const params = new URLSearchParams({ q: query, type: kind });
  if (filters.from) params.set("user_id", filters.from);
  if (filters.in) params.set("channel_id", filters.in);
  if (filters.after) params.set("after", filters.after);
  if (filters.before) params.set("before", filters.before);
  if (cursor) params.set("cursor", cursor);
  return `/api/search?${params}`;
}

export function isFileItem(item: SearchItem): item is SearchFileItem {
  return item.kind === "file" || "file_name" in item;
}

/** 結果を開くリンク。返信はスレッドを開いたうえで、その返信へ移動する。 */
export function searchResultLink(item: SearchItem): {
  channelId: string;
  search: { message: string; thread?: string };
} {
  const message = isFileItem(item) ? item.message_id : item.id;
  return {
    channelId: item.channel_id,
    search: item.parent_id ? { thread: item.parent_id, message } : { message },
  };
}

/** 新しい順を保ったまま、前のページと重なった項目を除いてつなぐ。 */
export function appendSearchItems(current: readonly SearchItem[], next: readonly SearchItem[]) {
  const seen = new Set(current.map((item) => item.id));
  return [...current, ...next.filter((item) => !seen.has(item.id))];
}
