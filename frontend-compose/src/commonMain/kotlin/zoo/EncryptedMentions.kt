package zoo

/**
 * 暗号化チャンネルの送信に付ける平文の `mentions`（docs/e2ee.md の「メッセージ」）。
 *
 * サーバーは暗号化した本文を読めないので、平文チャンネルでサーバーが本文から作るのと同じ規則
 * （`backend/src/notifications.rs` の `extract_mentions`）で、送信側が作る。React の
 * `components/chat/messages/mentions.ts` の `encryptedMentions`、`client-types` の
 * `mentions::encrypted_mentions` と同じ。
 */
data class EncryptedMentions(
    /** メンションしたユーザー ID（昇順、重複なし）。 */
    val userIds: List<String> = emptyList(),
    val channel: Boolean = false,
    val here: Boolean = false,
)

/** ユーザー名の末尾から外す句読点（`backend/src/notifications.rs` の `PUNCTUATION` と同じ）。 */
private val MENTION_PUNCTUATION = setOf('.', ',', '!', '?', ':', ';', '。', '、', '！', '？')

/** `@` の直前に来てよい文字（行頭のほか）。 */
private fun isMentionBoundary(prev: Char): Boolean =
    prev.isWhitespace() || prev in "([{<（「『\"'。、"

private fun isMentionNameChar(c: Char): Boolean =
    (c in 'a'..'z') || (c in 'A'..'Z') || (c in '0'..'9') || c == '.' || c == '-' || c == '_'

/**
 * 本文の `@name`、`@channel`、`@here` から `mentions` を作る。`users` は `(ユーザー ID, ユーザー名)`。
 * ユーザー名は、語のまま、なければ末尾の句読点を外して引く。引けない名前は入れない。
 */
fun encryptedMentions(content: String, users: Iterable<Pair<String, String>>): EncryptedMentions {
    val byName = users.associate { (id, name) -> name to id }
    val userIds = mutableSetOf<String>()
    var channel = false
    var here = false
    var i = 0
    while (i < content.length) {
        if (content[i] == '@' && (i == 0 || isMentionBoundary(content[i - 1]))) {
            val start = i + 1
            var end = start
            while (end < content.length && isMentionNameChar(content[end])) end++
            if (end > start) {
                val raw = content.substring(start, end)
                val trimmed = raw.trimEnd { it in MENTION_PUNCTUATION }
                when {
                    trimmed.equals("channel", ignoreCase = true) -> channel = true
                    trimmed.equals("here", ignoreCase = true) -> here = true
                    else -> (byName[raw] ?: byName[trimmed])?.let { userIds += it }
                }
                i = end
                continue
            }
        }
        i++
    }
    return EncryptedMentions(userIds.sorted(), channel, here)
}
