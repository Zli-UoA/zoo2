package zoo

private const val mentionBoundary = """[\s\p{Zs}\u2028\u2029\uFEFF(\[{<（「『"'。、]"""
private const val mentionName = "[a-zA-Z0-9._-]+"
internal val mentionNamePattern = Regex(mentionName)
internal val mentionQueryPattern = Regex("""(?:^|$mentionBoundary)@([\p{L}\p{M}\p{N}._-]*)\z""")
private val mentionPunctuation = Regex("[.,!?:;。、！？]+$")

internal expect fun normalizeMentionSearch(value: String): String

fun allowedMentionNames(mentions: EncryptedMentions, users: Map<String, Row>): Set<String> =
    mentions.userIds.mapNotNull { id -> users[id]?.text("user_name") }
        .filter { it.isNotEmpty() && !it.equals("channel", true) && !it.equals("here", true) }.toSet() +
        listOfNotNull("channel".takeIf { mentions.channel }, "here".takeIf { mentions.here })

fun isAllowedMention(token: String, names: Set<String>): Boolean {
    val raw = token.removePrefix("@")
    val trimmed = raw.replace(mentionPunctuation, "")
    return when (trimmed.lowercase()) {
        "channel" -> "channel" in names
        "here" -> "here" in names
        else -> raw in names || trimmed in names
    }
}
