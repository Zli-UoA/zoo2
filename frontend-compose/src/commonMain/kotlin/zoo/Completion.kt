package zoo

data class CompletionRange(
    val start: Int,
    val end: Int,
    val query: String,
    val mfm: Boolean,
    val kind: String = if (mfm) "mfm" else "mention",
)

data class CompletionOption(val name: String, val label: String, val appId: String? = null)

val mfmFunctions =
    listOf(
        "tada",
        "jelly",
        "twitch",
        "shake",
        "spin",
        "jump",
        "bounce",
        "flip",
        "x2",
        "x3",
        "x4",
        "scale",
        "position",
        "fg",
        "bg",
        "border",
        "font",
        "blur",
        "rainbow",
        "sparkle",
        "rotate",
        "ruby",
        "unixtime",
        "clickable",
    )
private val mfmTemplates =
    mapOf(
        "font" to "font.serif",
        "scale" to "scale.x=2,y=2",
        "position" to "position.x=1,y=0",
        "fg" to "fg.color=ff0000",
        "bg" to "bg.color=ffff00",
        "border" to "border.width=1,style=solid",
        "rotate" to "rotate.deg=90",
        "clickable" to "clickable.ev=click",
    )

fun completionAt(value: String, caret: Int): CompletionRange? {
    if (caret !in 0..value.length) return null
    val prefix = value.take(caret)
    val inCode = insideCode(value, caret)
    Regex("^/([a-zA-Z0-9_-]*)$").find(prefix)?.let {
        return CompletionRange(0, caret, it.groupValues[1].lowercase(), false, "command")
    }
    var channelStart = caret
    while (
        channelStart > 0 &&
            !value[channelStart - 1].isWhitespace() &&
            value[channelStart - 1] !in "#.,!?<>()[]{}「」『』、。！？"
    ) channelStart--
    if (
        !inCode && channelStart > 0 &&
            value[channelStart - 1] == '#' &&
            (channelStart == 1 ||
                value[channelStart - 2].isWhitespace() ||
                value[channelStart - 2] in "([{（「『")
    )
        return CompletionRange(
            channelStart - 1,
            caret,
            normalizeQuery(value.substring(channelStart, caret)),
            false,
            "channel",
        )
    val mfm = Regex("\\$\\[?([a-zA-Z0-9]*)$").find(prefix)
    if (!inCode && mfm != null && !Regex("https?://[^\\s]*$").containsMatchIn(prefix)) {
        val escapes = prefix.take(mfm.range.first).takeLastWhile { it == '\\' }.length
        val code =
            prefix.take(mfm.range.first).count { it == '`' } % 2 == 1 ||
                (prefix.take(mfm.range.first).split("```").size - 1) % 2 == 1
        if (escapes % 2 == 0 && !code)
            return CompletionRange(mfm.range.first, caret, mfm.groupValues[1].lowercase(), true)
    }
    val mention = mentionQueryPattern.find(prefix) ?: return null
    val query = mention.groupValues[1]
    return CompletionRange(caret - query.length - 1, caret, normalizeMentionSearch(query), false)
}

fun completionOptions(
    range: CompletionRange?,
    users: List<Row>,
    memberIds: Set<String>,
    channels: List<Row> = emptyList(),
    commands: List<Row> = emptyList(),
): List<CompletionOption> {
    if (range == null) return emptyList()
    if (range.kind == "command")
        return commands
            .filter { it.text("name").startsWith(range.query, true) }
            .map { CompletionOption(it.text("name"), it.text("app_name"), it.text("app_id")) }
    if (range.kind == "channel")
        return visibleChannels(channels)
            .filter {
                !it.flag("is_dm") &&
                    normalizeQuery(it.text("channel_name")).contains(range.query, true)
            }
            .take(20)
            .map { CompletionOption(it.text("channel_name"), "") }
    if (range.mfm)
        return mfmFunctions
            .filter { it.startsWith(range.query) }
            .map { CompletionOption(it, "mfm.functions.$it") }
    val search = normalizeMentionSearch(range.query)
    return (listOf(CompletionOption("channel", "@channel"), CompletionOption("here", "@here")) +
            users
                .filter {
                    it.text("id") in memberIds &&
                        // Slack から取り込んだ未連携のプレースホルダなど、無効化済みのユーザーは選ばせない
                        !it.flag("is_deactivated") &&
                        (!it.flag("is_bot") || it.flag("bot_mentionable")) &&
                        mentionNamePattern.matches(it.text("user_name")) &&
                        it.text("user_name").lowercase() !in setOf("channel", "here")
                }
                .map { CompletionOption(it.text("user_name"), author(it)) })
        .filter { normalizeMentionSearch(it.name).contains(search) || normalizeMentionSearch(it.label).contains(search) }
        .take(8)
}

fun insertCompletion(value: String, range: CompletionRange, name: String): Pair<String, Int> {
    if (range.kind in listOf("channel", "command")) {
        val token = (if (range.kind == "channel") "#" else "/") + name
        val tail =
            value.drop(range.end).dropWhile { !it.isWhitespace() && it !in ".,!?<>()[]{}「」『』、。！？" }
        return value.take(range.start) + token + (if (tail.startsWith(' ')) "" else " ") + tail to
            range.start + token.length + 1
    }
    val insert = if (range.mfm) "$[${mfmTemplates[name] ?: name} ]" else "@$name "
    return (value.take(range.start) + insert + value.drop(range.end)) to
        (range.start + insert.length - if (range.mfm) 1 else 0)
}

fun insideCode(value: String, position: Int): Boolean {
    var delimiter = 0
    Regex("(?<!\\\\)`+").findAll(value.take(position)).forEach {
        val length = it.value.length
        if (delimiter == 0) delimiter = length
        else if (length == delimiter || delimiter >= 3 && length >= delimiter) delimiter = 0
    }
    return delimiter != 0
}

fun normalizeQuery(value: String): String =
    value
        .map {
            when (it.code) {
                in 0xff01..0xff5e -> (it.code - 0xfee0).toChar()
                0x3000 -> ' '
                else -> it
            }
        }
        .joinToString("")
        .lowercase()
