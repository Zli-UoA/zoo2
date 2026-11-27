package zoo.crypto

import java.io.File
import java.nio.file.Files
import javax.crypto.KeyGenerator
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.EncryptedMessage
import zoo.crypto.ffi.Mentions
import zoo.crypto.ffi.MessageContent
import zoo.crypto.ffi.OlmSessionStore
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.OutgoingMessage
import zoo.crypto.ffi.verifyDeviceKeys

/**
 * Push の通知の本文のための復号（#510。docs/e2ee.md の「Android の Push の復号」）。アプリが暗号ストアを開いて
 * いればそれを使い、開いていなければ写しから読むだけで復号して、何も書き戻さないことを確かめる。
 */
class NotificationDecryptionTest {
    private val root: File = Files.createTempDirectory("zoo-e2ee-push").toFile()
    private val wrapping = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private val files = CryptoStoreFiles(File(root, "alice"), { wrapping }) {}
    private val bob = DeviceAccount(BOB, BOB_DEVICE)
    private val outbound = OutboundSession(CHANNEL, 1uL, 0uL)

    @AfterTest
    fun cleanUp() {
        outbound.close()
        bob.close()
        root.deleteRecursively()
    }

    /** Alice の暗号ストアを作り、Bob の送信用セッションの鍵を Olm で受け取る。 */
    private fun openWithRoomKey(): CryptoStore {
        val store = CryptoStore.open(files, ALICE)!!.store
        store.markRegistered()
        OlmSessionStore().use { sessions ->
            val oneTimeKey = store.prepareKeyUpload(0, false)!!.oneTimeKeys.values.first()
            store.markKeysPublished()
            val target = verifyDeviceKeys(store.deviceKeys(), ALICE, store.deviceId)
            val session = bob.createOutboundOlmSession(sessions, target, oneTimeKey.toString())
            val content = bob.encryptRoomKey(sessions, session, target, outbound)
            val deviceKeys = buildJsonObject { put(BOB, buildJsonObject { put(BOB_DEVICE, Json.parseToJsonElement(bob.deviceKeys())) }) }
            val result = store.processToDevice(
                listOf(ToDeviceRow("1", BOB, "m.room.encrypted", Json.parseToJsonElement(content))), deviceKeys)
            assertEquals(1, result.roomKeys)
        }
        return store
    }

    private fun message(id: String, body: String, session: OutboundSession = outbound): EncryptedMessage =
        EncryptedMessage(CHANNEL, id, null, BOB, "message", 1uL, 0uL,
            session.encrypt(bob, OutgoingMessage(id, null, 0uL, MessageContent(body, "mfm", "message"), Mentions(emptyList(), false, false))))

    @Test
    fun decryptsWithTheStoreTheAppHoldsOpen() {
        val store = openWithRoomKey()
        try {
            val decrypted = CryptoStore.decryptForNotification(files, ALICE, message(MESSAGE, "SECRET-PUSH"))
            assertEquals("SECRET-PUSH", decrypted?.payload?.content?.body)
            // 宛先が別のユーザーなら、開いているストアでも復号しない。
            assertNull(CryptoStore.decryptForNotification(files, BOB, message(MESSAGE, "SECRET-PUSH")))
        } finally { store.close() }
    }

    @Test
    fun decryptsFromAReadOnlyCopyWhenTheAppIsClosedAndWritesNothing() {
        openWithRoomKey().close()
        val state = File(files.directory, "state.json")
        val before = state.readBytes()
        val modified = state.lastModified()
        assertEquals("SECRET-PUSH", CryptoStore.decryptForNotification(files, ALICE, message(MESSAGE, "SECRET-PUSH"))?.payload?.content?.body)
        assertContentEquals(before, state.readBytes())
        assertEquals(modified, state.lastModified())
        // 読み終えたら印を返すので、アプリはそのまま開ける。
        CryptoStore.open(files, ALICE, create = false)!!.store.close()

        // 鍵が届いていないセッションは、復号せずに「新しいメッセージ」のまま出す。
        OutboundSession(CHANNEL, 1uL, 0uL).use { other ->
            assertNull(CryptoStore.decryptForNotification(files, ALICE, message(OTHER_MESSAGE, "UNKNOWN", other)))
        }
        // 照合に失敗した（行の送信者が違う）ものも出さない。
        val forged = message(OTHER_MESSAGE, "FORGED").let { EncryptedMessage(it.channelId, it.messageId, it.parentId, ALICE, it.messageKind, it.epoch, it.revision, it.ciphertext) }
        assertNull(CryptoStore.decryptForNotification(files, ALICE, forged))
    }

    @Test
    fun aMissingStorageKeyNeverRecreatesOrDeletesTheStore() {
        openWithRoomKey().close()
        val state = File(files.directory, "state.json")
        val before = state.readBytes()
        assertTrue(File(files.directory, "storage.key").delete())
        assertNull(CryptoStore.decryptForNotification(files, ALICE, message(MESSAGE, "SECRET-PUSH")))
        assertContentEquals(before, state.readBytes())
        // 暗号ストアがないユーザーでも、何も作らない。
        val empty = CryptoStoreFiles(File(root, "nobody"), { wrapping }) {}
        assertNull(CryptoStore.decryptForNotification(empty, ALICE, message(MESSAGE, "SECRET-PUSH")))
        assertTrue(!empty.directory.exists())
    }

    private companion object {
        const val ALICE = CryptoStoreTest.ALICE
        const val BOB = CryptoStoreTest.BOB
        const val BOB_DEVICE = CryptoStoreTest.BOB_DEVICE
        const val CHANNEL = CryptoStoreTest.CHANNEL
        const val MESSAGE = "00000000-0000-4000-8000-0000000000d1"
        const val OTHER_MESSAGE = "00000000-0000-4000-8000-0000000000d2"
    }
}
