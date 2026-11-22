package zoo

import kotlin.test.Test
import kotlin.test.assertEquals

class SearchHighlightTest {
    private fun hit(text: String) = HighlightSegment(text, true)

    private fun plain(text: String) = HighlightSegment(text, false)

    @Test
    fun splitsOnWhitespaceLikeTheServerWithoutDuplicatesUpToFiveTerms() {
        assertEquals(listOf("検索", "テスト", "foo"), searchTerms("  検索　テスト 検索\tfoo "))
        assertEquals(listOf("a", "b", "c", "d", "e"), searchTerms("a b c d e f g"))
        assertEquals(emptyList(), searchTerms("   "))
    }

    @Test
    fun marksEveryHitCaseInsensitively() {
        assertEquals(
            listOf(hit("Rust"), plain(" と "), hit("rust"), plain(" と "), hit("RUST")),
            highlightSegments("Rust と rust と RUST", listOf("rust")),
        )
        assertEquals(listOf(hit("Ärger")), highlightSegments("Ärger", listOf("ärger")))
    }

    @Test
    fun prefersTheLongerTermWhereTermsOverlap() {
        assertEquals(
            listOf(hit("全文検索"), plain("の"), hit("検索")),
            highlightSegments("全文検索の検索", listOf("検索", "全文検索")),
        )
    }

    @Test
    fun treatsRegularExpressionCharactersLiterally() {
        assertEquals(
            listOf(hit("a.b"), plain(" axb "), hit("(c)")),
            highlightSegments("a.b axb (c)", listOf("a.b", "(c)")),
        )
    }

    @Test
    fun returnsTheTextUnchangedWithoutTermsOrHits() {
        assertEquals(listOf(plain("本文")), highlightSegments("本文", emptyList()))
        assertEquals(listOf(plain("本文")), highlightSegments("本文", listOf("なし")))
    }

    @Test
    fun keepsSurrogatePairsWhole() {
        assertEquals(listOf(hit("🐘"), plain("と"), hit("🐘")), highlightSegments("🐘と🐘", listOf("🐘")))
    }

    @Test
    fun findsTheWordsTheServerRejects() {
        assertEquals(null, searchQueryProblem("a b c d e"))
        assertEquals(null, searchQueryProblem("a b c d e a b"), "Duplicates count once")
        assertEquals(SearchQueryProblem.TooManyTerms, searchQueryProblem("a b c d e f"))
        assertEquals(null, searchQueryProblem("🐘".repeat(MAX_SEARCH_TERM_CHARS)), "Counted in code points")
        assertEquals(SearchQueryProblem.TermTooLong, searchQueryProblem("x".repeat(MAX_SEARCH_TERM_CHARS + 1)))
        assertEquals(SearchQueryProblem.TermTooLong, searchQueryProblem("a b c d e f " + "x".repeat(101)),
            "Like the server, the length is checked first")
    }
}
