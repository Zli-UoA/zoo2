package zoo

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class ChannelNoteMarkdownTest {
    @Test fun togglesOnlyAstTasks() {
        val content = "- [ ] one\r\n    - [x] nested\r\n> - [ ] quoted\r\n\r\n```\r\n- [ ] code\r\n```\r\n"
        val first = content.indexOf("[ ]") + 1
        val nested = content.indexOf("[x]") + 1
        val quoted = content.indexOf("[ ]", first + 2) + 1
        val fake = content.lastIndexOf("[ ]") + 1
        assertEquals('x', toggleChannelNoteTask(content, first, true)[first], "first")
        assertEquals(' ', toggleChannelNoteTask(content, nested, false)[nested], "nested")
        assertEquals('x', toggleChannelNoteTask(content, quoted, true)[quoted], "quoted")
        assertEquals(content, toggleChannelNoteTask(content, fake, true))
        assertEquals(content, toggleChannelNoteTask(content, -1, true))
    }

    @Test fun checkboxNodeMustContainExactlyOneValidMarker() {
        assertEquals(3, channelNoteMarkerOffset("- [x] text", 2, 5))
        assertEquals(3, channelNoteMarkerOffset("- [x] text", 2, 6))
        assertNull(channelNoteMarkerOffset("- [q] text", 2, 5))
        assertNull(channelNoteMarkerOffset("- [x] text", 0, 9))
    }
    @Test fun linksPermitOnlyTheChatAllowlist() {
        assertEquals(true, channelNoteSafeHref("https://example.com"))
        assertEquals(true, channelNoteSafeHref("mailto:a@example.com"))
        assertEquals(true, channelNoteSafeHref("/channels/abc"))
        assertEquals(false, channelNoteSafeHref("//example.com"))
        assertEquals(false, channelNoteSafeHref("javascript:alert(1)"))
        assertEquals(false, channelNoteSafeHref("data:text/html,hi"))
    }
    @Test fun emptyTasksAreInteractive() {
        val input = "- [ ]\r\n- [x]\r\n"
        val first = input.indexOf("[ ]") + 1
        val second = input.indexOf("[x]") + 1
        assertEquals('x', toggleChannelNoteTask(input, first, true)[first])
        assertEquals(' ', toggleChannelNoteTask(input, second, false)[second])
        val code = "```\n- [ ]\n```"
        assertEquals(code, toggleChannelNoteTask(code, code.indexOf("[ ]") + 1, true))
    }
}
