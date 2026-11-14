package zoo.crypto

import java.io.File
import java.io.IOException
import java.nio.file.Files
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertNotEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import zoo.ApiException
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.OlmSessionStore
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.PasskeySlot
import zoo.crypto.ffi.ZooCryptoException
import zoo.crypto.ffi.verifyDeviceKeys

/**
 * 初回設定と解錠（#360）。TUI の `tests/e2ee_setup.rs`、React の `setupController.test.ts` と同じく、状態を持つ
 * 模擬サーバーで、保存の途中で止めて開け直しても同じ鍵で最後まで終わること、再認証の 403 から同じ鍵で
 * やり直せること、同時の初回設定、鍵バックアップへの送信の条件を確かめる。暗号は実際の zoo-crypto（UniFFI）。
 */
class E2eeSetupTest {
    private val root: File = Files.createTempDirectory("zoo-e2ee-setup").toFile()
    private val server = FakeServer()
    private val opened = mutableListOf<CryptoStore>()

    @AfterTest
    fun cleanUp() {
        opened.forEach { it.close() }
        root.deleteRecursively()
    }

    /** 端末 1 つ（暗号ストアと、それを使う [E2eeSetup]）。 */
    private inner class Device(name: String) {
        private var wrapping: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
        private val files = CryptoStoreFiles(File(root, name), { wrapping }) {}
        var store: CryptoStore = openStore()
        val setup = E2eeSetup(server, ALICE, store = { store })

        private fun openStore(): CryptoStore =
            CryptoStore.open(files, ALICE)!!.store.also {
                if (!it.isRegistered) it.markRegistered()
                opened += it
            }

        /** アプリを終了して開き直す（メモリーの鍵は消える）。 */
        fun restart() {
            store.close()
            store = openStore()
        }
    }

    @Test
    fun firstSetupSavesEverythingAfterReauth() = runBlocking {
        val device = Device("a")
        assertEquals(E2eeStatus.NotSetUp, device.setup.loadStatus())
        val start = device.setup.begin()
        assertEquals(48 + 11, start.recoveryKey.length)
        // 再認証なしの保存は 403。同じ鍵のまま、再認証してやり直せる。
        assertFailsWith<E2eeSetupException.ReauthRequired> { device.setup.completeSetup(start.session, null) }
        assertNull(server.defaultKeyId)
        device.setup.completeSetup(start.session, "id-token")
        assertEquals(start.session.keyId(), server.defaultKeyId)
        assertEquals(listOf("id-token", "id-token"), server.reauthHeaders)
        assertNotNull(server.keys["master_key"])
        assertEquals(1, server.signatures.size)
        assertEquals(1, server.versions.size)
        assertTrue(device.store.hasOwnKeys)
        assertEquals(1L, device.store.secretsRevision)
        assertIs<E2eeStatus.Unlocked>(device.setup.loadStatus())
        // 自分のマスター鍵を、配布先の判定の記録に入れた。
        assertNotNull(device.store.trustStore.ownMasterKey())
        start.session.close()
    }

    @Test
    fun resumesFromEveryStopWithTheRecoveryKey() = runBlocking {
        for (stop in listOf("cross_signing", "signatures", "versions")) {
            server.reset()
            val device = Device("resume-$stop")
            val start = device.setup.begin()
            server.failNext += stop
            assertFailsWith<IOException> { device.setup.completeSetup(start.session, "t") }
            val master = server.keys["master_key"]
            start.session.close()
            // アプリを終了して開き直し、控えたリカバリーキーで解錠して続ける。
            device.restart()
            assertIs<E2eeStatus.Locked>(device.setup.loadStatus())
            val unlocked = device.setup.unlock(UnlockMethod.Recovery(start.recoveryKey))
            assertEquals(stop == "cross_signing", unlocked.needsReauth, stop)
            if (unlocked.needsReauth) device.setup.saveRemaining(unlocked.session, unlocked.secretsRevision, "t")
            unlocked.session.close()
            assertIs<E2eeStatus.Unlocked>(device.setup.loadStatus(), stop)
            if (master != null) assertEquals(master, server.keys["master_key"], stop)
            assertEquals(1, server.versions.size, stop)
        }
    }

