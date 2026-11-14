package zoo.crypto

import java.io.File
import java.nio.file.Files
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.OlmSessionStore
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.verifyDeviceKeys
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Android の暗号ストア（#359）。JVM では Android Keystore を使えないので、保存鍵はソフトウェアの
 * AES-256-GCM の鍵で包む（[StorageKeyStoreTest] と同じ）。
 */
class CryptoStoreTest {
    private val root: File = Files.createTempDirectory("zoo-crypto-store").toFile()
    private var wrapping: SecretKey = softwareKey()
    private var wrappingDeleted = 0
    private val files =
        CryptoStoreFiles(File(root, "store"), { wrapping }) {
            wrappingDeleted++
            wrapping = softwareKey()
        }

    @AfterTest
    fun cleanUp() {
        root.deleteRecursively()
    }

    private fun open(create: Boolean = true, replaced: MutableList<String> = mutableListOf()) =
        CryptoStore.open(files, BOB, create) { id, _ -> replaced += id }

    @Test
    fun createsDeviceOnceAndRestoresIt() {
        val first = assertNotNull(open())
        assertNull(first.replacedDeviceId)
        val deviceId = first.store.deviceId
        first.store.use { store ->
            assertFalse(store.isRegistered)
            val request = store.registrationRequest()
            assertEquals(BOB, request["device_keys"]!!.jsonObject["user_id"]!!.jsonPrimitive.content)
            assertEquals(50, request["one_time_keys"]!!.jsonObject.size)
            assertEquals(1, request["fallback_keys"]!!.jsonObject.size)
            // 同じストアは同時に 2 つ開けない。
            assertFailsWith<CryptoStoreBusyException> { open() }
            store.markRegistered()
            assertTrue(store.registrationRequest()["one_time_keys"]!!.jsonObject.isEmpty())
        }
        val state = File(files.directory, "state.json").readText()
        // 秘密は平文で置かない（端末アカウントは封筒の中）。
        assertFalse("ed25519" in state)
        open()!!.store.use { store ->
            assertEquals(deviceId, store.deviceId)
            assertTrue(store.isRegistered)
            assertEquals(0, store.pendingBackupCount())
        }
        assertEquals(deviceId, CryptoStore.storedDeviceId(files, BOB))
        assertNull(CryptoStore.storedDeviceId(files, ALICE))
        // 別のユーザーでは開かない。
        assertFailsWith<IllegalStateException> { CryptoStore.open(files, ALICE) }
    }

    @Test
    fun doesNotCreateWhenAskedNotTo() {
        assertNull(open(create = false))
        assertFalse(File(files.directory, "state.json").exists())
        // 閉じたことになっていて、あとで開ける。
        open()!!.store.close()
    }

    @Test
    fun replacesDeviceWhenStorageKeyIsLost() {
        val before = open()!!.store.use { it.deviceId }
        File(files.directory, "storage.key").delete()
        val replaced = mutableListOf<String>()
        open(replaced = replaced)!!.let { opened ->
            opened.store.use { assertNotEquals(before, it.deviceId) }
            assertEquals(before, opened.replacedDeviceId)
        }
        assertEquals(listOf(before), replaced)
    }

    @Test
    fun replacesDeviceWhenWrappingKeyChanged() {
        val before = open()!!.store.use { it.deviceId }
        // 別の端末への復元などで、Keystore の鍵が変わった。
        wrapping = softwareKey()
        val opened = open()!!
        opened.store.close()
        assertEquals(before, opened.replacedDeviceId)
        assertEquals(1, wrappingDeleted)
    }

    @Test
    fun keepsStoreWhenKeystoreFailsTemporarily() {
        open()!!.store.close()
        val failing = CryptoStoreFiles(files.directory, { throw java.security.KeyStoreException("busy") }) {}
        assertFailsWith<java.security.KeyStoreException> { CryptoStore.open(failing, BOB) }
        assertTrue(File(files.directory, "state.json").exists())
        open()!!.store.close()
    }

