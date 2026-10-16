import { normalizeUtcTimestamp, parseUtcTimestamp } from "../../../lib/timestamps";

/**
 * `messageFeedQuery` の select 結果。`users` の結合がまだ届いていない行では
 * `user_name` / `display_name` が undefined になる（左結合）。
 */
export interface FeedMessage {
  id: string;
  user_id: string;
  content: string;
  created_at: string;
  user_name?: string | null;
  display_name?: string | null;
}

const WEEKDAYS = ["日", "月", "火", "水", "木", "金", "土"];

function pad(value: number): string {
  return value.toString().padStart(2, "0");
}

/** 投稿者名。表示名 → ユーザー名 → id の順にフォールバックする。 */
export function authorLabel(message: FeedMessage): string {
  return message.display_name || message.user_name || message.user_id;
}

/** `Date` を時計表示（HH:mm）に整える。 */
export function formatClock(date: Date): string {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Electric の naive UTC 文字列を端末ローカルの HH:mm に変換する。 */
export function formatTime(value: string): string {
  return formatClock(parseUtcTimestamp(value));
}

/** タイトル属性向けの完全な日時。 */
export function fullTimestamp(value: string): string {
  return new Date(normalizeUtcTimestamp(value)).toLocaleString("ja-JP");
}

/** 端末ローカルの暦日が同じか。 */
export function isSameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** 端末ローカルの暦日キー（YYYY-MM-DD）。並べ替えや比較に使う。 */
export function localDayKey(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/**
 * 日付区切りのラベル。今日/昨日は相対表記、それ以前は「2026年6月10日(水)」。
 * `now` を渡せるようにしてテストを端末タイムゾーンから切り離す。
 */
export function formatRelativeDay(date: Date, now: Date): string {
  if (isSameLocalDay(date, now)) {
    return "今日";
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (isSameLocalDay(date, yesterday)) {
    return "昨日";
  }
  return `${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日(${
    WEEKDAYS[date.getDay()]
  })`;
}

/** メッセージの日付区切りラベル。 */
export function formatDateSeparator(value: string, now: Date = new Date()): string {
  return formatRelativeDay(parseUtcTimestamp(value), now);
}
