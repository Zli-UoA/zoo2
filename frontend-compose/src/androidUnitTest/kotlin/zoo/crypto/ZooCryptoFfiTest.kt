package zoo.crypto

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import zoo.crypto.ffi.AddOutcome
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.EncryptedMessage
import zoo.crypto.ffi.InboundSessionStore
import zoo.crypto.ffi.Mentions
import zoo.crypto.ffi.MessageContent
import zoo.crypto.ffi.OlmSessionStore
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.OutgoingMessage
import zoo.crypto.ffi.RevisionStatus
import zoo.crypto.ffi.SessionOrigin
import zoo.crypto.ffi.ZooCryptoException
import zoo.crypto.ffi.verifyDeviceKeys
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * zoo-crypto の UniFFI バインディング（#353）を JVM の単体テストで通す。
 * ホスト向けにビルドした libzoo_crypto_ffi を JNA で読む（build.gradle.kts の `jna.library.path`）。
 */
class ZooCryptoFfiTest {
    @Test
    fun generatesDeviceKeysAndOneTimeKeys() {
        DeviceAccount(ALICE, ALICE_DEVICE).use { account ->
            val deviceKeys = Json.parseToJsonElement(account.deviceKeys()).jsonObject
            assertEquals(ALICE, deviceKeys["user_id"].toString().trim('"'))
            assertEquals(ALICE_DEVICE, deviceKeys["device_id"].toString().trim('"'))
            assertEquals(43, account.curve25519Key().length)
            assertEquals(43, account.ed25519Key().length)
            assertEquals("ed25519:$ALICE_DEVICE", account.signingKeyId())
            assertTrue(account.maxNumberOfOneTimeKeys() > 0uL)

            account.generateOneTimeKeys(5uL)
            assertEquals(5, Json.parseToJsonElement(account.oneTimeKeys()).jsonObject.size)
            account.markKeysAsPublished()
            assertEquals(0, Json.parseToJsonElement(account.oneTimeKeys()).jsonObject.size)

            // 自分の端末鍵は、自己署名の検証を通る。
            verifyDeviceKeys(account.deviceKeys(), ALICE, ALICE_DEVICE).use { verified ->
                assertEquals(account.curve25519Key(), verified.curve25519Key())
            }
        }
    }

    @Test
    fun rejectsInvalidDeviceId() {
        assertFailsWith<ZooCryptoException.InvalidId> { DeviceAccount(ALICE, "not-a-uuid") }
    }

    @Test
    fun picklesWithStorageKey() {
        val storageKey = ByteArray(32) { it.toByte() }
        DeviceAccount(ALICE, ALICE_DEVICE).use { account ->
            val pickle = account.pickle(storageKey)
            DeviceAccount.fromPickle(pickle, storageKey, ALICE, ALICE_DEVICE).use { restored ->
                assertEquals(account.curve25519Key(), restored.curve25519Key())
                assertEquals(account.ed25519Key(), restored.ed25519Key())
            }
            assertFailsWith<ZooCryptoException.Pickle> {
                DeviceAccount.fromPickle(pickle, ByteArray(32), ALICE, ALICE_DEVICE)
            }
            assertFailsWith<ZooCryptoException.PickleOwnerMismatch> {
                DeviceAccount.fromPickle(pickle, storageKey, BOB, ALICE_DEVICE)
            }
        }
    }

    @Test
    fun encryptsAndDecryptsOwnMegolmMessage() {
        val account = DeviceAccount(ALICE, ALICE_DEVICE)
        val outbound = OutboundSession(CHANNEL, 1uL, 0uL)
        val inbound = InboundSessionStore()
        assertEquals(AddOutcome.ADDED, inbound.addOutbound(account, outbound))

        val ciphertext = outbound.encrypt(account, message("こんにちは"))
        val decrypted = inbound.decrypt(row(ciphertext))
        assertEquals("こんにちは", decrypted.payload.content.body)
        assertEquals(0u, decrypted.messageIndex)
        assertEquals(SessionOrigin.OLM, decrypted.origin)
        assertEquals(RevisionStatus.Latest, decrypted.revision)
    }

    @Test
    fun distributesRoomKeyOverOlmAndDecryptsMegolm() {
        val alice = DeviceAccount(ALICE, ALICE_DEVICE)
        val bob = DeviceAccount(BOB, BOB_DEVICE)
        bob.generateOneTimeKeys(1uL)
        val oneTimeKey = Json.parseToJsonElement(bob.oneTimeKeys()).jsonObject.values.single().toString()
        bob.markKeysAsPublished()

        // Alice が Bob の端末に、Olm で Megolm セッション鍵を送る。
        val aliceSessions = OlmSessionStore()
        val bobKeys = verifyDeviceKeys(bob.deviceKeys(), BOB, BOB_DEVICE)
        val sessionId = alice.createOutboundOlmSession(aliceSessions, bobKeys, oneTimeKey)
        val outbound = OutboundSession(CHANNEL, 1uL, 0uL)
        val toDevice = alice.encryptRoomKey(aliceSessions, sessionId, bobKeys, outbound)

        // Bob が受け取り、検証して保存する。
        val bobSessions = OlmSessionStore()
        val decrypted = bob.decryptToDevice(bobSessions, toDevice)
        assertEquals(1uL, bobSessions.len())
        assertEquals("zoo.room_key", decrypted.eventType())
        val roomKey = decrypted.verifyRoomKey(bob, alice.deviceKeys(), null)
        assertFalse(roomKey.senderVerified())
        val inbound = InboundSessionStore()
        assertEquals(AddOutcome.ADDED, inbound.addRoomKey(roomKey))
        assertFailsWith<ZooCryptoException.AlreadyConsumed> { inbound.addRoomKey(roomKey) }

        // Alice の Megolm のメッセージを Bob が読める。
        val ciphertext = outbound.encrypt(alice, message("hello"))
        val message = inbound.decrypt(row(ciphertext))
        assertEquals("hello", message.payload.content.body)
        assertEquals(MESSAGE, message.payload.messageId)
        assertFalse(message.senderVerified)
    }

    @Test
    fun reportsMissingRoomKey() {
        val account = DeviceAccount(ALICE, ALICE_DEVICE)
        val outbound = OutboundSession(CHANNEL, 1uL, 0uL)
        val ciphertext = outbound.encrypt(account, message("x"))
        val error = assertFailsWith<ZooCryptoException.MissingRoomKey> { InboundSessionStore().decrypt(row(ciphertext)) }
        assertEquals(outbound.sessionId(), error.sessionId)
    }

    private fun message(body: String) = OutgoingMessage(
        messageId = MESSAGE,
        parentId = null,
        revision = 0uL,
        content = MessageContent(body = body, format = "mfm", kind = "message"),
        mentions = Mentions(userIds = emptyList(), channel = false, here = false),
    )

    private fun row(ciphertext: String) = EncryptedMessage(
        channelId = CHANNEL,
        messageId = MESSAGE,
        parentId = null,
        userId = ALICE,
        messageKind = "message",
        epoch = 1uL,
        revision = 0uL,
        ciphertext = ciphertext,
    )

    private companion object {
        const val ALICE = "00000000-0000-4000-8000-00000000000a"
        const val ALICE_DEVICE = "00000000-0000-4000-8000-0000000000a1"
        const val BOB = "00000000-0000-4000-8000-00000000000b"
        const val BOB_DEVICE = "00000000-0000-4000-8000-0000000000b1"
        const val CHANNEL = "00000000-0000-4000-8000-0000000000c1"
        const val MESSAGE = "00000000-0000-4000-8000-0000000000d1"
    }
}
