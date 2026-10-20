/** 返信数の集計に必要な最小限の行。 */
export interface ReplyParentRow {
  parent_id: string | null;
}

/**
 * 返信の `parent_id` を数えてルートメッセージごとの返信数にする。
 *
 * サーバに `reply_count` は持たせず（返信 INSERT と集計 UPDATE が別 txid になり
 * クライアントの購読が揺れるため）、同じ messages Shape からクライアントで数える。
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
