import type { ChannelRow } from "../../lib/electric/types";

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
 * - アーカイブ済みも閲覧できるため一覧には残す。
 * - 並び順は変えない。
 */
export function filterDiscoverableChannels(
  channels: ChannelRow[],
  joinedChannelIds: ReadonlySet<string>,
  query: string,
): ChannelRow[] {
  const needle = query.trim().toLowerCase();
  return channels.filter((channel) => {
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
  "name-asc",
  "name-desc",
  "created-desc",
  "created-asc",
  "members-desc",
  "members-asc",
] as const;

export type ChannelSort = (typeof CHANNEL_SORTS)[number];

export const DEFAULT_CHANNEL_SORT: ChannelSort = "name-asc";

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
 * - 作成日時は `created_at`、メンバー数は `memberCounts`（未登録は 0）で比べる。
 */
export function sortChannels(
  channels: ChannelRow[],
  sort: ChannelSort,
  memberCounts: ReadonlyMap<string, number> = new Map(),
): ChannelRow[] {
  const direction = sort.endsWith("-desc") ? -1 : 1;
  const key = (channel: ChannelRow): string | number => {
    if (sort.startsWith("created")) return channel.created_at;
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
