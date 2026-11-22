/** サーバーと同じく、空白で区切った語（重複を除き、最大 5 個）。`backend/src/routes/search.rs` の `parse_terms`。 */
export const MAX_SEARCH_TERMS = 5;

export function searchTerms(query: string): string[] {
  const terms: string[] = [];
  for (const term of query.split(/\s+/u)) {
    if (term && !terms.includes(term)) terms.push(term);
  }
  return terms.slice(0, MAX_SEARCH_TERMS);
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
