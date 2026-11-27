package zoo.android.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.style.TextDecoration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import zoo.obj

class EntryTextTest {
    @Test
    fun parsesNestedFormattingAndRetainsLiteralText() {
        val text = formatEntry("**bold ~~gone~~** <plain>**literal**</plain> `code`")
        assertEquals("bold gone **literal** code", text.text)
        assertTrue(text.spanStyles.any { it.item.textDecoration == TextDecoration.LineThrough && it.start == 5 && it.end == 9 })
    }

    @Test
    fun showsCodeFencesWithoutTheLanguageAndKeepsLinkAnnotations() {
        val text = formatEntry("[Guide](https://example.test/guide)\n```kotlin\nval a = 1\n```")
        assertEquals("Guide\nval a = 1\n", text.text)
        assertEquals("https://example.test/guide", (text.getLinkAnnotations(0, 5).single().item as LinkAnnotation.Url).url)
    }

    @Test
    fun linkLabelsKeepEmojiWhileCodeAndPlainRegionsKeepShortcodes() {
        val text = formatEntry("[**:お祝い 猫:**](https://example.test/emoji) `:お祝い 猫:` <plain>:お祝い 猫:</plain>",
            emoji = setOf(":お祝い 猫:"))
        assertEquals(":お祝い 猫: :お祝い 猫: :お祝い 猫:", text.text)
        assertEquals("https://example.test/emoji", (text.getLinkAnnotations(0, text.length).single().item as LinkAnnotation.Url).url)
        val emoji = text.getStringAnnotations("zoo-entry-emoji", 0, text.length).single()
        assertEquals(0, emoji.start)
        assertEquals(7, emoji.end)
    }

    @Test
    fun marksSearchTermsOnlyInTextParts() {
        val mark = SpanStyle(background = Color.Yellow)
        val text = formatEntry(
            "**Rust** と `rust` と \$[x2 rust] と @rust と [Rust の本](https://example.test) と <plain>**rust**</plain>\n```\nrust\n```",
            highlight = EntryHighlight(listOf("rust"), mark),
        )
        assertEquals("Rust と rust と rust と @rust と Rust の本 と **rust**\nrust\n", text.text)
        // The bold, effect, link-label and `<plain>` words are marked (the `<plain>` syntax stays literal, as on Web);
        // inline code, the mention and the code fence are not.
        val marked = text.spanStyles.filter { it.item == mark }.map { it.start until it.end }
        assertEquals(listOf(0 until 4, 14 until 18, 29 until 33, 41 until 45), marked)
    }

    @Test
    fun snippetsCanDrawLinksAsText() {
        val text = formatEntry("[**Guide**](https://example.test/guide) と https://example.test/raw", links = false)
        assertEquals("Guide と https://example.test/raw", text.text)
        assertTrue(text.getLinkAnnotations(0, text.length).isEmpty())
        assertTrue(text.spanStyles.any { it.item.fontWeight == AndroidWeights.emphasis && it.start == 0 && it.end == 5 })
    }

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
