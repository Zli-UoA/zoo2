import type { DisplayEmoji, MessageDisplayData } from "../../../lib/electric/types";

/** クライアントで表示する種類。ノート（`type=notes`）は画面がないので扱わない。 */
export const SEARCH_KINDS = ["messages", "files"] as const;
export type SearchKind = (typeof SEARCH_KINDS)[number];

export interface SearchRouteParams {
  q?: string;
  type?: SearchKind;
}

type SearchAuthor = NonNullable<MessageDisplayData["author"]>;

interface SearchItemBase {
  id: string;
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

/** `/search` の URL の検索パラメーター。既定値（空の語・messages）は URL に出さない。 */
export function parseSearchRouteParams(search: Record<string, unknown>): SearchRouteParams {
  // TanStack Router は `?q=123` を数値として読むので文字列へ戻す。
  const raw =
    typeof search.q === "string"
      ? search.q
      : typeof search.q === "number" || typeof search.q === "boolean"
        ? String(search.q)
        : "";
  const q = normalizeSearchQuery(raw);
  return {
    q: q || undefined,
    type: search.type === "files" ? "files" : undefined,
  };
}

/** 前後の空白を除き、連続する空白を 1 つにする。サーバーも空白で語を区切る。 */
export function normalizeSearchQuery(value: string): string {
  return value.trim().replace(/\s+/gu, " ");
}

export function searchKindOf(params: SearchRouteParams): SearchKind {
  return params.type ?? "messages";
}

export function searchRequestPath(query: string, kind: SearchKind, cursor: string | null): string {
  const params = new URLSearchParams({ q: query, type: kind });
  if (cursor) params.set("cursor", cursor);
  return `/api/search?${params}`;
}

export function isFileItem(item: SearchItem): item is SearchFileItem {
  return "file_name" in item;
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
