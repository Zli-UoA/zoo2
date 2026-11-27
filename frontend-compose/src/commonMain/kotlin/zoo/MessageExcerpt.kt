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

    fun inspect(source: String, slices: List<InlineSlice>, rubyBaseEnd: Int? = null) {
        if (needsCatalog || ':' !in source || slices.isEmpty()) return
        var first = 0
        for (match in customEmojiMatches(source)) {
            val start = match.range.first
            val end = match.range.last + 1
            if (rubyBaseEnd != null && start > rubyBaseEnd) break
            if (match.value in known || !isWorkspaceEmojiCandidate(match.value)) continue
            while (first < slices.lastIndex && slices[first].end <= start) first++
            val initial = slices[first]
            // A nested span is inspected when its body is appended. Literal spans are never inspected.
            if (initial.start < start && initial.token.kind !in setOf("text", "emoji")) continue
            if (initial.start == start && initial.end == end && initial.token.kind == "emoji") continue
            var previous = start - 1
            while (previous >= initial.start && source[previous] == '\\') previous--
            if ((start - previous - 1) % 2 != 0) continue
            var changesExcerpt = rubyBaseEnd != null
            var overlapsKnown = false
            for (index in first..slices.lastIndex) {
                val slice = slices[index]
                if (slice.start >= end) break
                if (slice.token.kind == "accountEmoji" || (slice.token.kind == "emoji" && slice.token.value in known)) {
                    overlapsKnown = true
                    break
                }
                if (slice.token.kind !in setOf("text", "emoji")) changesExcerpt = true
            }
            // A potential shortcode cannot take a colon already used by a registered emoji.
            if (changesExcerpt && !overlapsKnown) {
                needsCatalog = true
                return
            }
        }
    }
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
    probe?.inspect(source, slices)
    slices.forEach { (token) ->
        val value = token.value
        when (token.kind) {
            "bold", "italic", "strike", "small" -> appendTokens(value, depth + 1, customEmojis, probe)
            "effect" -> {
                val effect = mfmFunction(value.substring(2, value.length - 1))
                val body = if (effect.name == "ruby") {
                    val rubySlices = inlineSlices(effect.body, customEmojis)
                    val ruby = mfmRuby(effect.body, rubySlices)
                    probe?.inspect(effect.body, rubySlices, ruby.base.length)
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
