package zoo

/**
 * Tokens retain their source bodies so nested styles can be rendered in the same paragraph. [bare] marks a
 * link written as its URL itself (a `[label](url)` link is not, even when the label is the URL).
 */
data class Inline(val value: String, val kind: String = "text", val url: String? = null, val bare: Boolean = false)

private val atoms = Regex(
    "https?://[^\\s<>]+|:@[A-Za-z0-9._-]+:|@[A-Za-z0-9._-]+(?:@[A-Za-z0-9.-]+)?|#[^\\s#.,!?:;<>]+|:[a-zA-Z0-9_+-]+:"
)
// Shared emoji aliases may include Unicode and spaces. Only registered candidates become tokens.
private val customEmojiShortcode = Regex(""":[^:\p{Cc}]{1,100}:""")

fun findCustomEmojiShortcodes(source: String): Sequence<String> = sequence {
    var match = customEmojiShortcode.find(source)
    while (match != null) {
        yield(match.value)
        // The closing colon of an unregistered candidate can open a registered shortcode.
        match = customEmojiShortcode.find(source, match.range.first + 1)
    }
}

private val functionHeader = Regex("[A-Za-z0-9_]+(?:\\.[A-Za-z0-9_=,+.\\-]+)?[ \\t\\n]")
private val formats = listOf(
    Triple("<small>", "</small>", "small"),
    Triple("<b>", "</b>", "bold"),
    Triple("<i>", "</i>", "italic"),
    Triple("<s>", "</s>", "strike"),
    Triple("***", "***", "tada"),
    Triple("**", "**", "bold"),
    Triple("__", "__", "bold"),
    Triple("~~", "~~", "strike"),
    Triple("*", "*", "italic"),
    Triple("_", "_", "italic"),
)

private data class InlineSlice(val token: Inline, val start: Int, val end: Int)

fun parseInline(source: String, customEmojis: Set<String> = emptySet()): List<Inline> =
    inlineSlices(source, customEmojis).map { it.token }

private fun inlineSlices(source: String, customEmojis: Set<String> = emptySet()): List<InlineSlice> =
    MfmScanner(source.length).inlineSlices(source, customEmojis)
fun richSegments(source: String): List<Pair<String, Boolean>> = MfmScanner(source.length).richSegments(source)

/** A shared work limit bounds repeated searches as well as recursion depth. */
private class MfmScanner(length: Int) {
    private var remaining = (length.toLong() * 64).coerceIn(4096, 1_000_000).toInt()
    private fun step(): Boolean = remaining-- > 0

    /** End of a literal region. Brackets in these regions never close an MFM function. */
    private fun literalEnd(source: String, at: Int): Int? {
        val (open, close) = when {
            source.startsWith("```", at) -> "```" to "```"
            source.startsWith("`", at) -> "`" to "`"
            source.startsWith("<plain>", at) -> "<plain>" to "</plain>"
            source.startsWith("\\(", at) -> "\\(" to "\\)"
            source.startsWith("\\[", at) -> "\\[" to "\\]"
            else -> return null
        }
        val end = source.indexOf(close, at + open.length)
        if (end < 0 || (open == "`" && '\n' in source.substring(at + 1, end))) return null
        return end + close.length
    }

    private fun closing(source: String, from: Int, close: String, open: String? = null, budget: Int = 0): Int? {
        if (budget > 20) return null
        var nesting = 1
        var at = from
        while (at < source.length) {
            if (!step()) break
            // A nested function owns its formatting delimiters.
            if (source.startsWith("$[", at)) {
                val nestedEnd = functionEnd(source, at, budget + 1)
                if (nestedEnd != null) {
                    at = nestedEnd
                    continue
                }
            }
            if (close == "*" && source.startsWith("**", at)) {
                val nestedEnd = closing(source, at + 2, "**", budget = budget + 1)
                if (nestedEnd != null) {
                    at = nestedEnd + 2
                    continue
                }
            }
            if (source.startsWith(close, at)) {
                nesting--
                if (nesting == 0) return at
                at += close.length
                continue
            }
            val literal = literalEnd(source, at)
            if (literal != null) {
                at = literal
                continue
            }
            if (open != null && source.startsWith(open, at)) {
                nesting++
                at += open.length
                continue
            }
            if (source[at] == '\\' && at + 1 < source.length) at += 2 else at++
        }
        return null
    }

