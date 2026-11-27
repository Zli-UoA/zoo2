package zoo

/** 要約の最大文字数（コードポイント単位）。画面側でも行数で切るので、これは保険の上限（Web と同じ）。 */
const val EXCERPT_MAX_LENGTH = 200

private const val EXCERPT_MAX_DEPTH = 16

private val whitespace = Regex("\\s+")

/**
 * メッセージ本文を、埋め込みカードに出す短い素の文字列にする（Web の `messageExcerpt`）。装飾・リンク・
 * コードの記法を外し、改行や連続する空白を 1 つにまとめ、長いときは切って `…` を付ける。
 */
fun messageExcerpt(
    content: String, maxLength: Int = EXCERPT_MAX_LENGTH, customEmojis: Set<String> = emptySet(),
): String {
    val text = excerptText(content, customEmojis)
    val collapsed = text.replace(whitespace, " ").trim()
    if (collapsed.codePointCount() <= maxLength) return collapsed
    var end = 0
    repeat(maxLength) {
        end += if (collapsed[end].isHighSurrogate() && collapsed.getOrNull(end + 1)?.isLowSurrogate() == true) 2 else 1
    }
    return collapsed.substring(0, end).trimEnd() + "…"
}

/** Whether knowing another custom emoji could preserve text currently lost by the excerpt parser. */
fun messageExcerptNeedsWorkspaceEmoji(content: String, customEmojis: Set<String>): Boolean {
    if (':' !in content) return false
    val probe = ExcerptEmojiProbe(customEmojis)
    excerptText(content, customEmojis, probe)
    return probe.needsCatalog
}

private fun excerptText(
    content: String, customEmojis: Set<String>, probe: ExcerptEmojiProbe? = null,
): String = buildString {
    messageHeadingBlocks(content).forEach { block ->
        appendBlocks(block.content, 0, customEmojis, probe)
        append(' ')
    }
}

private class ExcerptEmojiProbe(private val known: Set<String>) {
    var needsCatalog = false
        private set

    fun candidates(source: String): Iterator<MatchResult> =
        if (needsCatalog || ':' !in source) emptyList<MatchResult>().iterator()
        else customEmojiMatches(source).iterator()

    fun inspect(
        source: String, slices: List<InlineSlice>, first: Int, match: MatchResult,
        prefix: CharSequence, ruby: Boolean = false,
    ) {
        if (needsCatalog || match.value in known || !isWorkspaceEmojiCandidate(match.value)) return
        val start = match.range.first
        val end = match.range.last + 1
        val initial = slices[first]
        // Nested markup is inspected as it is rendered. The parser does not visit literals again.
        if (initial.start < start && initial.token.kind !in setOf("text", "emoji")) return
        if (initial.start == start && initial.end == end && initial.token.kind == "emoji") return
        var previous = start - 1
        while (previous >= initial.start && source[previous] == '\\') previous--
        if ((start - previous - 1) % 2 != 0) return

        var changesText = false
        var startsBeforeKnown = false
        for (index in first..slices.lastIndex) {
            val slice = slices[index]
            if (slice.start >= end) break
            if (slice.token.kind == "accountEmoji" || (slice.token.kind == "emoji" && slice.token.value in known)) {
                if (slice.start <= start) return
                startsBeforeKnown = true
            } else if (slice.token.kind !in setOf("text", "emoji")) {
                changesText = true
            } else if (slice.token.kind == "text") {
                val overlapEnd = minOf(end, slice.end)
                changesText = changesText || (maxOf(start, slice.start) until overlapEnd - 1).any {
                    source[it] == '\\' && source[it + 1] in inlineEscapes
                }
            }
        }
        // In a time such as 12:30:猫:, an existing image owns the last colon of the time.
        val sharesTimeSeparator = startsBeforeKnown && source.getOrNull(start - 1)?.isDigit() == true &&
            (start + 1 until end - 1).all { source[it].isDigit() }
        if (!changesText && (!ruby || sharesTimeSeparator)) return
        // Recognizing an alias can move the ruby boundary back into the displayed prefix.
        val plainPrefix = if (ruby) "" else source.subSequence(initial.start, start)
        needsCatalog = excerptHasRoom(prefix, plainPrefix, includeEllipsisChange = ruby)
    }

