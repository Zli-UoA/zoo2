package zoo

import androidx.compose.ui.text.TextRange
import kotlin.test.*

class ComposerFormattingTest {
    @Test fun reversedJapaneseSelectionKeepsTheSelectedTextAndSurroundingDraft() {
        val edit = formatComposerText("前日本語後", TextRange(4, 1), ComposerFormat.Bold)
        assertEquals("前<b>日本語</b>後", edit.text)
        assertEquals(TextRange(4, 7), edit.selection)
        assertEquals("bold", parseInline(edit.text).first { it.kind == "bold" }.kind)
    }
    @Test fun emptySelectionPlacesCaretBetweenMarkers() {
        val edit = formatComposerText("前後", TextRange(1), ComposerFormat.Italic)
        assertEquals("前<i></i>後", edit.text)
        assertEquals(TextRange(4), edit.selection)
    }
    @Test fun nestedFormattingDoesNotProduceTheMfmTripleAsteriskEffect() {
        val bold = formatComposerText("文字", TextRange(0, 2), ComposerFormat.Bold)
        val italic = formatComposerText(bold.text, bold.selection, ComposerFormat.Italic)
        assertEquals("<b><i>文字</i></b>", italic.text)
        assertEquals("bold", parseInline(italic.text).single().kind)
        assertEquals("italic", parseInline(parseInline(italic.text).single().value).single().kind)
    }
    @Test fun codeBlockGetsLineBoundariesWithoutRemovingBackticksOrJapanese() {
        val edit = formatComposerText("前a`b\n日本語後", TextRange(1, 8), ComposerFormat.Code)
        assertEquals("前\n```\na`b\n日本語\n```\n後", edit.text)
        assertEquals("a`b\n日本語", edit.text.substring(edit.selection.min, edit.selection.max))
        val fenced = "```\n既存のコード\n```"
        assertEquals(fenced, formatComposerText(fenced, TextRange(0, fenced.length), ComposerFormat.Code).text)
    }
    @Test fun emojiInsertionReplacesSelectionAndKeepsTheUtf16CaretAfterTheEmoji() {
        val edit = insertComposerText("前日本語後", TextRange(1, 4), "😀")
        assertEquals("前😀後", edit.text)
        assertEquals(TextRange(3), edit.selection)
    }
}