    @Test
    fun anotherDeviceUnlocksAndSignsItself() = runBlocking {
        val first = Device("first")
        val start = first.setup.begin()
        first.setup.completeSetup(start.session, "t")
        start.session.close()

        val second = Device("second")
        assertEquals(LockReason.NEW, (second.setup.loadStatus() as E2eeStatus.Locked).reason)
        // 形式の誤りと、鍵の違いを分ける。
        assertFailsWith<ZooCryptoException.InvalidRecoveryKey> { second.setup.unlock(UnlockMethod.Recovery("abcd")) }
        val other = second.setup.begin()
        assertFailsWith<ZooCryptoException.KeySlotUnlock> { second.setup.unlock(UnlockMethod.Recovery(other.recoveryKey)) }
        other.session.close()

        val unlocked = second.setup.unlock(UnlockMethod.Recovery(start.recoveryKey.replace(" ", "")))
        assertFalse(unlocked.needsReauth)
        unlocked.session.close()
        assertEquals(2, server.signatures.size)
        assertEquals(listOf(server.recoverySlotId), server.usedSlots)
        assertIs<E2eeStatus.Unlocked>(second.setup.loadStatus())
    }

    @Test
    fun concurrentSetupLosesToTheFirstSave() = runBlocking {
        val first = Device("first")
        val second = Device("second")
        val a = first.setup.begin()
        val b = second.setup.begin()
        first.setup.completeSetup(a.session, "t")
        assertFailsWith<E2eeSetupException.Conflict> { second.setup.completeSetup(b.session, "t") }
        assertEquals(a.session.keyId(), server.defaultKeyId)
        assertFalse(second.store.hasOwnKeys)
        // 先の端末のリカバリーキーで解錠できる。
        second.setup.unlock(UnlockMethod.Recovery(a.recoveryKey)).session.close()
        assertIs<E2eeStatus.Unlocked>(second.setup.loadStatus())
        a.session.close()
        b.session.close()
    }

    @Test
    fun lostSecretStorageResponseIsReadBack() = runBlocking {
        val device = Device("a")
        val start = device.setup.begin()
        // SSSS は保存されたが、応答をなくした。送り直すと 409 になるので、読み直して続ける。
        server.loseResponse += "secret_storage"
        assertFailsWith<IOException> { device.setup.completeSetup(start.session, "t") }
        assertEquals(start.session.keyId(), server.defaultKeyId)
        device.setup.completeSetup(start.session, "t")
        assertIs<E2eeStatus.Unlocked>(device.setup.loadStatus())
        start.session.close()
    }

    @Test
    fun differentServerKeysAreAMismatch() = runBlocking {
        val device = Device("a")
        val start = device.setup.begin()
        server.failNext += "cross_signing"
        assertFailsWith<IOException> { device.setup.completeSetup(start.session, "t") }
        // 別の鍵のクロス署名鍵が置かれた（失効の途中か、改ざん）。署名しない。
        val other = device.setup.begin()
        val plan = other.session.plan(server.keysQuery().toString(), server.versionsJson().toString(), device.store.deviceKeys())
        server.keys.putAll(Json.parseToJsonElement(plan.crossSigning!!).jsonObject.filterKeys { it.endsWith("_key") })
        assertFailsWith<E2eeSetupException.ServerKeyMismatch> {
            device.setup.unlock(UnlockMethod.Recovery(start.recoveryKey))
        }
        assertEquals(0, server.signatures.size)
        assertFalse(device.store.hasOwnKeys)
        start.session.close()
        other.session.close()
    }

