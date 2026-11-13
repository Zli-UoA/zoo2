import type { ChannelRow } from "../../lib/electric/types";

/** サイドバーに出す、現在ユーザーが参加中でアーカイブされていないチャンネルだけを残す。 */
export function filterJoinedChannels(
  channels: ChannelRow[],
  joinedChannelIds: ReadonlySet<string>,
): ChannelRow[] {
  return channels.filter(
    (channel) => joinedChannelIds.has(channel.id) && channel.archive_at === null,
  );
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
    if (channel.is_private && !(channel.archive_at !== null && joined)) {
      return false;
    }
    if (channel.archive_at === null && joined) {
      return false;
    }
    return needle.length === 0 || channel.channel_name.toLowerCase().includes(needle);
  });
}
