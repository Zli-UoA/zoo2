import type { ChannelLeaveRow, ChannelRow } from "../../lib/electric/types";
import { parseUtcTimestamp } from "../../lib/timestamps";

/** サイドバーに出す、現在ユーザーが参加中でアーカイブされていないチャンネルだけを残す。 */
export function filterJoinedChannels(
  channels: ChannelRow[],
  joinedChannelIds: ReadonlySet<string>,
): ChannelRow[] {
  return channels.filter((channel) => joinedChannelIds.has(channel.id) && !channel.archive_at);
}

/** アーカイブ済みでなければ参加できる。公開チャンネルのみを対象にする。 */
export function isChannelJoinable(channel: ChannelRow): boolean {
  return !channel.is_private && channel.archive_at === null;
}

/**
 * 「チャンネルを探す」に出す候補を絞り込む。
 *
 * - 未参加の非公開チャンネルと、参加済みでアーカイブされていないチャンネルは出さない。
 * - チャンネル名の部分一致（大文字小文字と前後の空白を無視）。
 * - アーカイブ済みは `includeArchived` のときだけ出す（既定は出さない）。
 * - 並び順は変えない。
 */
export function filterDiscoverableChannels(
  channels: ChannelRow[],
  joinedChannelIds: ReadonlySet<string>,
  query: string,
  options: { includeArchived?: boolean } = {},
): ChannelRow[] {
  const needle = query.trim().toLowerCase();
  return channels.filter((channel) => {
    if (channel.archive_at && !options.includeArchived) {
      return false;
    }
    const joined = joinedChannelIds.has(channel.id);
    // 参加済みのアーカイブ済みは、サイドバーに出さない代わりに非公開でもここに残す。
    if (channel.is_private && !(channel.archive_at && joined)) {
      return false;
    }
    if (!channel.archive_at && joined) {
      return false;
    }
    return needle.length === 0 || channel.channel_name.toLowerCase().includes(needle);
  });
}

export const CHANNEL_SORTS = [
  "left-desc",
  "left-asc",
  "name-asc",
  "name-desc",
  "created-desc",
  "created-asc",
  "members-desc",
  "members-asc",
] as const;

export type ChannelSort = (typeof CHANNEL_SORTS)[number];

/** 既定はチャンネル作成日の新しい順。 */
export const DEFAULT_CHANNEL_SORT: ChannelSort = "created-desc";

export function isChannelSort(value: unknown): value is ChannelSort {
  return CHANNEL_SORTS.includes(value as ChannelSort);
}

function compareNames(a: ChannelRow, b: ChannelRow): number {
  return a.channel_name.localeCompare(b.channel_name, undefined, { sensitivity: "base" });
}

/**
 * 「チャンネルを探す」の並び順。入力は変更しない。
 *
 * - 同順位（同じ作成日時・同じメンバー数）は名前の昇順で安定させる。
 * - 作成日時は `created_at`、メンバー数は `memberCounts`（未登録は 0）、退出日は `leaveDates` で比べる。
 */
export function sortChannels(
  channels: ChannelRow[],
  sort: ChannelSort,
  memberCounts: ReadonlyMap<string, number> = new Map(),
  leaveDates: ReadonlyMap<string, string> = new Map(),
): ChannelRow[] {
  if (sort === "left-desc" || sort === "left-asc") {
    return [...channels].sort(
      compareByLeaveDate(leaveDates, sort === "left-desc" ? "newest" : "oldest"),
    );
  }
  const direction = sort.endsWith("-desc") ? -1 : 1;
  const key = (channel: ChannelRow): string | number => {
    if (sort.startsWith("created")) {
      // 空白区切りと ISO 形式が混ざっても時刻順になるよう、数値にして比べる。
      const time = parseUtcTimestamp(channel.created_at).getTime();
      return Number.isNaN(time) ? 0 : time;
    }
    if (sort.startsWith("members")) return memberCounts.get(channel.id) ?? 0;
    return 0;
  };
  return [...channels].sort((a, b) => {
    if (sort.startsWith("name")) {
      return direction * compareNames(a, b);
    }
    const ka = key(a);
    const kb = key(b);
    const order = ka < kb ? -1 : ka > kb ? 1 : 0;
    return order !== 0 ? direction * order : compareNames(a, b);
  });
}

/**
 * 退出日の対応表（channel_id → 退出日）を作る。
 *
 * 再参加して今は参加中のチャンネルは、前回の退出日が残っていても含めない
 * （退出済みのチャンネルだけを退出日で並べるため）。
 */
export function buildLeaveDateMap(
  leaves: readonly ChannelLeaveRow[],
  joinedChannelIds: ReadonlySet<string>,
): Map<string, string> {
  const map = new Map<string, string>();
  for (const leave of leaves) {
    if (!joinedChannelIds.has(leave.channel_id)) {
      map.set(leave.channel_id, leave.left_at);
    }
  }
  return map;
}

function leaveTime(leaveDates: ReadonlyMap<string, string>, channelId: string): number | null {
  const leftAt = leaveDates.get(channelId);
  if (leftAt === undefined) return null;
  const time = parseUtcTimestamp(leftAt).getTime();
  return Number.isNaN(time) ? null : time;
}

/**
 * 退出日で比較する関数を返す。`newest` は新しい退出が先、`oldest` は古い退出が先。
 *
 * 退出日のないチャンネル（一度も自分で退出していない、キックされた、機能の導入前に退出した）は、
 * どちらの向きでも最後に置く。同じ退出日どうしと退出日のないものどうしは、名前順にする。
 */
export function compareByLeaveDate(
  leaveDates: ReadonlyMap<string, string>,
  direction: "newest" | "oldest",
): (a: ChannelRow, b: ChannelRow) => number {
  return (a, b) => {
    const timeA = leaveTime(leaveDates, a.id);
    const timeB = leaveTime(leaveDates, b.id);
    if (timeA !== null && timeB !== null && timeA !== timeB) {
      return direction === "newest" ? timeB - timeA : timeA - timeB;
    }
    if ((timeA === null) !== (timeB === null)) {
      return timeA === null ? 1 : -1;
    }
    return a.channel_name.localeCompare(b.channel_name);
  };
}