    @Test
    fun uploadsBackupOnlyAfterUnlockAndChecksTheVersion() = runBlocking {
        val first = Device("first")
        val start = first.setup.begin()
        first.setup.completeSetup(start.session, "t")
        start.session.close()

        val second = Device("second")
        receiveRoomKey(second.store)
        assertEquals(1, second.store.pendingBackupCount())
        // 解錠する前は送らない（今の版も取りに行かない）。
        assertEquals(BackupUploadResult.Done(0), second.setup.uploadKeyBackup())
        assertEquals(0, server.backupUploads.size)
        assertNull(second.setup.runStartupCheck())

        second.setup.unlock(UnlockMethod.Recovery(start.recoveryKey)).session.close()
        assertEquals(listOf(1), server.backupUploads)
        assertEquals(0, second.store.pendingBackupCount())
        assertEquals(BackupUploadResult.Done(0), second.setup.uploadKeyBackup())

        // 新しいセッションは、起動時の確認のあとで送る。
        receiveRoomKey(second.store)
        second.restart()
        assertIs<E2eeStatus.Unlocked>(second.setup.runStartupCheck())
        assertEquals(listOf(1, 1), server.backupUploads)

        // 自分のマスター鍵で署名していない版には送らない。
        val stranger = second.setup.begin()
        val foreign =
            stranger.session.plan(
                FakeServer().keysQuery().toString(), FakeServer().versionsJson().toString(), second.store.deviceKeys(),
            )
        server.addVersion(Json.parseToJsonElement(foreign.backupVersion!!).jsonObject)
        stranger.session.close()
        receiveRoomKey(second.store)
        assertIs<BackupUploadResult.Rejected>(second.setup.uploadKeyBackup())
        assertEquals(listOf(1, 1), server.backupUploads)
        assertEquals(1, second.store.pendingBackupCount())
    }

    @Test
    fun conflictingBackupUploadStopsUntilUnlocked() = runBlocking {
        val device = Device("a")
        val start = device.setup.begin()
        device.setup.completeSetup(start.session, "t")
        start.session.close()
        receiveRoomKey(device.store)
        server.backupConflict = "E2EE_SSSS_CONFLICT"
        assertEquals(BackupUploadResult.Conflict("E2EE_SSSS_CONFLICT"), device.setup.uploadKeyBackup())
        // 記録した secrets_revision を消したので、解錠し直しを求める。
        assertNull(device.store.secretsRevision)
        assertEquals(LockReason.MISMATCH, (device.setup.loadStatus() as E2eeStatus.Locked).reason)
        assertEquals(1, device.store.pendingBackupCount())
    }

    @Test
    fun setupAndUnlockWithPasskey() = runBlocking {
        val passkey = FakePasskey()
        val first = Device("first")
        val start = first.setup.begin()
        first.setup.addSetupPasskey(start.session, passkey, "account.zli.works", null)
        first.setup.completeSetup(start.session, "t")
        start.session.close()
        assertEquals(2, server.slots.size)

        val second = Device("second")
        val storage = (second.setup.loadStatus() as E2eeStatus.Locked).storage
        assertEquals(listOf(passkey.credentialId), passkeySlots(ALICE, storage).map { it.credentialId })
        second.setup.unlock(UnlockMethod.Passkey(passkey, "account.zli.works")).session.close()
        assertEquals("account.zli.works", passkey.lastRpId)
        assertIs<E2eeStatus.Unlocked>(second.setup.loadStatus())
        assertEquals(1, server.usedSlots.size)
        assertNotEquals(server.recoverySlotId, server.usedSlots.single())

        // 別のパスキーを選んだら、スロットがない。
        val third = Device("third")
        assertFailsWith<PasskeyException> {
            third.setup.unlock(UnlockMethod.Passkey(FakePasskey(credentialId = "b3RoZXI"), "account.zli.works"))
        }
    }

