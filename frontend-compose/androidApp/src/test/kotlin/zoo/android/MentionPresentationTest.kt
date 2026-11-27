package zoo.android

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import kotlin.test.Test
import kotlin.test.assertEquals
import zoo.android.ui.formatEntry
import zoo.obj

class MentionPresentationTest {
    @Test
    fun encryptedBodyDecoratesOnlyListedMentionsAndKeepsPunctuationOutside() {
        val style = SpanStyle(background = Color.Yellow)
        val result = formatEntry(
            "@alice. @bob @HERE", mentionStyle = style, allowedMentions = setOf("alice", "here"),
            mentionProfiles = mapOf("alice" to obj("display_name" to "Alice")),
        )

        assertEquals("@ Alice. @bob @HERE", result.text)
        assertEquals(listOf("@ Alice", "@HERE"), result.spanStyles.filter { it.item == style }
            .map { result.text.substring(it.start, it.end) })
    }
}
