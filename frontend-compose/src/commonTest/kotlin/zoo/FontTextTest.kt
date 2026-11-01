package zoo

import androidx.compose.ui.text.font.FontFamily
import kotlin.test.*

class FontTextTest {
    private val fonts = Fonts(FontFamily.SansSerif, FontFamily.Serif, FontFamily.Cursive)

    @Test
    fun geistOwnsItsPunctuationAndCombiningGlyphs() {
        val text = "A…B—“quote”"
        val rendered = fontText(text, fonts)
        assertEquals(text, rendered.text)
        assertEquals(fonts.latin, rendered.spanStyles.single().item.fontFamily)
    }

    @Test
    fun astralJapaneseUsesCjkRatherThanEmoji() {
        assertEquals(fonts.cjk, fontText("𠮷", fonts).spanStyles.single().item.fontFamily)
    }

    @Test
    fun emojiPresentationAndJoinedSequencesStayInOneFontRun() {
        for (text in listOf("©️", "1️⃣", "👩🏽‍💻", "🇯🇵")) {
            val rendered = fontText(text, fonts)
            val span = rendered.spanStyles.single()
            assertEquals(text, rendered.text)
            assertEquals(0, span.start)
            assertEquals(text.length, span.end)
            assertEquals(fonts.emoji, span.item.fontFamily)
        }
    }
}
