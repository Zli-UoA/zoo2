package zoo

import kotlin.test.*

class HomeComposerStateTest {
    @Test
    fun postLimitCountsUnicodeCodePointsRatherThanUtf16Units() {
        assertEquals(4, homePostLength("猫🙂a𠮷"))
        assertEquals(1000, homePostLength("🙂".repeat(1000)))
        assertEquals(1001, homePostLength("🙂".repeat(1000) + "a"))
        // Like Array.from, combined emoji count their constituent code points.
        assertEquals(3, homePostLength("👩‍💻"))
    }

    @Test
    fun closingAndReplyingKeepOneDraftAndReplyStartsWithoutChannel() {
        val state = HomeComposerState()
        state.content = "途中まで書いた下書き"
        state.channelId = "general"
        var focusedWhileOpen = false
        state.focusOnOpen = { focusedWhileOpen = state.opened }
        state.open()
        assertTrue(
            focusedWhileOpen,
            "The opening gesture focuses the retained editor synchronously",
        )
        state.close()
        assertFalse(state.opened, "Closing removes the form from the visible interaction tree")
        assertEquals("途中まで書いた下書き", state.content)
        assertEquals("general", state.channelId)
        state.reply(obj("id" to "remote-post", "visibility" to "followers", "apub" to true))
        assertTrue(state.opened)
        assertEquals("途中まで書いた下書き", state.content)
        assertEquals("", state.channelId)
        assertEquals("followers", state.visibility)
        assertEquals("remote-post", state.replyTarget?.text("id"))
    }

    @Test
    fun sendingPreventsCloseAndReplacingTheReply() {
        val state = HomeComposerState()
        state.reply(obj("id" to "first", "visibility" to "public"))
        var focusCalls = 0
        state.focusOnOpen = { focusCalls++ }
        state.sending = true
        state.close()
        state.reply(obj("id" to "second", "visibility" to "followers"))
        assertTrue(state.opened)
        assertEquals("first", state.replyTarget?.text("id"))
        assertEquals("public", state.visibility)
        assertEquals(0, focusCalls)
    }
}
