package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import zoo.android.ui.EmojiChoice
import zoo.android.ui.rankEmojiChoices

class EmojiRankTest {
    private fun choice(name: String, keywords: String = "") = EmojiChoice(":$name:", "", name, keywords = keywords)

    @Test
    fun exactNameMatchesComeFirstThenPrefixThenSubstring() {
        val choices = listOf(
            choice("kusa_hayasu"), choice("big_kusa"), choice("kusa"), choice("kusaa"),
            choice("w", "kusa"), choice("other"),
        )
        assertEquals(
            listOf("kusa", "kusaa", "kusa_hayasu", "big_kusa", "w"),
            rankEmojiChoices(choices, "kusa").map { it.name },
        )
    }

    @Test
    fun keywordMatchesRankByExactThenPrefixThenSubstring() {
        val choices = listOf(
            choice("x", "kusaa"), choice("long_name", "kusa"), choice("y", "bigkusa"), choice("z", "kusa"),
        )
        assertEquals(listOf("z", "long_name", "x", "y"), rankEmojiChoices(choices, "kusa").map { it.name })
    }

    @Test
    fun emptyQueryKeepsTheOriginalOrder() {
        val choices = listOf(choice("longer_name"), choice("a"), choice("mid"))
        assertEquals(listOf("longer_name", "a", "mid"), rankEmojiChoices(choices, "  ").map { it.name })
    }
}