    private fun functionEnd(source: String, at: Int, budget: Int = 0): Int? {
        if (budget > 20) return null
        var cursor = at + 2
        var brackets = 1
        while (cursor < source.length) {
            if (!step()) return null
            val literal = literalEnd(source, cursor)
            if (literal != null) {
                cursor = literal
                continue
            }
            if (source.startsWith("$[", cursor)) {
                brackets++
                cursor += 2
                continue
            }
            val link = if (source[cursor] == '[' || source.startsWith("?[", cursor))
                linkAt(source, cursor, budget + 1) else null
            if (link != null) {
                cursor = link.second
                continue
            }
            when (source[cursor]) {
                '\\' -> {
                    cursor += 2
                    continue
                }
                ']' -> {
                    brackets--
                    if (brackets == 0) return cursor + 1
                }
            }
            cursor++
        }
        return null
    }

    private fun linkAt(source: String, at: Int, budget: Int = 0): Pair<Inline, Int>? {
        if (budget > 20) return null
        val start = if (source.startsWith("?[", at)) at + 1 else at
        if (source.getOrNull(start) != '[') return null
        val labelEnd = closing(source, start + 1, "](", budget = budget + 1) ?: return null
        val urlStart = labelEnd + 2
        val urlEnd = linkDestinationEnd(source, urlStart) { step() } ?: return null
        return Inline(
            source.substring(start + 1, labelEnd),
            "link",
            source.substring(urlStart, urlEnd),
        ) to (urlEnd + 1)
    }

    fun inlineSlices(source: String, customEmojis: Set<String>): List<InlineSlice> {
        val result = mutableListOf<InlineSlice>()
        var at = 0
        var pendingStart = 0
        val pending = StringBuilder()
        fun flush() {
            if (pending.isNotEmpty()) {
                result += InlineSlice(Inline(pending.toString()), pendingStart, at)
                pending.clear()
            }
        }
        while (at < source.length) {
            if (!step()) break
            var token: Inline? = null
            var end = at
            val literal = literalEnd(source, at)
            if (literal != null) {
                val plain = source.startsWith("<plain>", at)
                val math = source.startsWith("\\(", at) || source.startsWith("\\[", at)
                val fence = source.startsWith("```", at)
                val open = if (plain) 7 else if (math) 2 else if (fence) 3 else 1
                val close = if (plain) 8 else open
                token = Inline(
                    source.substring(at + open, literal - close),
                    if (plain) "plain" else if (math) "math" else "code",
                )
                end = literal
            } else if (source[at] == '<' &&
                (source.startsWith("https://", at + 1) || source.startsWith("http://", at + 1))) {
                val close = source.indexOf('>', at + 1)
                if (close >= 0 && source.substring(at + 1, close).none { it.isWhitespace() }) {
                    val url = source.substring(at + 1, close)
                    token = Inline(url, "link", url, bare = true)
                    end = close + 1
                }
            } else if (source[at] == '\\' && at + 1 < source.length && source[at + 1] in "\\`*_{}[]()#+-.!~$<>@:") {
                pending.append(source[at + 1])
                at += 2
                continue
            } else if (source.startsWith("$[", at) && functionHeader.matchAt(source, at + 2) != null) {
                functionEnd(source, at)?.let {
                    end = it
                    token = Inline(source.substring(at, end), "effect")
                }
            } else {
                val link = if (source[at] == '[' || source.startsWith("?[", at)) linkAt(source, at) else null
                if (link != null) {
                    token = link.first
                    end = link.second
                }
                else {
                    for ((open, close, kind) in formats) {
                        if (!source.startsWith(open, at)) continue
                        // Single delimiters inside words are literal.
                        if (open.length == 1 && source.getOrNull(at - 1)?.isLetterOrDigit() == true) continue
                        val stop = closing(source, at + open.length, close, open.takeIf { it.startsWith('<') }) ?: continue
                        if (stop == at + open.length) continue
                        val body = source.substring(at + open.length, stop)
                        if (open.length == 1 && '\n' in body) continue
                        token = if (kind == "tada") Inline("$[tada $body]", "effect") else Inline(body, kind)
                        end = stop + close.length
                        break
                    }
                    val atom = if (token == null) atoms.matchAt(source, at) else null
                    if (token == null && atom?.value?.startsWith(":@") != true && source[at] == ':' && customEmojis.isNotEmpty()) {
                        val shortcode = customEmojiShortcode.matchAt(source, at)?.value
                        if (shortcode != null && shortcode in customEmojis) {
                            token = Inline(shortcode, "emoji")
                            end = at + shortcode.length
                        }
                    }
                    if (token == null) {
                        val match = atom
                        if (match != null) {
                            val value = match.value
                            val kind = when {
                                value.startsWith("http") -> "link"
                                value.startsWith(":@") -> "accountEmoji"
                                value.startsWith('@') -> "mention"
                                value.startsWith('#') -> "hashtag"
                                else -> "emoji"
                            }
                            val boundary = source.getOrNull(at - 1)?.let {
                                !it.isLetterOrDigit() && it !in "_/@"
                            } != false
                            val overlapsRegistered = kind == "emoji" && customEmojis.isNotEmpty() &&
                                customEmojiShortcode.matchAt(source, match.range.last)?.value in customEmojis
                            if (!overlapsRegistered && (kind !in setOf("mention", "hashtag") || boundary)) {
                                val trimmed = if (kind == "link") leadingUrl(value) else value
                                if (trimmed != null) {
                                    token = Inline(trimmed, kind, trimmed.takeIf { kind == "link" }, kind == "link")
                                    end = at + trimmed.length
                                }
                            }
                        }
                    }
                }
            }
            if (token != null) {
                flush()
                result += InlineSlice(token, at, end)
                at = end
                pendingStart = at
            } else {
                pending.append(source[at])
                at++
            }
        }
        pending.append(source.substring(at))
        at = source.length
        flush()
        return result
    }