    fun inspectRuby(source: String, slices: List<InlineSlice>, output: CharSequence) {
        val matches = candidates(source)
        var match = matches.nextOrNull()
        for ((index, slice) in slices.withIndex()) {
            while (match != null && match.range.first < slice.end) {
                inspect(source, slices, index, match, output, ruby = true)
                if (needsCatalog) return
                match = matches.nextOrNull()
            }
            if (match == null) break
        }
    }
}

private fun <T> Iterator<T>.nextOrNull(): T? = if (hasNext()) next() else null

private fun excerptHasRoom(
    output: CharSequence, sourcePrefix: CharSequence, includeEllipsisChange: Boolean,
): Boolean {
    var length = 0
    var space = false
    fun count(part: CharSequence, unescape: Boolean): Boolean {
        var index = 0
        while (index < part.length) {
            if (unescape && part[index] == '\\' && index + 1 < part.length && part[index + 1] in inlineEscapes) index++
            val char = part[index++]
            if (char.isWhitespace()) {
                if (length > 0) space = true
            } else if (!char.isLowSurrogate()) {
                if (space) length++
                space = false
                if (++length > EXCERPT_MAX_LENGTH) return false
            }
        }
        return true
    }
    return count(output, unescape = false) && count(sourcePrefix, unescape = true) &&
        (length < EXCERPT_MAX_LENGTH || includeEllipsisChange)
}

/** ブロック（コード・中央寄せ・引用）の記法を外す。前後に空白を置いて、前後の文とつながらないようにする。 */
private fun StringBuilder.appendBlocks(body: String, depth: Int, customEmojis: Set<String>, probe: ExcerptEmojiProbe?) {
    if (depth >= EXCERPT_MAX_DEPTH) {
        append(body)
        return
    }
    richSegments(body).forEach { (source, block) ->
        when {
            block && source.startsWith("```") -> append(' ').append(mfmCodeBlock(source).body).append(' ')
            block && source.startsWith("<center>") -> {
                append(' ')
                appendBlocks(source.removePrefix("<center>").removeSuffix("</center>"), depth + 1, customEmojis, probe)
                append(' ')
            }
            block && source.startsWith(">") -> source.lines().forEach { line ->
                append(' ')
                appendBlocks(line.removePrefix(">").trimStart(), depth + 1, customEmojis, probe)
            }
            else -> appendTokens(source, depth, customEmojis, probe)
        }
    }
}

private fun StringBuilder.appendTokens(source: String, depth: Int, customEmojis: Set<String>, probe: ExcerptEmojiProbe?) {
    if (depth >= EXCERPT_MAX_DEPTH) {
        append(source)
        return
    }
    val slices = inlineSlices(source, customEmojis)
    val candidates = probe?.candidates(source)
    var candidate = candidates?.nextOrNull()
    slices.forEachIndexed { index, (token, _, end) ->
        while (true) {
            val match = candidate ?: break
            if (match.range.first >= end) break
            probe?.inspect(source, slices, index, match, this)
            candidate = if (probe?.needsCatalog == true) null else candidates?.nextOrNull()
        }
        val value = token.value
        when (token.kind) {
            "bold", "italic", "strike", "small" -> appendTokens(value, depth + 1, customEmojis, probe)
            "effect" -> {
                val effect = mfmFunction(value.substring(2, value.length - 1))
                val body = if (effect.name == "ruby") {
                    val rubySlices = inlineSlices(effect.body, customEmojis)
                    val ruby = mfmRuby(effect.body, rubySlices)
                    probe?.inspectRuby(effect.body, rubySlices, this)
                    ruby.base
                } else effect.body
                appendTokens(body, depth + 1, customEmojis, probe)
            }
            // URL の文字そのものは、そのまま出す。`[文字](URL)` は文字だけを出す。
            "link" -> if (value == token.url) append(value) else appendTokens(value, depth + 1, customEmojis, probe)
            else -> append(value)
        }
    }
}
