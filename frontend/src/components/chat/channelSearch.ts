import type { ChannelRow } from "../../lib/electric/types";

/**
 * チャンネル名で一覧を絞り込む。前後の空白は無視し、大文字小文字を区別しない。
 * 並び順は変えず、入力が空なら元の配列をそのまま返す。
 */
export function filterChannels(channels: ChannelRow[], query: string): ChannelRow[] {
  const needle = query.trim().toLowerCase();
  if (!needle) {
    return channels;
  }
  return channels.filter((channel) => channel.channel_name.toLowerCase().includes(needle));
}