    /** Split block and embedded regions without interpreting delimiters in literal text. */
    fun richSegments(source: String): List<Pair<String, Boolean>> {
        val result = mutableListOf<Pair<String, Boolean>>()
        var at = 0
        var plain = 0
        while (at < source.length) {
            if (!step()) break
            var end: Int? = null
            if ((at == 0 || source[at - 1] == '\n') && source[at] == '>') {
                end = source.indexOf('\n', at).let { if (it < 0) source.length else it }
                while (end!! < source.length && source.getOrNull(end + 1) == '>')
                    end = source.indexOf('\n', end + 1).let { if (it < 0) source.length else it }
            } else {
                val literal = literalEnd(source, at)
                if (literal != null) {
                    if (source[at] == '`' && !source.startsWith("```", at)) {
                        at = literal
                        continue
                    }
                    end = literal
                } else if (source.startsWith("$[", at)) end = functionEnd(source, at)
                else if (source.startsWith("<center>", at)) {
                    // Centers may contain other centers and literal closing tags.
                    var cursor = at + 8
                    var nesting = 1
                    while (cursor < source.length) {
                        if (!step()) break
                        val protected = literalEnd(source, cursor)
                        if (protected != null) {
                            cursor = protected
                            continue
                        }
                        if (source.startsWith("<center>", cursor)) {
                            nesting++
                            cursor += 8
                        }
                        else if (source.startsWith("</center>", cursor)) {
                            nesting--
                            cursor += 9
                            if (nesting == 0) {
                                end = cursor
                                break
                            }
                        } else cursor++
                    }
                } else if (source[at] == '\\' && at + 1 < source.length) {
                    at += 2
                    continue
                }
            }
            if (end == null) {
                at++
                continue
            }
            if (at > plain) result += source.substring(plain, at) to false
            result += source.substring(at, end) to true
            at = end
            plain = end
        }
        if (plain < source.length) result += source.substring(plain) to false
        return result
    }

}

data class MessageHeadingBlock(val content: String, val level: Int? = null)

/** Heading markers only belong to root text, never to code or an MFM subtree. */
fun messageHeadingBlocks(content: String): List<MessageHeadingBlock> {
    val normalized = content.replace("\r\n", "\n").replace('\r', '\n')
    val protectedRanges = BooleanArray(normalized.length)
    inlineSlices(normalized).filter { it.token.kind != "text" }.forEach { slice ->
        for (index in slice.start until slice.end) protectedRanges[index] = true
    }
    var cursor = 0
    richSegments(normalized).forEach { (text, embedded) ->
        if (embedded) for (index in cursor until cursor + text.length) protectedRanges[index] = true
        cursor += text.length
    }
    val blocks = mutableListOf<MessageHeadingBlock>()
    val pending = mutableListOf<String>()
    fun flush() {
        if (pending.isNotEmpty()) {
            val text = pending.joinToString("\n")
            if (text.isNotEmpty()) blocks += MessageHeadingBlock(text)
            pending.clear()
        }
    }
    var offset = 0
    normalized.lines().forEach { line ->
        val marker = Regex("^ {0,3}(#{1,6})[ \t]+").find(line)
        val source = line.substring(marker?.value?.length ?: 0)
        if (
            marker != null &&
                source.isNotBlank() &&
                protectedRanges.getOrNull(offset + marker.value.indexOf('#')) != true &&
                !Regex("^([^\n]+) (検索|Search)$").matches(line)
        ) {
            flush()
            blocks +=
                MessageHeadingBlock(
                    source.replace(Regex("[ \t]+#+[ \t]*$"), "").trimEnd(),
                    marker.groupValues[1].length,
                )
        } else pending += line
        offset += line.length + 1
    }
    flush()
    return blocks
}

