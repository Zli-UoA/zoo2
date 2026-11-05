package zoo

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import kotlin.test.*

class InputLengthTest {
    @Test
    fun truncatingAnInsertionKeepsTheExistingSuffixAndCaret() {
        val previous = TextFieldValue("abcd", TextRange(1, 3))
        val next = TextFieldValue("aXYZd", TextRange(4))
        val accepted = limitFieldEdit(previous, next, 4)
        assertEquals("aXYd", accepted.text)
        assertEquals(TextRange(3), accepted.selection)
        val atEnd = previous.copy(selection = TextRange(4))
        assertEquals(atEnd, limitFieldEdit(atEnd, TextFieldValue("abcde", TextRange(5)), 4))
    }

    @Test
    fun compositionIsUnrestrictedUntilCommitAndSurrogatePairsStayWhole() {
        val before = TextFieldValue("a", TextRange(1))
        val composing = TextFieldValue("a日本語", TextRange(4), TextRange(1, 4))
        assertSame(composing, limitFieldEdit(before, composing, 2))
        assertEquals("a日", limitFieldEdit(before, composing.copy(composition = null), 2).text)
        assertEquals("a", limitFieldEdit(before, TextFieldValue("a😀", TextRange(3)), 2).text)
    }
}
