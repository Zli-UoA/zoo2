package zoo

/** Tokens retain their source bodies so nested styles can be rendered in the same paragraph. */
data class Inline(val value: String, val kind: String = "text", val url: String? = null)

private val atoms = Regex(
    "https?://[^\\s<>]+|:@[A-Za-z0-9._-]+:|@[A-Za-z0-9._-]+(?:@[A-Za-z0-9.-]+)?|#[^\\s#.,!?:;<>]+|:[a-zA-Z0-9_+-]+:"
)
private val functionHeader = Regex("[A-Za-z0-9_]+(?:\\.[A-Za-z0-9_=,+.\\-]+)?[ \\t\\n]")
private val formats = listOf(
    Triple("<small>", "</small>", "small"),
    Triple("<b>", "</b>", "bold"),
    Triple("<i>", "</i>", "italic"),
    Triple("<s>", "</s>", "strike"),
    Triple("**", "**", "bold"),
    Triple("~~", "~~", "strike"),
    Triple("*", "*", "italic"),
    Triple("_", "_", "italic"),
)

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

private fun closing(source: String, from: Int, close: String, open: String? = null): Int? {
    var nesting = 1
    var at = from
    while (at < source.length) {
        // A nested function owns its formatting delimiters.
        if (source.startsWith("$[", at)) {
            val nestedEnd = functionEnd(source, at)
            if (nestedEnd != null) {
                at = nestedEnd
                continue
            }
        }
        if (close == "*" && source.startsWith("**", at)) {
            val nestedEnd = closing(source, at + 2, "**")
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

private fun functionEnd(source: String, at: Int): Int? {
    var cursor = at + 2
    var brackets = 1
    while (cursor < source.length) {
        val literal = literalEnd(source, cursor)
        if (literal != null) {
            cursor = literal
            continue
        }
        when (source[cursor]) {
            '\\' -> {
                cursor += 2
                continue
            }
            '[' -> brackets++
            ']' -> {
                brackets--
                if (brackets == 0) return cursor + 1
            }
        }
        cursor++
    }
    return null
}

private fun linkAt(source: String, at: Int): Pair<Inline, Int>? {
    val start = if (source.startsWith("?[", at)) at + 1 else at
    if (source.getOrNull(start) != '[') return null
    val labelEnd = closing(source, start + 1, "](") ?: return null
    val urlStart = labelEnd + 2
    if (!source.startsWith("https://", urlStart) && !source.startsWith("http://", urlStart)) return null
    var cursor = urlStart
    var parentheses = 1
    while (cursor < source.length && !source[cursor].isWhitespace()) {
        if (source[cursor] == '(') parentheses++
        if (source[cursor] == ')') {
            parentheses--
            if (parentheses == 0) {
                return Inline(
                    source.substring(start + 1, labelEnd),
                    "link",
                    source.substring(urlStart, cursor),
                ) to (cursor + 1)
            }
        }
        cursor++
    }
    return null
}

private data class InlineSlice(val token: Inline, val start: Int, val end: Int)

fun parseInline(source: String): List<Inline> = inlineSlices(source).map { it.token }

private fun inlineSlices(source: String): List<InlineSlice> {
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
                    token = Inline(body, kind)
                    end = stop + close.length
                    break
                }
                if (token == null) {
                    val match = atoms.matchAt(source, at)
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
                        if (kind !in setOf("mention", "hashtag") || boundary) {
                            val trimmed = if (kind == "link") trimMfmUrl(value) else value
                            token = Inline(trimmed, kind, trimmed.takeIf { kind == "link" })
                            end = at + trimmed.length
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
    flush()
    return result
}

/** Split block and embedded regions without interpreting delimiters in literal text. */
fun richSegments(source: String): List<Pair<String, Boolean>> {
    val result = mutableListOf<Pair<String, Boolean>>()
    var at = 0
    var plain = 0
    while (at < source.length) {
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

data class MessageHeadingBlock(val content: String, val level: Int? = null)

/** Heading markers only belong to root text, never to code or an MFM subtree. */
fun messageHeadingBlocks(content: String): List<MessageHeadingBlock> {
    val protectedRanges = BooleanArray(content.length)
    inlineSlices(content).filter { it.token.kind != "text" }.forEach { slice ->
        for (index in slice.start until slice.end) protectedRanges[index] = true
    }
    var cursor = 0
    richSegments(content).forEach { (text, embedded) ->
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
    content.lines().forEach { line ->
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

private fun trimMfmUrl(value: String): String {
    var result = value.trimEnd('.', ',', '!', '?', ';', ':')
    for ((open, close) in listOf('(' to ')', '[' to ']', '{' to '}')) {
        var excess = result.count { it == close } - result.count { it == open }
        while (excess > 0 && result.endsWith(close)) {
            result = result.dropLast(1)
            excess--
        }
    }
    return result
}

data class MfmRuby(val base: String, val reading: String)

/** The reading is root text; spaces inside markup belong to the base. */
fun mfmRuby(source: String): MfmRuby {
    val slices = inlineSlices(source)
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