/** 閉じていない括弧の対応を探す長さ。閉じない括弧が並ぶ本文で、同じ部分を繰り返し調べないようにする。 */
private const val URL_BRACKET_LOOKAHEAD = 1024

/**
 * [text] の [start] から始まる `http://` か `https://` の URL。Web（`mfm-js`）と同じく、URL に使う ASCII の
 * 文字と、対応のとれた括弧までを URL とし、末尾の `.` と `,` は除く。日本語や全角の記号が続いても、そこで終わる。
 * 表示と、本文のリンクの抽出（[previewUrls]）で同じ位置で切る。
 */
internal fun leadingUrl(text: String, start: Int = 0): String? {
    val scheme = listOf("https://", "http://").firstOrNull { text.startsWith(it, start) } ?: return null
    // 閉じていない括弧の位置と、対応する閉じ括弧。
    val open = mutableListOf<Pair<Int, Char>>()
    var end = start + scheme.length
    while (end < text.length) {
        val unclosed = open.firstOrNull()
        if (unclosed != null && end - unclosed.first > URL_BRACKET_LOOKAHEAD) break
        val ch = text[end]
        val used = when (ch) {
            '(', '[' -> {
                open.add(end to if (ch == '(') ')' else ']')
                true
            }
            ')', ']' -> {
                // 対応しないときは、開き括弧を残す（閉じていない括弧として、URL から除くため）。
                val paired = open.lastOrNull()?.second == ch
                if (paired) open.removeAt(open.lastIndex)
                paired
            }
            else -> ch in 'a'..'z' || ch in 'A'..'Z' || ch in '0'..'9' || ch in "._,/:%#@$&?!~=+-"
        }
        if (!used) break
        end++
    }
    // 閉じていない括弧は、URL に含めない。
    open.firstOrNull()?.let { end = it.first }
    val url = text.substring(start, end).trimEnd('.', ',')
    return url.takeIf { it.length > scheme.length }
}

/**
 * `[文字](` の直後 [start] から始まる行き先の終わり（対応する `)` の位置）。行き先は空白を含まず、丸括弧は対にする。
 * http(s) の URL でないとき、閉じないとき、[step] が打ち切ったときは `null`（リンクではない）。
 */
internal fun linkDestinationEnd(text: String, start: Int, step: () -> Boolean): Int? {
    if (!text.startsWith("https://", start) && !text.startsWith("http://", start)) return null
    var parentheses = 1
    for (cursor in start until text.length) {
        val ch = text[cursor]
        if (ch.isWhitespace() || !step()) return null
        if (ch == '(') parentheses++
        if (ch == ')' && --parentheses == 0) return cursor
    }
    return null
}

data class MfmRuby(val base: String, val reading: String)

/** The reading is root text; spaces inside markup belong to the base. */
fun mfmRuby(source: String, customEmojis: Set<String> = emptySet()): MfmRuby {
    val slices = inlineSlices(source, customEmojis)
    val last = slices.lastOrNull() ?: return MfmRuby(source, "")
    if (last.token.kind != "text") return MfmRuby(source, "")
    if (slices.size == 1) {
        val separator = source.indexOfFirst { it.isWhitespace() }
        return if (separator < 0) MfmRuby(source, "")
        else MfmRuby(source.take(separator), source.substring(separator + 1).trim())
    }
    return MfmRuby(source.take(last.start).trimEnd(), last.token.value.trim())
}

data class MfmCodeBlock(val body: String, val language: String)

fun mfmCodeBlock(source: String): MfmCodeBlock {
    val inner = source.removePrefix("```").removeSuffix("```")
    val newline = inner.indexOf('\n')
    return if (newline < 0) MfmCodeBlock(inner, "")
    else MfmCodeBlock(inner.substring(newline + 1), inner.take(newline).trim())
}
