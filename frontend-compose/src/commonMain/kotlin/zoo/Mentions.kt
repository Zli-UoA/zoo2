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


private val messageMentionPattern = Regex("(?:^|$mentionBoundary)@($mentionName)")

fun messageMentionsUser(body: String, username: String, usernames: Set<String>): Boolean =
    messageMentionPattern.findAll(body).any { match ->
        val raw = match.groupValues[1]
        val name = if (raw in usernames) raw else raw.trimEnd('.')
        name.equals("channel", true) || name.equals("here", true) || username.isNotEmpty() && username == name
    }
