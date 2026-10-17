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

/** 本文をリンク化するための断片。`text` はそのまま、`link` は href として描く。 */
export type MessageTextSegment = { kind: "text" | "link"; value: string };

/**
 * `http(s)` URL の候補。`<` `>` と引用符・空白で終端し、`javascript:` などは
 * そもそも開始スキームが合わないため拾わない。`\b` で語中（`foohttps://`）を防ぐ。
 */
const URL_PATTERN = /\bhttps?:\/\/[^\s<>"'`]+/gi;

/** URL 末尾に紛れ込みやすい約物（日本語の句読点を含む）。 */
const TRAILING_PUNCTUATION = /[.,;:!?、。！？…]+$/;

/** 閉じ括弧 → 対応する開き括弧。URL 内で釣り合っていなければ文の約物とみなす。 */
const CLOSING_BRACKETS: Record<string, string> = {
  ")": "(",
  "]": "[",
  "}": "{",
  "）": "（",
  "】": "【",
  "》": "《",
  "」": "「",
  "』": "『",
  "〕": "〔",
};

function countOccurrences(value: string, char: string): number {
  let total = 0;
  for (const current of value) {
    if (current === char) {
      total += 1;
    }
  }
  return total;
}

/** 文末の約物を URL から落とす。閉じ括弧は対応する開き括弧が無いときだけ落とす。 */
function trimUrlEnd(candidate: string): string {
  let url = candidate.replace(TRAILING_PUNCTUATION, "");
  for (;;) {
    const last = url.at(-1);
    if (!last) {
      return url;
    }
    const opener = CLOSING_BRACKETS[last];
    if (!opener || countOccurrences(url, last) <= countOccurrences(url, opener)) {
      return url;
    }
    url = url.slice(0, -1);
  }
}

/**
 * 本文を素のテキストと URL の断片に分ける。文字列の連結だけで組み立てるため、
 * 描画側で React が値をエスケープし、`href` は常に `http(s)` で始まる。
 */
export function messageTextSegments(content: string): MessageTextSegment[] {
  const segments: MessageTextSegment[] = [];
  let cursor = 0;
  for (const match of content.matchAll(URL_PATTERN)) {
    const start = match.index ?? 0;
    const url = trimUrlEnd(match[0]);
    if (!url) {
      continue;
    }
    if (start > cursor) {
      segments.push({ kind: "text", value: content.slice(cursor, start) });
    }
    segments.push({ kind: "link", value: url });
    cursor = start + url.length;
  }
  if (cursor < content.length) {
    segments.push({ kind: "text", value: content.slice(cursor) });
  }
  return segments.length > 0 ? segments : [{ kind: "text", value: content }];
}

/**
 * スクリーンリーダーへ `aria-live` で読み上げる新着メッセージの要約。
 * 改行・連続空白は 1 つに畳み、長文は `maxLength` 文字（コードポイント単位）で切る。
 */
export function newMessageAnnouncement(message: FeedMessage, maxLength = 80): string {
  const collapsed = message.content.replace(/\s+/g, " ").trim();
  const characters = Array.from(collapsed);
  const excerpt =
    characters.length > maxLength ? `${characters.slice(0, maxLength).join("")}…` : collapsed;
  const author = authorLabel(message);
  return excerpt ? `新着メッセージ: ${author}「${excerpt}」` : `新着メッセージ: ${author}`;
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
