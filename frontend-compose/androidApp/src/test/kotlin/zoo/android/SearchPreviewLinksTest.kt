package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import zoo.obj

class SearchPreviewLinksTest {
    @Test
    fun retainsClassifiedUrlsOnlyForTheSameMessageRevision() {
        val links = SearchPreviewLinks()
        val fullMessage = obj("channel_id" to "channel", "id" to "message", "created_at" to "2026-11-26T00:00:00Z")
        val hit = obj("channel_id" to "channel", "id" to "message", "created_at" to "2026-11-26T00:00:00", "updated_at" to null)
        val edited = obj("channel_id" to "channel", "id" to "message", "updated_at" to "2026-11-26T00:01:00")
        links.remember(fullMessage, "https://external.test/photo")
        assertEquals("https://external.test/photo", links.cached(hit))
        assertNull(links.cached(edited))
        assertNull(links.cached(fullMessage))
        links.remove("channel", "message")
        assertNull(links.cached(hit))
    }
}
