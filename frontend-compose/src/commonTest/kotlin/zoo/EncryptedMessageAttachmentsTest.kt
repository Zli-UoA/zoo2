package zoo

import kotlin.test.*
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

class EncryptedMessageAttachmentsTest {
    private val plainChannel = buildJsonObject { put("encrypted", JsonPrimitive("f")) }
    private val encryptedChannel = buildJsonObject { put("encrypted", JsonPrimitive("t")) }

    private fun attachment(id: String, messageId: String, createdAt: String) = buildJsonObject {
        put("id", JsonPrimitive(id))
        put("message_id", JsonPrimitive(messageId))
        put("file_name", JsonPrimitive("encrypted.bin"))
        put("created_at", JsonPrimitive(createdAt))
    }

    private val rows =
        listOf(
            attachment("b", "m1", "2026-11-14T00:00:02Z"),
            attachment("a", "m1", "2026-11-14T00:00:01Z"),
            attachment("c", "m2", "2026-11-14T00:00:03Z"),
        )

    @Test
    fun plainMessagesShowTheirOwnAttachmentRowsInOrder() {
        val message = buildJsonObject {
            put("id", JsonPrimitive("m1"))
            put("ciphertext", JsonNull)
        }
        assertEquals(
            listOf("a", "b"),
            visibleMessageAttachments(plainChannel, message, null, rows).map { it.text("id") },
        )
    }

    @Test
    fun encryptedMessagesHideTheCiphertextAttachmentRows() {
        // #364: サーバーの行は暗号文（encrypted.bin）。Android で Megolm を受信できるまでは出さない。
        val inEncryptedChannel = buildJsonObject { put("id", JsonPrimitive("m1")) }
        assertEquals(
            emptyList<Row>(),
            visibleMessageAttachments(encryptedChannel, inEncryptedChannel, null, rows),
        )
        // チャンネルの行がまだ届いていなくても、本文が暗号文なら出さない（jsonb は文字列でもオブジェクトでも届く）。
        val withObject = buildJsonObject {
            put("id", JsonPrimitive("m1"))
            put("ciphertext", buildJsonObject { put("session_id", JsonPrimitive("s")) })
        }
        val withString = buildJsonObject {
            put("id", JsonPrimitive("m1"))
            put("ciphertext", JsonPrimitive("{\"session_id\":\"s\"}"))
        }
        assertEquals(emptyList<Row>(), visibleMessageAttachments(plainChannel, withObject, null, rows))
        assertEquals(emptyList<Row>(), visibleMessageAttachments(plainChannel, withString, null, rows))
    }
}
