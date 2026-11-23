import type { DisplayEmoji, MessageDisplayData } from "../../../lib/electric/types";

/**
 * クライアントで表示する種類。`all`（既定）はメッセージとファイルをまとめて探す。
 * ノート（`type=notes`）は画面がないので扱わない。
 */
export const SEARCH_KINDS = ["all", "messages", "files"] as const;
export type SearchKind = (typeof SEARCH_KINDS)[number];

/** 送信者・チャンネルの絞り込みで一度に選べる数（サーバーの `MAX_FILTER_IDS`）。 */
export const MAX_SEARCH_FILTER_IDS = 20;

/**
 * 絞り込み。送信者（`user_id`）・チャンネル（`channel_id`）はどれか（複数可）、期間は端末の暦の日
 * （`YYYY-MM-DD`）。`noBots` はボット（アプリ）の投稿を除き、`mine` は自分が参加中の会話（所属する
 * チャンネル・DM、参加しているスレッド、メンションされたスレッド）に絞る。
 */
export interface SearchFilters {
  from?: string[];
  in?: string[];
  /** この日を含む。 */
  after?: string;
  /** この日を含まない。 */
  before?: string;
  noBots?: boolean;
  mine?: boolean;
}

/** URL での形。送信者とチャンネルはカンマ区切り、フラグは `1`。 */
export interface SearchRouteParams {
  q?: string;
  type?: Exclude<SearchKind, "all">;
  from?: string;
  in?: string;
  after?: string;
  before?: string;
  nobots?: "1";
  mine?: "1";
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
  const ids = (value: unknown) => {
    const list = splitIds(typeof value === "string" ? value : "");
    return list.length ? list.join(",") : undefined;
  };
  const day = (value: unknown) =>
    typeof value === "string" && isSearchDay(value) ? value : undefined;
  // ルーターは `1` を数値として読む。
  const flag = (value: unknown) =>
    value === "1" || value === 1 || value === true ? ("1" as const) : undefined;
  const params: SearchRouteParams = {
    q: q || undefined,
    type: search.type === "files" || search.type === "messages" ? search.type : undefined,
    from: ids(search.from),
    in: ids(search.in),
    after: day(search.after),
    before: day(search.before),
    nobots: flag(search.nobots),
    mine: flag(search.mine),
  };
  for (const key of Object.keys(params) as (keyof SearchRouteParams)[]) {
    if (params[key] === undefined) delete params[key];
  }
  return params;
}

/** カンマ区切りの ID を、重複と空を除いて最大 [`MAX_SEARCH_FILTER_IDS`] 個にする。 */
function splitIds(value: string): string[] {
  const out: string[] = [];
  for (const id of value.split(",").map((part) => part.trim())) {
    if (id && !out.includes(id)) out.push(id);
  }
  return out.slice(0, MAX_SEARCH_FILTER_IDS);
}

export function searchFiltersOf(params: SearchRouteParams): SearchFilters {
  const list = (value: string | undefined) => (value ? splitIds(value) : undefined);
  return {
    from: list(params.from),
    in: list(params.in),
    after: params.after,
    before: params.before,
    noBots: params.nobots === "1" || undefined,
    mine: params.mine === "1" || undefined,
  };
}

/** 絞り込みを URL の形にする（既定値は出さない）。 */
export function searchRouteFilters(filters: SearchFilters) {
  return {
    from: filters.from?.length ? filters.from.join(",") : undefined,
    in: filters.in?.length ? filters.in.join(",") : undefined,
    after: filters.after || undefined,
    before: filters.before || undefined,
    nobots: filters.noBots ? ("1" as const) : undefined,
    mine: filters.mine ? ("1" as const) : undefined,
  };
}

export function hasSearchFilters(filters: SearchFilters): boolean {
  return !!(
    filters.from?.length ||
    filters.in?.length ||
    filters.after ||
    filters.before ||
    filters.noBots ||
    filters.mine
  );
}

