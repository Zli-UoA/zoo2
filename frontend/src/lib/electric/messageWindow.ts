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

/** 上端に来たときに、窓を過去へ広げる行数 */
export const MESSAGE_PAGE_SIZE = 100;

/**
 * 端末にある行の時刻 (`timestamps`) をもとに、`since` を過去へ 1 ページ分広げる。
 *
 * 表示中の行はすべて `since` 以降なので、表示中の最古の行の時刻を `expandWindowSince` に渡しても窓は
 * 広がらない。そこで、`since` より前にある行のうち新しい方から `pageSize` 件目の時刻まで広げる
 * (それより少なければ、いちばん古い行まで)。`since` より前の行がなければ、同じ値を返す。
 */
export function olderWindowSince(
  since: TimestampString,
  timestamps: Iterable<TimestampString>,
  pageSize: number = MESSAGE_PAGE_SIZE,
): TimestampString {
  const sinceMs = parseUtcTimestamp(since).getTime();
  const older: number[] = [];
  for (const timestamp of timestamps) {
    const ms = parseUtcTimestamp(timestamp).getTime();
    if (Number.isFinite(ms) && (!Number.isFinite(sinceMs) || ms < sinceMs)) {
      older.push(ms);
    }
  }
  if (older.length === 0) {
    return since;
  }
  older.sort((a, b) => b - a);
  const target = older[Math.min(Math.max(pageSize, 1), older.length) - 1]!;
  return formatWindowTimestamp(target - 1);
}
