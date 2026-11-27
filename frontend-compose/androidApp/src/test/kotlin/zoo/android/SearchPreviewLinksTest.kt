package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
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
        assertNull(links.cached(hit, edited))
        assertNull(links.cached(hit, kotlinx.serialization.json.JsonObject(hit + obj("deleted_at" to "2026-11-26T00:02:00Z"))))
        assertNull(links.cached(hit, kotlinx.serialization.json.JsonObject(hit + obj("ciphertext" to "encrypted"))))
        assertNull(links.cached(fullMessage))
        links.remove("channel", "message")
        assertNull(links.cached(hit))
    }

    @Test
    fun searchChannelPermissionTracksDeletionEncryptionAndMembership() {
        val public = obj("id" to "channel", "is_private" to false)
        val private = obj("id" to "channel", "is_private" to true)
        assertTrue(searchChannelVisible(public, member = false, guest = false, encrypted = false))
        assertFalse(searchChannelVisible(public, member = false, guest = true, encrypted = false))
        assertTrue(searchChannelVisible(private, member = true, guest = true, encrypted = false))
        assertFalse(searchChannelVisible(private, member = false, guest = false, encrypted = false))
        assertFalse(searchChannelVisible(private, member = true, guest = false, encrypted = true))
        val deleted = obj("id" to "channel", "deleted_at" to "2026-11-26T00:00:00Z")
        assertFalse(searchChannelVisible(deleted, member = true, guest = false, encrypted = false))
    }

}