    /** ほかの端末から、Megolm セッションの鍵を受け取る（鍵バックアップの送信待ちが 1 件増える）。 */
    private fun receiveRoomKey(store: CryptoStore) {
        DeviceAccount(BOB, BOB_DEVICE).use { bob ->
            OlmSessionStore().use { sessions ->
                val upload = store.prepareKeyUpload(0, false)
                val target =
                    verifyDeviceKeys(store.deviceKeys(), ALICE, store.deviceId)
                val oneTimeKey = upload!!.oneTimeKeys.values.first()
                store.markKeysPublished()
                val sessionId = bob.createOutboundOlmSession(sessions, target, oneTimeKey.toString())
                val content = bob.encryptRoomKey(sessions, sessionId, target, OutboundSession(CHANNEL, 1uL, 0uL))
                val deviceKeys = buildJsonObject {
                    put(BOB, buildJsonObject { put(BOB_DEVICE, Json.parseToJsonElement(bob.deviceKeys())) })
                }
                val result =
                    store.processToDevice(
                        listOf(ToDeviceRow(java.util.UUID.randomUUID().toString(), BOB, "m.room.encrypted", Json.parseToJsonElement(content))),
                        deviceKeys,
                    )
                assertEquals(1, result.roomKeys)
            }
        }
    }

    /** PRF を返す仮の認証器（credential ID ごとに決まった出力）。 */
    private class FakePasskey(val credentialId: String = "Y3JlZGVudGlhbA") : PasskeyClient {
        var lastRpId: String? = null
        private val outputs = mutableMapOf<String, ByteArray>()

        private fun output(salt: ByteArray) =
            java.security.MessageDigest.getInstance("SHA-256").digest(credentialId.encodeToByteArray() + salt)

        override suspend fun prfForNewSlot(rpId: String, prfSalt: ByteArray): PrfResult {
            lastRpId = rpId
            return PrfResult(credentialId, output(prfSalt))
        }

        override suspend fun prfForUnlock(rpId: String, slots: List<PasskeySlot>): PrfResult {
            lastRpId = rpId
            val slot =
                slots.firstOrNull { it.credentialId == credentialId }
                    ?: return PrfResult(credentialId, ByteArray(32))
            return PrfResult(credentialId, output(slot.prfSalt))
        }
    }

    /** 初回設定と解錠の API を模した、状態を持つサーバー（docs/e2ee.md の API の要点だけ）。 */
    private class FakeServer : E2eeSetupApi {
        var defaultKeyId: String? = null
        var revision = 0L
        var storageKey: JsonElement = JsonNull
        var secrets: JsonObject = JsonObject(emptyMap())
        val slots = mutableListOf<JsonObject>()
        val keys = mutableMapOf<String, JsonElement>()
        val signatures = mutableListOf<JsonObject>()
        val versions = mutableListOf<JsonObject>()
        val reauthHeaders = mutableListOf<String>()
        val usedSlots = mutableListOf<String>()
        val backupUploads = mutableListOf<Int>()
        val failNext = mutableSetOf<String>()
        val loseResponse = mutableSetOf<String>()
        var backupConflict: String? = null

        val recoverySlotId: String?
            get() = slots.firstOrNull { it["kind"]?.jsonPrimitive?.content == "recovery" }?.get("id")?.jsonPrimitive?.content

        fun reset() {
            defaultKeyId = null
            revision = 0
            slots.clear()
            keys.clear()
            signatures.clear()
            versions.clear()
            failNext.clear()
        }

        private fun fail(op: String) {
            if (failNext.remove(op)) throw IOException("通信に失敗しました（$op）")
        }

        private fun requireReauth(reauth: String?) {
            if (reauth == null) throw ApiException(403, code("E2EE_REAUTH_REQUIRED"))
            reauthHeaders += reauth
        }

        private fun code(code: String) = buildJsonObject { put("code", code) }

        private fun checkDefaultKey(body: JsonObject) {
            val expected = body["expected_default_key_id"] ?: return
            if (expected.jsonPrimitive.content != defaultKeyId) throw ApiException(409, code("E2EE_SSSS_CONFLICT"))
        }

        override suspend fun secretStorage(): JsonObject = buildJsonObject {
            put("default_key_id", defaultKeyId?.let(::JsonPrimitive) ?: JsonNull)
            if (defaultKeyId != null) {
                put("keys", buildJsonObject { put(defaultKeyId!!, storageKey) })
                put("secrets", secrets)
                put("secrets_revision", revision)
            } else {
                put("keys", JsonObject(emptyMap()))
                put("secrets", JsonObject(emptyMap()))
                put("secrets_revision", JsonNull)
            }
            put("slots", JsonArray(slots))
        }

