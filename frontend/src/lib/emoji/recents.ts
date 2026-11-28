/**
 * リアクションの識別値を最近の履歴用に作る。
 * 画像 ID のある絵文字は名前だけだと同名の別絵文字と衝突するため、ID と名前の両方を残す。
 */
export function recentReactionValue(reaction: string, emojiId?: string, local = false): string {
  if (!emojiId && !local) return reaction;
  const name = reaction.startsWith(":") && reaction.endsWith(":") ? reaction : `:${reaction}:`;
  // ローカルのワークスペース絵文字は名前で解決できるので、チャットでも使える短縮形を残す。
  if (local) return name;
  return `apub:${emojiId}:${name}`;
}
