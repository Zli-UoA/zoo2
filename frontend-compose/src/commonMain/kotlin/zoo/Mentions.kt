package zoo

private const val mentionBoundary = """[\s\p{Zs}\u2028\u2029\uFEFF(\[{<（「『"'。、]"""
private const val mentionName = "[a-zA-Z0-9._-]+"
internal val mentionNamePattern = Regex(mentionName)
internal val mentionQueryPattern = Regex("""(?:^|$mentionBoundary)@([\p{L}\p{M}\p{N}._-]*)\z""")

internal expect fun normalizeMentionSearch(value: String): String

fun allowedMentionNames(mentions: EncryptedMentions, users: Map<String, Row>): Set<String> =
    mentions.userIds.mapNotNull { id -> users[id]?.text("user_name") }
        .filter { it.isNotEmpty() && !it.equals("channel", true) && !it.equals("here", true) }.toSet() +
        listOfNotNull("channel".takeIf { mentions.channel }, "here".takeIf { mentions.here })

fun resolvedMentionName(rawName: String, exactMatch: Boolean): String {
    val trimmed = rawName.trimEnd('.')
    return when {
        trimmed.equals("channel", true) || trimmed.equals("here", true) -> trimmed
        exactMatch || '@' in rawName -> rawName
        else -> trimmed
    }
}

fun isAllowedMention(username: String, names: Set<String>): Boolean =
    if (username.equals("channel", true) || username.equals("here", true)) username.lowercase() in names
    else username in names
