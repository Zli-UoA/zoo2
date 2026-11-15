package zoo.crypto

import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import zoo.crypto.ffi.AttachmentInfo
import zoo.crypto.ffi.EncryptedMessage
import zoo.crypto.ffi.InboundSessionStore
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.Mentions
import zoo.crypto.ffi.MessageContent
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.OutgoingMessage
import zoo.crypto.ffi.ZooCryptoException
import zoo.crypto.ffi.decryptAttachment
import zoo.crypto.ffi.encryptAttachment
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse

/** 暗号化した添付（#364）を UniFFI のバインディングで通す。 */
class EncryptedAttachmentFfiTest {
    private val plaintext = "ZOO2-E2EE-364-PLAINTEXT".encodeToByteArray()

    @Test
    fun encryptsAndDecryptsThroughTheCipher() {
        val encrypted = UniffiAttachmentCipher.encrypt(plaintext)
        assertEquals(plaintext.size, encrypted.ciphertext.size)
        assertFalse(encrypted.ciphertext.contentEquals(plaintext))
        assertEquals("v2", encrypted.file.getValue("v").jsonPrimitive.content)
        assertEquals("A256CTR", encrypted.file.getValue("key").jsonObject.getValue("alg").jsonPrimitive.content)
        assertContentEquals(plaintext, UniffiAttachmentCipher.decrypt(encrypted.file, encrypted.ciphertext))

        val tampered = encrypted.ciphertext.copyOf().also { it[0] = (it[0].toInt() xor 1).toByte() }
        assertFailsWith<ZooCryptoException.AttachmentDecryption> {
            UniffiAttachmentCipher.decrypt(encrypted.file, tampered)
        }
    }

    @Test
    fun attachmentsTravelInsideTheMegolmPayload() {
        val encrypted = encryptAttachment(plaintext)
        DeviceAccount(ALICE, ALICE_DEVICE).use { account ->
            val outbound = OutboundSession(CHANNEL, 1uL, 0uL)
            val inbound = InboundSessionStore()
            inbound.addOutbound(account, outbound)
            val ciphertext =
                outbound.encrypt(
                    account,
                    OutgoingMessage(
                        messageId = MESSAGE,
                        parentId = null,
                        revision = 0uL,
                        content = MessageContent(body = "", format = "mfm", kind = "message"),
                        mentions = Mentions(userIds = emptyList(), channel = false, here = false),
                        attachments =
                            listOf(
                                AttachmentInfo(
                                    attachmentId = ATTACHMENT,
                                    name = "secret.txt",
                                    mimetype = "text/plain",
                                    size = plaintext.size.toULong(),
                                    file = encrypted.file,
                                )
                            ),
                    ),
                )
            val decrypted =
                inbound.decrypt(
                    EncryptedMessage(
                        channelId = CHANNEL,
                        messageId = MESSAGE,
                        parentId = null,
                        userId = ALICE,
                        messageKind = "message",
                        epoch = 1uL,
                        revision = 0uL,
                        ciphertext = ciphertext,
                    )
                )
            val attachment = decrypted.payload.attachments.single()
            assertEquals("secret.txt", attachment.name)
            assertContentEquals(plaintext, decryptAttachment(attachment.file, encrypted.ciphertext))
        }
    }

    private companion object {
        const val ALICE = "00000000-0000-4000-8000-00000000000a"
        const val ALICE_DEVICE = "00000000-0000-4000-8000-0000000000a1"
        const val CHANNEL = "00000000-0000-4000-8000-0000000000c1"
        const val MESSAGE = "00000000-0000-4000-8000-0000000000d1"
        const val ATTACHMENT = "00000000-0000-4000-8000-0000000000e1"
    }
}
