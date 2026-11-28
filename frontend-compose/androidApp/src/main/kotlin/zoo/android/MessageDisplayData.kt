package zoo.android

import zoo.Row
import zoo.array
import zoo.nested
import zoo.rows
import zoo.text

/**
 * メッセージ行の添付・リアクションは、サーバーの表示用 VIEW が行の `display_data` に同じトランザクションで書く。
 * チャンネル全件の `attachments` / `message_reactions` Shape は購読しない（Web と同じ）。
 */
internal fun displayAttachments(row: Row): List<Row> = row.nested("display_data").array("attachments").rows()

/**
 * ActivityPub 由来のリアクション（`emoji` が `apub:<image_id>:<name>`、`user_id` が相手サーバーの actor）は、
 * Android にはその表示も操作もないので除く。バッジにして押すと、その文字列を送ってしまう。
 */
internal fun displayReactions(row: Row): List<Row> =
    row.nested("display_data").array("reactions").rows().filterNot { it.text("emoji").startsWith("apub:") }

/** 取得済みの行の中で、[target] が `created_at`・`id` の並びで [oldest] 以降（古い側の端を含む）か。 */
internal fun pageCovers(oldest: Pair<String, String>?, createdAt: String, id: String): Boolean {
    if (oldest == null) return false
    val byTime = oldest.first.compareTo(createdAt)
    return byTime < 0 || (byTime == 0 && oldest.second <= id)
}
