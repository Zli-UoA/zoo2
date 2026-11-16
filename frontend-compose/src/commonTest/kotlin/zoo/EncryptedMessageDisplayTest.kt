package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class EncryptedMessageDisplayTest {
    @Test
    fun staleAndConflictingRevisionsKeepTheConfirmedBody() {
        val state = EncryptedMessageDisplayState()
        state.receive(DecryptedBody("revision 2", "e2ee.sender.unverified"))
        state.receive(DecryptedBody(notice = "e2ee.revision.stale", revision = MessageRevision.STALE))
        assertEquals("revision 2", state.decoded?.body)
        assertEquals("e2ee.revision.stale", state.decoded?.revisionNotice)
        assertEquals("e2ee.sender.unverified", state.decoded?.notice)
        state.receive(DecryptedBody("different body", "e2ee.revision.conflict", MessageRevision.CONFLICT))
        assertEquals("revision 2", state.decoded?.body)
        assertEquals(MessageRevision.CONFLICT, state.decoded?.revision)
        state.receive(DecryptedBody("revision 3"))
        assertEquals("revision 3", state.decoded?.body)
        assertNull(state.decoded?.notice)
        val unseen = EncryptedMessageDisplayState()
        unseen.receive(DecryptedBody("unconfirmed old body", revision = MessageRevision.STALE))
        assertNull(unseen.decoded?.body)
    }

    @Test
    fun displayMetadataIsNotPartOfTheDecryptionKey() {
        val row = obj("id" to "m", "channel_id" to "c", "ciphertext" to "cipher", "revision" to "2")
        assertEquals(encryptedMessagePayload(row), encryptedMessagePayload(JsonObject(row +
            ("display_data" to obj("reply_count" to 10)))))
        assertNotEquals(encryptedMessagePayload(row), encryptedMessagePayload(JsonObject(row +
            ("revision" to JsonPrimitive("3")))))
    }
}
