/** サーバーと同じく、空白で区切った語（重複を除き、最大 5 個）。`backend/src/routes/search.rs` の `parse_terms`。 */
export const MAX_SEARCH_TERMS = 5;

export function searchTerms(query: string): string[] {
  const terms: string[] = [];
  for (const term of query.split(/\s+/u)) {
    if (term && !terms.includes(term)) terms.push(term);
  }
  return terms.slice(0, MAX_SEARCH_TERMS);
}

/** サーバーの `MAX_TERM_CHARS`。1 語の文字数（コードポイント）の上限。 */
export const MAX_SEARCH_TERM_CHARS = 100;

export type SearchQueryProblem = "tooManyTerms" | "termTooLong";

/**
 * サーバーの `parse_terms` が 400 で拒否する語を、送信前に見つける。語の長さを先に見るのもサーバーと同じ。
 * 拒否される語は再試行しても通らないので、理由を示して送らない。
 */
export function searchQueryProblem(query: string): SearchQueryProblem | null {
  const terms: string[] = [];
  for (const term of query.split(/\s+/u)) {
    if (!term) continue;
    // サーバーの `chars().count()` と同じく、書記素ではなくコードポイントで数える。
    if (Array.from(term).length > MAX_SEARCH_TERM_CHARS) return "termTooLong";
    if (!terms.includes(term)) terms.push(term);
  }
  return terms.length > MAX_SEARCH_TERMS ? "tooManyTerms" : null;
}

export interface HighlightSegment {
  text: string;
  match: boolean;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

/**
 * `text` を、検索語に当たった部分とそれ以外に分ける。大文字と小文字は区別しない（サーバーの `lower()` と同じ）。
 * 重なる語は長い方を優先し、当たらなければ 1 つの区間をそのまま返す。
 */
export function highlightSegments(text: string, terms: readonly string[]): HighlightSegment[] {
  const usable = terms.filter(Boolean);
  if (!text || !usable.length) return [{ text, match: false }];
  const pattern = new RegExp(
    [...usable]
      .sort((a, b) => b.length - a.length)
      .map(escapeRegExp)
      .join("|"),
    "giu",
  );
  const segments: HighlightSegment[] = [];
  let last = 0;
  for (const found of text.matchAll(pattern)) {
    const start = found.index;
    if (start > last) segments.push({ text: text.slice(last, start), match: false });
    segments.push({ text: found[0], match: true });
    last = start + found[0].length;
  }
  if (last < text.length) segments.push({ text: text.slice(last), match: false });
  return segments.length ? segments : [{ text, match: false }];
}

/** 抜粋で、最初の一致の前に残す文字数（コードポイント）。 */
export const SNIPPET_CONTEXT_BEFORE = 16;

/**
 * 抜粋の最初の一致より前を、`keep` 文字まで縮めて `…` を付ける。サーバーの抜粋は一致の 40 文字前から
 * 始まるので、狭い画面の 3 行では一致が表示範囲の外に出ることがある。一致がなければそのまま返す。
 */
export function focusSnippet(
  text: string,
  terms: readonly string[],
  keep = SNIPPET_CONTEXT_BEFORE,
): string {
  const segments = highlightSegments(text, terms);
  const first = segments.findIndex((segment) => segment.match);
  if (first <= 0) return text;
  const before = Array.from(
    segments
      .slice(0, first)
      .map((segment) => segment.text)
      .join(""),
  );
  if (before.length <= keep + 1) return text;
  const rest = segments
    .slice(first)
    .map((segment) => segment.text)
    .join("");
  return `…${before.slice(-keep).join("")}${rest}`;
}
