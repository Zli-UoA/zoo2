package zoo

import kotlin.test.*

class HomePreferencesTest {
    @Test
    fun storedPostingControlsValidateTypesAndRemovePrivateChannelLinks() {
        val saved = parseHomePostSettings("""{"visibility":"public","apub":false,"channel_id":"general","content":"not a saved draft"}""")
        assertEquals("public", saved.text("visibility"))
        assertFalse(saved.flag("apub"))
        assertEquals("general", saved.text("channel_id"))
        assertNull(saved["content"])
        assertEquals("", parseHomePostSettings("""{"visibility":"followers","channel_id":"general"}""").text("channel_id"))
        for (raw in listOf("{", "[]", "null", """{"visibility":"invalid","apub":"false"}""")) {
            val fallback = parseHomePostSettings(raw)
            assertEquals("unlisted", fallback.text("visibility"))
            assertTrue(fallback.flag("apub"))
        }
    }

    @Test
    fun repliesRetainTheNarrowerSavedAudience() {
        assertEquals("followers", homeReplyVisibility("followers", "public"))
        assertEquals("unlisted", homeReplyVisibility("public", "unlisted"))
        assertEquals("followers", homeReplyVisibility("public", "followers"))
    }
}
