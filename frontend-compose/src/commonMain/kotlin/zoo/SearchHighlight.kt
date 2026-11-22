package zoo

/** The server's limit in `parse_terms` (`backend/src/routes/search.rs`): more distinct terms are rejected with 400. */
const val MAX_SEARCH_TERMS = 5

/** The server's `MAX_TERM_CHARS`: a longer term (in code points) is rejected with 400. */
const val MAX_SEARCH_TERM_CHARS = 100

/** Why the server would reject a query; the key under `globalSearch.` (Web's `SearchQueryProblem`). */
enum class SearchQueryProblem(val key: String, val max: Int) {
    TooManyTerms("tooManyTerms", MAX_SEARCH_TERMS),
    TermTooLong("termTooLong", MAX_SEARCH_TERM_CHARS),
}

/** Finds what `parse_terms` rejects before sending; like the server, the term length is checked first. */
fun searchQueryProblem(query: String): SearchQueryProblem? {
    val terms = mutableListOf<String>()
    for (term in query.splitWhere { it.isWhitespace() }) {
        if (term.isEmpty()) continue
        if (term.searchTermLength() > MAX_SEARCH_TERM_CHARS) return SearchQueryProblem.TermTooLong
        if (term !in terms) terms += term
    }
    return if (terms.size > MAX_SEARCH_TERMS) SearchQueryProblem.TooManyTerms else null
}

private fun String.searchTermLength(): Int {
    var count = 0
    var index = 0
    while (index < length) {
        index += if (this[index].isHighSurrogate() && index + 1 < length && this[index + 1].isLowSurrogate()) 2 else 1
        count++
    }
    return count
}

/** The query's distinct whitespace-separated terms to mark, at most [MAX_SEARCH_TERMS] (like Web's `searchTerms`). */
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
