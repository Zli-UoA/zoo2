/**
 * 現在時刻を UTC の ISO 8601 文字列（`YYYY-MM-DDTHH:MM:SS.sssZ`）で返す。
 *
 * 楽観行の `created_at` などに使う書き込み側の時刻ヘルパ。Electric が配信する
 * `YYYY-MM-DD HH:MM:SS.sss` は `lib/timestamps.ts` の `normalizeUtcTimestamp` で
 * 同じ UTC 形式に正規化されるため、両者は同じ基準で比較できる。
 */
export function nowIso(): string {
  return new Date().toISOString();
}
