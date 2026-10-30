import { parseUtcTimestamp, type TimestampString } from "../timestamps";

/** 初期表示で読み込む期間。上方向のスクロールで過去へ広げる。 */
export const MESSAGE_WINDOW_MS = 30 * 24 * 60 * 60 * 1000;

/**
 * Electric が返す `YYYY-MM-DD HH:MM:SS.sss`（UTC）形式へそろえる。
 * live query の `gte` は文字列比較になるため、比較対象の形式を揃える。
 */
export function formatWindowTimestamp(value: number | Date): TimestampString {
  const date = typeof value === "number" ? new Date(value) : value;
  return date.toISOString().replace("T", " ").replace("Z", "");
}

/** 直近30日の開始時刻。 */
export function messageWindowStart(now: number = Date.now()): TimestampString {
  return formatWindowTimestamp(now - MESSAGE_WINDOW_MS);
}

/**
 * `since` を `oldest` より前へ広げる。すでに十分古ければ同じ値を返す。
 *
 * `-1ms` して、同じミリ秒に並ぶメッセージを取りこぼさない。文字列のまま
 * 比較すると Electric 形式（空白区切り）と `nowIso()`（`T` 区切り）が混ざる
 * ため、必ず時刻へ戻して比較する。
 */
export function expandWindowSince(
  since: TimestampString,
  oldest: TimestampString,
): TimestampString {
  const sinceMs = parseUtcTimestamp(since).getTime();
  const oldestMs = parseUtcTimestamp(oldest).getTime();
  if (!Number.isFinite(oldestMs)) {
    return since;
  }
  if (Number.isFinite(sinceMs) && oldestMs >= sinceMs) {
    return since;
  }
  return formatWindowTimestamp(oldestMs - 1);
}
