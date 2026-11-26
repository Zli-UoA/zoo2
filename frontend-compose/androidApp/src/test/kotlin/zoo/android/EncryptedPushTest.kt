package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import zoo.crypto.ffi.DecryptedMessage
import zoo.crypto.ffi.EncryptedMessage
import zoo.crypto.ffi.Mentions
import zoo.crypto.ffi.MessageContent
import zoo.crypto.ffi.MessagePayload
import zoo.crypto.ffi.RevisionStatus
import zoo.crypto.ffi.SessionOrigin

/** Push の `e2ee` の読み取りと、本文の置き換えの規則（Web の `protocol.ts` と同じ）。 */
class EncryptedPushTest {
    private val user = "00000000-0000-4000-8000-000000000001"
    private val sender = "00000000-0000-4000-8000-000000000004"

    private fun e2ee(recipient: String = user, extra: String = "") = """{"recipient_id":"$recipient","message":{
        "channel_id":"00000000-0000-4000-8000-000000000002","message_id":"00000000-0000-4000-8000-000000000003",
        "parent_id":null,"user_id":"$sender","message_kind":"message","epoch":3,"revision":1$extra,
        "ciphertext":{"sender_key":"k","session_id":"s","ciphertext":"c"}}}"""

    private fun decrypted(body: String) = DecryptedMessage(
        MessagePayload("m.zoo.message", "c", "m", null, 0uL, 0uL, MessageContent(body, "mfm", "message"), Mentions(emptyList(), false, false), emptyList()),
        0u, SessionOrigin.OLM, true, RevisionStatus.Latest)

    @Test
    fun parsesTheRowThatTheCoreVerifies() {
        val message = assertNotNull(pushEncryptedMessage(e2ee(), user))
        assertEquals(sender, message.userId)
        assertEquals(3uL, message.epoch)
        assertEquals(1uL, message.revision)
        assertNull(message.parentId)
    }

    @Test
    fun rejectsOtherRecipientsAndMalformedRows() {
        assertNull(pushEncryptedMessage(e2ee(recipient = "00000000-0000-4000-8000-000000000009"), user))
        assertNull(pushEncryptedMessage(e2ee(recipient = "not-a-uuid"), "not-a-uuid"))
        assertNull(pushEncryptedMessage(null, user))
        assertNull(pushEncryptedMessage("not json", user))
        assertNull(pushEncryptedMessage(e2ee().replace("\"epoch\":3", "\"epoch\":\"3\""), user))
        assertNull(pushEncryptedMessage(e2ee().replace("\"epoch\":3", "\"epoch\":-1"), user))
        assertNull(pushEncryptedMessage(e2ee().replace("\"session_id\":\"s\",", ""), user))
    }

    @Test
    fun replacesTheBodyOnlyWhenTheDeviceDecryptsIt() {
        val data = mapOf("body" to "新しいメッセージ", "e2ee" to e2ee())
        var asked: EncryptedMessage? = null
        assertEquals("本文", decryptPushBody(data, user) { asked = it; decrypted("本文") })
        assertNotNull(asked)
        // 復号できない、例外、`e2ee` がない、宛先が違うときは、サーバーの本文のまま。
        assertNull(decryptPushBody(data, user) { null })
        assertNull(decryptPushBody(data, user) { error("locked") })
        assertNull(decryptPushBody(mapOf("body" to "x"), user) { decrypted("本文") })
        assertNull(decryptPushBody(data, "00000000-0000-4000-8000-000000000009") { decrypted("本文") })
    }

    @Test
    fun cutsTheBodyAt240CodePoints() {
        val long = "😀".repeat(300)
        assertEquals("😀".repeat(240), pushBody(long))
        assertEquals("短い", pushBody("短い"))
    }
}