    @Test
    fun replenishesOneTimeKeys() {
        open()!!.store.use { store ->
            store.markRegistered()
            // サーバーに最大数があり、フォールバック鍵も使われていない。
            assertNull(store.prepareKeyUpload(50, hasUnusedFallbackKey = true))
            val upload = assertNotNull(store.prepareKeyUpload(45, hasUnusedFallbackKey = false))
            assertEquals(5, upload.oneTimeKeys.size)
            assertEquals(1, upload.fallbackKeys.size)
            // 応答をなくしたら、同じ鍵を送り直す。
            assertEquals(upload.oneTimeKeys, store.prepareKeyUpload(45, hasUnusedFallbackKey = false)!!.oneTimeKeys)
            store.markKeysPublished()
            assertNull(store.prepareKeyUpload(50, hasUnusedFallbackKey = true))
        }
    }

    @Test
    fun processesRoomKeysOnceAndKeepsThemAcrossRestarts() {
        val alice = DeviceAccount(ALICE, ALICE_DEVICE)
        val aliceSessions = OlmSessionStore()
        val store = open()!!.store
        val request = store.registrationRequest()
        store.markRegistered()
        val bobKeys = verifyDeviceKeys(request["device_keys"].toString(), BOB, store.deviceId)
        val oneTimeKey = request["one_time_keys"]!!.jsonObject.values.first()
        val sessionId = alice.createOutboundOlmSession(aliceSessions, bobKeys, oneTimeKey.toString())
        val deviceKeys = buildJsonObject {
            put(ALICE, buildJsonObject { put(ALICE_DEVICE, Json.parseToJsonElement(alice.deviceKeys())) })
        }
        fun roomKey(id: String) =
            ToDeviceRow(
                id, ALICE, "m.room.encrypted",
                Json.parseToJsonElement(
                    alice.encryptRoomKey(aliceSessions, sessionId, bobKeys, OutboundSession(CHANNEL, 1uL, 0uL))
                ),
            )

        val first = roomKey("10")
        val garbage = ToDeviceRow("11", ALICE, "m.room.encrypted", JsonObject(mapOf("x" to JsonPrimitive(1))))
        val plain = ToDeviceRow("12", ALICE, "m.other", JsonObject(emptyMap()))
        val result = store.processToDevice(listOf(first, garbage, plain), deviceKeys)
        assertEquals(listOf("10", "11", "12"), result.processed)
        assertEquals(1, result.roomKeys)
        assertEquals(2, result.dropped)
        assertEquals(1, result.newOlmSessions)
        assertEquals(1, store.pendingBackupCount())

        // ack の前に終了した：同じメッセージは復号し直さない。
        store.close()
        val reopened = open()!!.store
        val again = reopened.processToDevice(listOf(first), deviceKeys)
        assertEquals(listOf("10"), again.processed)
        assertEquals(0, again.roomKeys)
        assertEquals(1, reopened.pendingBackupCount())
        reopened.acknowledged(again.processed)

        // 戻した Olm セッションで、次の鍵も受け取れる。
        val second = reopened.processToDevice(listOf(roomKey("13")), deviceKeys)
        assertEquals(1, second.roomKeys)
        assertEquals(0, second.newOlmSessions)
        assertEquals(2, reopened.pendingBackupCount())
        reopened.close()
        alice.close()
        aliceSessions.close()
    }

    @Test
    fun resavesAlreadyProcessedRowsBeforeAck() {
        val original = CryptoStore.directorySync
        val synced = mutableListOf<File>()
        val store = open()!!.store
        val row = ToDeviceRow("30", ALICE, "m.other", JsonObject(emptyMap()))
        try {
            // rename のあとで同期に失敗した：保存の失敗になり、ID を返さない。
            CryptoStore.directorySync = { throw java.io.IOException("sync failed") }
            assertFailsWith<java.io.IOException> { store.processToDevice(listOf(row), JsonObject(emptyMap())) }
            store.close()
            // 開き直すと、新しい state.json の印は読める。それでも同期を確かめ直すまで ID を返さない。
            val reopened = open()!!.store
            assertFailsWith<java.io.IOException> { reopened.processToDevice(listOf(row), JsonObject(emptyMap())) }
            CryptoStore.directorySync = { synced += it }
            assertEquals(listOf("30"), reopened.processToDevice(listOf(row), JsonObject(emptyMap())).processed)
            assertEquals(listOf(files.directory.absoluteFile), synced.map { it.absoluteFile })
            reopened.close()
        } finally {
            CryptoStore.directorySync = original
        }
    }

