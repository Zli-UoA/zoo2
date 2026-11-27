package zoo

private const val mentionBoundary = """[\s\p{Zs}\u2028\u2029\uFEFF(\[{<（「『"'。、]"""
private const val mentionName = "[a-zA-Z0-9._-]+"
internal val mentionNamePattern = Regex(mentionName)
internal val mentionQueryPattern = Regex("""(?:^|$mentionBoundary)@([\p{L}\p{M}\p{N}._-]*)\z""")
private val mentionPattern = Regex("(?:^|$mentionBoundary)@($mentionName)")
private val mentionPunctuation = Regex("[.,!?:;。、！？]+$")

internal expect fun normalizeMentionSearch(value: String): String

data class MessageMentions(
    val userIds: List<String> = emptyList(),
    val channel: Boolean = false,
    val here: Boolean = false,
)

internal fun encryptedMentions(body: String, users: List<Row>): MessageMentions {
    val idsByName = users.associate { it.text("user_name") to it.text("id") }
    val userIds = mutableSetOf<String>()
    var channel = false
    var here = false
    for (match in mentionPattern.findAll(body)) {
        val raw = match.groupValues[1]
        val trimmed = raw.replace(mentionPunctuation, "")
        when (trimmed.lowercase()) {
            "channel" -> channel = true
            "here" -> here = true
            else -> (idsByName[raw] ?: idsByName[trimmed])?.takeIf { it.isNotEmpty() }?.let(userIds::add)
        }
    }
    return MessageMentions(userIds.sorted(), channel, here)
}

internal class MentionAllowlist(private val names: Set<String>, private val mentions: MessageMentions) {
    fun allows(token: String): Boolean {
        val raw = token.removePrefix("@")
        val trimmed = raw.replace(mentionPunctuation, "")
        return when (trimmed.lowercase()) {
            "channel" -> mentions.channel
            "here" -> mentions.here
            else -> raw in names || trimmed in names
        }
    }
}

internal fun mentionAllowlist(mentions: MessageMentions, users: List<Row>): MentionAllowlist {
    val ids = mentions.userIds.toSet()
    val names = users.asSequence()
        .filter { it.text("id") in ids }
        .map { it.text("user_name") }
        .filter { it.isNotEmpty() }
        .toSet()
    return MentionAllowlist(names, mentions)
}
