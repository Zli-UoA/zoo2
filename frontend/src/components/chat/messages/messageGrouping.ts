import { parseUtcTimestamp } from "../../../lib/timestamps";
import { isSameLocalDay, type FeedMessage } from "./messageFormat";

/** 同一投稿者の連投をまとめる時間のしきい値。 */
export const GROUP_GAP_MS = 5 * 60 * 1000;

export interface MessageItem {
  message: FeedMessage;
  /** 連投のかたまりの先頭（アバター・投稿者名を出す）。 */
  isGroupStart: boolean;
  /** 連投のかたまりの末尾（下余白を広げる）。 */
  isGroupEnd: boolean;
  /** 直前の行と日付が変わるため、上に日付区切りを描く。 */
  showDateSeparator: boolean;
}

function withinGroup(earlier: FeedMessage, later: FeedMessage, groupGapMs: number): boolean {
  const gap =
    parseUtcTimestamp(later.created_at).getTime() - parseUtcTimestamp(earlier.created_at).getTime();
  return gap >= 0 && gap <= groupGapMs;
}

/**
 * メッセージ列を表示用の行に変換する。
 *
 * - 日付が変わった行には区切りを出す。
 * - 同じ投稿者の投稿が `groupGapMs` 以内に続く場合は連投としてまとめる
 *   （投稿者名とアバターは先頭だけ）。
 */
export function buildMessageItems(
  messages: FeedMessage[],
  groupGapMs: number = GROUP_GAP_MS,
): MessageItem[] {
  return messages.map((message, index) => {
    const previous = messages[index - 1];
    const next = messages[index + 1];
    const messageDate = parseUtcTimestamp(message.created_at);

    const showDateSeparator =
      !previous || !isSameLocalDay(parseUtcTimestamp(previous.created_at), messageDate);

    const continuesFromPrevious =
      !!previous &&
      !showDateSeparator &&
      previous.user_id === message.user_id &&
      withinGroup(previous, message, groupGapMs);

    const continuesToNext =
      !!next &&
      next.user_id === message.user_id &&
      isSameLocalDay(parseUtcTimestamp(next.created_at), messageDate) &&
      withinGroup(message, next, groupGapMs);

    return {
      message,
      isGroupStart: !continuesFromPrevious,
      isGroupEnd: !continuesToNext,
      showDateSeparator,
    };
  });
}
