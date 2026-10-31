import type { ChannelRow } from "../../lib/electric/types";

/** サイドバーに出す、現在ユーザーが参加中のチャンネルだけを残す。 */
export function filterJoinedChannels(
  channels: ChannelRow[],
  joinedChannelIds: ReadonlySet<string>,
): ChannelRow[] {
  return channels.filter((channel) => joinedChannelIds.has(channel.id));
}

/** アーカイブ済みでなければ参加できる。公開チャンネルのみを対象にする。 */
export function isChannelJoinable(channel: ChannelRow): boolean {
  return !channel.is_private && channel.archive_at === null;
}

/**
 * 「チャンネルを探す」に出す候補を絞り込む。
 *
 * - 参加済みと非公開チャンネルは出さない。
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
    if (channel.is_private || joinedChannelIds.has(channel.id)) {
      return false;
    }
    return needle.length === 0 || channel.channel_name.toLowerCase().includes(needle);
  });
}
