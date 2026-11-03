/** 返信数の集計に必要な最小限の行。 */
export interface ReplyParentRow {
  parent_id: string | null;
}

/**
 * 返信の `parent_id` を数えてルートメッセージごとの返信数にする。
 *
 * 旧キャッシュ・楽観更新用の補完。確定した投稿は表示用VIEWから届く
 * display_data.reply_count を優先する（返信と同じトランザクションで更新される）。
 * 論理削除された返信は呼び出し側のクエリで除外済み。
 */
export function countRepliesByParent(rows: ReplyParentRow[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const row of rows) {
    if (!row.parent_id) {
      continue;
    }
    counts.set(row.parent_id, (counts.get(row.parent_id) ?? 0) + 1);
  }
  return counts;
}