        override suspend fun createSecretStorageKey(body: JsonObject, reauth: String?): Long? {
            requireReauth(reauth)
            fail("secret_storage")
            val keyId = body["key_id"]!!.jsonPrimitive.content
            if (defaultKeyId != null) throw ApiException(409, code("E2EE_SSSS_CONFLICT"))
            defaultKeyId = keyId
            storageKey = body["key"]!!
            secrets =
                JsonObject(
                    body["secrets"]!!.jsonObject.mapValues { (_, secret) ->
                        JsonObject(secret.jsonObject + ("key_id" to JsonPrimitive(keyId)))
                    }
                )
            slots += body["slots"]!!.jsonArray.map { it.jsonObject }
            revision += 1
            if (loseResponse.remove("secret_storage")) throw IOException("応答をなくしました")
            return revision
        }

        override suspend fun addKeySlot(slot: JsonObject, reauth: String?) {
            if (slots.none { it["id"] == slot["id"] }) slots += slot
        }

        override suspend fun markKeySlotUsed(slotId: String) {
            usedSlots += slotId
        }

        fun keysQuery(): JsonObject = buildJsonObject {
            put("device_keys", JsonObject(emptyMap()))
            for ((field, target) in listOf(
                "master_key" to "master_keys",
                "self_signing_key" to "self_signing_keys",
                "user_signing_key" to "user_signing_keys",
            )) {
                put(target, buildJsonObject { keys[field]?.let { put(ALICE, it) } })
            }
            put("signatures", JsonArray(signatures))
        }

        override suspend fun queryOwnKeys(userId: String) = keysQuery()

        fun versionsJson() = buildJsonObject { put("versions", JsonArray(versions)) }

        override suspend fun backupVersions() = versionsJson()

        override suspend fun currentBackupVersion(): JsonObject? = versions.lastOrNull()

        override suspend fun uploadCrossSigningKeys(body: JsonObject, reauth: String?) {
            requireReauth(reauth)
            checkDefaultKey(body)
            fail("cross_signing")
            for (field in listOf("master_key", "self_signing_key", "user_signing_key")) {
                body[field]?.let { keys[field] = it }
            }
        }

        override suspend fun uploadSignatures(signatures: JsonArray) {
            fail("signatures")
            for (upload in signatures.map { it.jsonObject }) {
                this.signatures +=
                    buildJsonObject {
                        put("signer_user_id", ALICE)
                        put("signer_key_id", upload["signer_key_id"]!!)
                        put("target_user_id", upload["target_user_id"]!!)
                        put("target_key_id", upload["target_key_id"]!!)
                        put("signature", upload["signature"]!!)
                    }
            }
        }

        fun addVersion(body: JsonObject) {
            versions +=
                buildJsonObject {
                    put("version", (versions.size + 1).toString())
                    put("algorithm", body["algorithm"]!!)
                    put("auth_data", body["auth_data"]!!)
                    put("deleted", false)
                }
        }

        override suspend fun createBackupVersion(body: JsonObject, reauth: String?) {
            checkDefaultKey(body)
            fail("versions")
            addVersion(body)
        }

        override suspend fun uploadBackupKeys(version: String, secretsRevision: Long, body: JsonObject) {
            backupConflict?.let { throw ApiException(409, code(it)) }
            check(version == versions.last()["version"]!!.jsonPrimitive.content)
            check(secretsRevision == revision)
            backupUploads += body["sessions"]!!.jsonArray.size
        }
    }

    private companion object {
        const val ALICE = CryptoStoreTest.ALICE
        const val BOB = CryptoStoreTest.BOB
        const val BOB_DEVICE = CryptoStoreTest.BOB_DEVICE
        const val CHANNEL = CryptoStoreTest.CHANNEL
    }
}
