package zoo

/** 要約の最大文字数（コードポイント単位）。画面側でも行数で切るので、これは保険の上限（Web と同じ）。 */
const val EXCERPT_MAX_LENGTH = 200

private const val EXCERPT_MAX_DEPTH = 16

private val whitespace = Regex("\\s+")

/**
 * メッセージ本文を、埋め込みカードに出す短い素の文字列にする（Web の `messageExcerpt`）。装飾・リンク・
 * コードの記法を外し、改行や連続する空白を 1 つにまとめ、長いときは切って `…` を付ける。
 */
fun messageExcerpt(content: String, maxLength: Int = EXCERPT_MAX_LENGTH): String {
    val text = buildString {
        messageHeadingBlocks(content).forEach { block ->
            appendBlocks(block.content, 0)
            append(' ')
        }
    }
    val collapsed = text.replace(whitespace, " ").trim()
    if (collapsed.codePointCount() <= maxLength) return collapsed
    var end = 0
    repeat(maxLength) {
        end += if (collapsed[end].isHighSurrogate() && collapsed.getOrNull(end + 1)?.isLowSurrogate() == true) 2 else 1
    }
    return collapsed.substring(0, end).trimEnd() + "…"
}

/** ブロック（コード・中央寄せ・引用）の記法を外す。前後に空白を置いて、前後の文とつながらないようにする。 */
private fun StringBuilder.appendBlocks(body: String, depth: Int) {
    if (depth >= EXCERPT_MAX_DEPTH) {
        append(body)
        return
    }
    richSegments(body).forEach { (source, block) ->
        when {
            block && source.startsWith("```") -> append(' ').append(mfmCodeBlock(source).body).append(' ')
            block && source.startsWith("<center>") -> {
                append(' ')
                appendBlocks(source.removePrefix("<center>").removeSuffix("</center>"), depth + 1)
                append(' ')
            }
            block && source.startsWith(">") -> source.lines().forEach { line ->
                append(' ')
                appendBlocks(line.removePrefix(">").trimStart(), depth + 1)
            }
            else -> appendTokens(source, depth)
        }
    }
}

private fun StringBuilder.appendTokens(source: String, depth: Int) {
    if (depth >= EXCERPT_MAX_DEPTH) {
        append(source)
        return
    }
    parseInline(source).forEach { token ->
        val value = token.value
        when (token.kind) {
            "bold", "italic", "strike", "small" -> appendTokens(value, depth + 1)
            "effect" -> {
                val effect = mfmFunction(value.substring(2, value.length - 1))
                appendTokens(if (effect.name == "ruby") mfmRuby(effect.body).base else effect.body, depth + 1)
            }
            // URL の文字そのものは、そのまま出す。`[文字](URL)` は文字だけを出す。
            "link" -> if (value == token.url) append(value) else appendTokens(value, depth + 1)
            else -> append(value)
        }
    }
}
