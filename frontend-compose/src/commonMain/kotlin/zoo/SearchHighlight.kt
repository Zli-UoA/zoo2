package zoo

/** Like the server's `parse_terms` (`backend/src/routes/search.rs`): at most this many distinct terms. */
const val MAX_SEARCH_TERMS = 5

/** The query's whitespace-separated terms, without duplicates, at most [MAX_SEARCH_TERMS] (as the server). */
fun searchTerms(query: String): List<String> =
    query.splitWhere { it.isWhitespace() }.filter { it.isNotEmpty() }.distinct().take(MAX_SEARCH_TERMS)

private fun String.splitWhere(separator: (Char) -> Boolean): List<String> {
    val parts = mutableListOf<String>()
    var start = 0
    forEachIndexed { index, char ->
        if (separator(char)) {
            parts += substring(start, index)
            start = index + 1
        }
    }
    parts += substring(start)
    return parts
}

data class HighlightSegment(val text: String, val match: Boolean)

/**
 * Splits [text] into the parts that hit a term and the rest. Case-insensitive (the server compares
 * `lower()`); where terms overlap the longer one wins. Without a hit the text is one segment.
 */
fun highlightSegments(text: String, terms: List<String>): List<HighlightSegment> {
    val usable = terms.filter { it.isNotEmpty() }
    if (text.isEmpty() || usable.isEmpty()) return listOf(HighlightSegment(text, false))
    val pattern = Regex(usable.sortedByDescending { it.length }.joinToString("|") { Regex.escape(it) }, RegexOption.IGNORE_CASE)
    val segments = mutableListOf<HighlightSegment>()
    var last = 0
    for (found in pattern.findAll(text)) {
        val start = found.range.first
        if (start > last) segments += HighlightSegment(text.substring(last, start), false)
        segments += HighlightSegment(found.value, true)
        last = found.range.last + 1
    }
    if (last < text.length) segments += HighlightSegment(text.substring(last), false)
    return segments.ifEmpty { listOf(HighlightSegment(text, false)) }
}