/** 期間の選択肢。`custom` は日付を指定する。 */
export const SEARCH_PERIODS = ["anytime", "last7", "last30", "last365", "custom"] as const;
export type SearchPeriod = (typeof SEARCH_PERIODS)[number];
const PERIOD_DAYS: Record<"last7" | "last30" | "last365", number> = {
  last7: 7,
  last30: 30,
  last365: 365,
};

/** 実在する日付（`YYYY-MM-DD`）か。`2026-13-01` や `2026-02-30` は除く。日付は端末の暦の日。 */
export function isSearchDay(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/u.test(value)) return false;
  const time = Date.parse(`${value}T00:00:00Z`);
  return !Number.isNaN(time) && new Date(time).toISOString().slice(0, 10) === value;
}

/** 日付（`YYYY-MM-DD`）を `days` 日ずらす（暦の上で数えるので、タイムゾーンによらない）。 */
export function shiftDay(day: string, days: number): string {
  return utcDay(new Date(Date.parse(`${day}T00:00:00Z`) + days * 86_400_000));
}

/** 画面の「終了日」はその日を含む。API の `before` は含まないので、翌日にして送る。 */
export function beforeFromEndDay(end: string | undefined): string | undefined {
  return end ? shiftDay(end, 1) : undefined;
}

export function endDayFromBefore(before: string | undefined): string | undefined {
  return before ? shiftDay(before, -1) : undefined;
}

/** UTC の日付（`YYYY-MM-DD`）。暦の計算にだけ使う。 */
function utcDay(date: Date): string {
  return date.toISOString().slice(0, 10);
}

const pad = (value: number, length = 2) => String(value).padStart(length, "0");

/** 端末のタイムゾーンでの日付（`YYYY-MM-DD`）。 */
export function localDay(date: Date): string {
  return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 「過去 N 日間」の開始日（端末の今日を含めて N 日）。 */
export function periodAfter(period: "last7" | "last30" | "last365", now: Date): string {
  return localDay(
    new Date(now.getFullYear(), now.getMonth(), now.getDate() - (PERIOD_DAYS[period] - 1)),
  );
}

/**
 * 日付（端末の暦の日）の 0 時を、オフセット付きの時刻（`2026-11-22T00:00:00+09:00`）にする。API の
 * `after`/`before` は、この瞬間で区切る（UTC の日付で区切ると、JST の 0〜9 時の投稿が前日に入る）。
 * 夏時間で 0 時がない日は、その日の最初の時刻にする。
 */
export function localMidnight(day: string): string {
  // 変換済みの時刻はそのまま（検索の最初に決めた境界を、続きのページでも使うため）。
  if (day.length !== 10) return day;
  const [year, month, date] = day.split("-").map(Number);
  const at = new Date(2000, 0, 1);
  at.setFullYear(year!, month! - 1, date!);
  const offset = -at.getTimezoneOffset();
  const sign = offset < 0 ? "-" : "+";
  const abs = Math.abs(offset);
  return `${localDay(at)}T${pad(at.getHours())}:${pad(at.getMinutes())}:00${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`;
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
  if (filters.from?.length) params.set("user_id", filters.from.join(","));
  if (filters.in?.length) params.set("channel_id", filters.in.join(","));
  if (filters.noBots) params.set("exclude_bots", "true");
  if (filters.mine) params.set("involving_me", "true");
  if (filters.after) params.set("after", localMidnight(filters.after));
  if (filters.before) params.set("before", localMidnight(filters.before));
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

/** 一覧のキー。「すべて」ではメッセージと添付の ID が重なりうるので、種類も含める。 */
export function searchItemKey(item: SearchItem): string {
  return `${isFileItem(item) ? "file" : "message"}:${item.id}`;
}

/** 新しい順を保ったまま、前のページと重なった項目を除いてつなぐ。 */
export function appendSearchItems(current: readonly SearchItem[], next: readonly SearchItem[]) {
  const seen = new Set(current.map(searchItemKey));
  return [...current, ...next.filter((item) => !seen.has(searchItemKey(item)))];
}