    @Test
    fun destroyRemovesFilesAndWrappingKey() {
        val store = open()!!.store
        store.destroy()
        assertFalse(files.directory.exists())
        assertEquals(1, wrappingDeleted)
        // 開けなかったストアも消せる。
        open()!!.store.close()
        CryptoStore.destroy(files)
        assertFalse(files.directory.exists())
        assertEquals(2, wrappingDeleted)
    }

    @Test
    fun closedStoreDoesNotDestroyStoreOpenedAgain() {
        val first = open()!!.store
        first.close()
        // ほかの処理が開き直したストアは、閉じたものから消さない。
        val second = open()!!.store
        assertFailsWith<CryptoStoreBusyException> { first.destroy() }
        assertTrue(File(files.directory, "state.json").exists())
        second.destroy()
        assertFalse(files.directory.exists())
        // 消し終えたら、また開ける。
        open()!!.store.close()
    }

    @Test
    fun readsToDeviceRowsFromShape() {
        val row =
            toDeviceRow(
                buildJsonObject {
                    put("id", JsonPrimitive("42"))
                    put("sender_user_id", JsonPrimitive(ALICE))
                    put("type", JsonPrimitive("m.room.encrypted"))
                    put("content", JsonPrimitive("""{"algorithm":"m.olm.v1.curve25519-aes-sha2"}"""))
                }
            )
        assertEquals("42", row.id)
        assertEquals("m.olm.v1.curve25519-aes-sha2", row.content.jsonObject["algorithm"]!!.jsonPrimitive.content)
        val broken = toDeviceRow(buildJsonObject { put("content", JsonPrimitive("{")) })
        assertEquals(kotlinx.serialization.json.JsonNull, broken.content)
    }

    @Test
    fun pendingDeletionsKeepOtherUsers() {
        val pending = PendingDeviceDeletions(File(root, "pending.json"))
        val a = PendingDeviceDeletion("https://a", ALICE, ALICE_DEVICE)
        val b = PendingDeviceDeletion("https://a", BOB, BOB_DEVICE)
        pending.add(a)
        pending.add(b)
        pending.add(a)
        assertEquals(listOf(a, b), pending.all())
        pending.remove(a)
        assertEquals(listOf(b), pending.all())
        pending.remove(b)
        assertTrue(pending.all().isEmpty())
    }

    @Test
    fun pendingDeletionsKeepUnconfirmedFlagAndCount() {
        val pending = PendingDeviceDeletions(File(root, "pending.json"))
        val a = PendingDeviceDeletion("https://a", ALICE, ALICE_DEVICE, unconfirmed = true)
        pending.add(a)
        pending.update(a.copy(notFound = 1))
        // 同じ端末を足し直しても、記録の状態は消さない。
        pending.add(a.copy(unconfirmed = false))
        assertEquals(listOf(a.copy(notFound = 1)), pending.all())
        pending.remove(a.copy(unconfirmed = false))
        assertTrue(pending.all().isEmpty())
    }

    @Test
    fun unreadablePendingDeletionsAreNotOverwritten() {
        val file = File(root, "pending.json")
        file.writeText("{broken")
        val pending = PendingDeviceDeletions(file)
        assertFailsWith<Exception> { pending.all() }
        assertFailsWith<Exception> { pending.add(PendingDeviceDeletion("https://a", BOB, BOB_DEVICE)) }
        assertFailsWith<Exception> { pending.remove(PendingDeviceDeletion("https://a", BOB, BOB_DEVICE)) }
        assertEquals("{broken", file.readText())
    }

    private fun softwareKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    internal companion object {
        const val ALICE = "00000000-0000-4000-8000-00000000000a"
        const val ALICE_DEVICE = "00000000-0000-4000-8000-0000000000a1"
        const val BOB = "00000000-0000-4000-8000-00000000000b"
        const val BOB_DEVICE = "00000000-0000-4000-8000-0000000000b1"
        const val CHANNEL = "00000000-0000-4000-8000-0000000000c1"
    }
}
