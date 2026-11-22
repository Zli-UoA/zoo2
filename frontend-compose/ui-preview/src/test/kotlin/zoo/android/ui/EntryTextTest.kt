package zoo.android.ui

import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.style.TextDecoration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

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
}
