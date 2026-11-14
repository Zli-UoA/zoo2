package zoo.crypto

import java.io.File
import java.io.IOException
import java.nio.file.Files
import javax.crypto.KeyGenerator
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import zoo.ApiException
import zoo.crypto.CryptoStoreTest.Companion.ALICE
import zoo.crypto.CryptoStoreTest.Companion.ALICE_DEVICE
import zoo.crypto.CryptoStoreTest.Companion.BOB
import zoo.crypto.CryptoStoreTest.Companion.CHANNEL
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.OlmSessionStore
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.verifyDeviceKeys
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

/** ログイン後の端末の登録と、ログアウトでの削除（#359。React の `deviceManager.test.ts` と同じ場面）。 */
@OptIn(ExperimentalCoroutinesApi::class)
class E2eeDeviceManagerTest {
    private val root: File = Files.createTempDirectory("zoo-e2ee-manager").toFile()
    private val wrapping = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private val files = CryptoStoreFiles(File(root, "store"), { wrapping }) {}
    private val pending = PendingDeviceDeletions(File(root, "pending.json"))
    private val api = FakeApi()
    private val rows = MutableStateFlow<List<ToDeviceRow>>(emptyList())
    private val subscribed = mutableListOf<String>()

    @AfterTest
    fun cleanUp() {
        root.deleteRecursively()
    }

    /**
     * マネージャーは自分で終わらないので、テストの終わりに止める。`backgroundScope` は
     * `advanceUntilIdle` で進まないので使わない。
     */
    private val managerJob = Job()

    private fun managerTest(body: suspend TestScope.() -> Unit) = runTest {
        try {
            body()
        } finally {
            managerJob.cancel()
        }
    }

    private fun TestScope.manager(pending: PendingDeviceDeletions = this@E2eeDeviceManagerTest.pending) =
        E2eeDeviceManager(
            api = api,
            server = SERVER,
            userId = BOB,
            displayName = "Zoo Android（test）",
            files = files,
            pending = pending,
            toDevice = { deviceId ->
                subscribed += deviceId
                rows
            },
            scope = CoroutineScope(StandardTestDispatcher(testScheduler) + Job(managerJob)),
            io = StandardTestDispatcher(testScheduler),
            onError = { it.printStackTrace() },
        )

    @Test
    fun registersOnLoginAndDeletesOnLogout() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val body = api.registered.single()
        val deviceId = body["device_keys"]!!.jsonObject["device_id"]!!.jsonPrimitive.content
        assertEquals("Zoo Android（test）", body["display_name"]!!.jsonPrimitive.content)
        assertEquals(deviceId, manager.currentDeviceId)
        assertEquals(listOf(deviceId), subscribed)
        // 登録の応答で最大数があるので、補充しない。
        assertTrue(api.uploads.isEmpty())

        // Alice から鍵が届く。処理して ack を送り、使われたワンタイム鍵を補充する。
        rows.value = listOf(roomKeyFromAlice(body, "10"))
        advanceUntilIdle()
        assertEquals(listOf(listOf("10")), api.acks)
        assertEquals(listOf(0, 1), api.uploads.map { it.second })

        // ログアウト：送れていない鍵の数を見せてから、暗号ストアを消して端末を消す。
        assertEquals(1, manager.prepareLogout())
        manager.destroyForLogout()
        assertEquals(listOf(deviceId), api.deleted)
        assertFalse(files.directory.exists())
        assertTrue(pending.all().isEmpty())
    }

    @Test
    fun cancellingLogoutResumes() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        assertEquals(0, manager.prepareLogout())
        manager.cancelLogout()
        advanceUntilIdle()
        // 同じ端末で処理に戻る（登録し直さない）。
        assertEquals(1, api.registered.size)
        assertEquals(2, subscribed.size)
        assertEquals(subscribed[0], subscribed[1])
        assertTrue(files.directory.exists())
    }

    @Test
    fun reregistersWhenDeviceWasDeletedElsewhere() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val first = manager.currentDeviceId!!
        // 別の端末から消された：ack と鍵のアップロードが 404 になる。
        api.gone += first
        rows.value = listOf(ToDeviceRow("11", ALICE, "m.room.encrypted", JsonObject(emptyMap())))
        advanceUntilIdle()
        advanceTimeBy(6.seconds)
        runCurrent()
        advanceUntilIdle()
        val second = manager.currentDeviceId!!
        assertNotEquals(first, second)
        assertEquals(2, api.registered.size)
        assertEquals(listOf("11"), api.acks.last())
    }

    @Test
    fun retriesFailedDeletionOnNextLogin() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val first = manager.currentDeviceId!!
        api.deleteFailure = IOException("offline")
        manager.prepareLogout()
        manager.destroyForLogout()
        assertEquals(listOf(PendingDeviceDeletion(SERVER, BOB, first)), pending.all())

        api.deleteFailure = null
        manager().start()
        advanceUntilIdle()
        assertEquals(listOf(first), api.deleted)
        assertTrue(pending.all().isEmpty())
        assertEquals(2, api.registered.size)
    }

    @Test
    fun keepsDeviceWhenRegistrationOutcomeIsUnknown() = managerTest {
        api.registerGate = CompletableDeferred()
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val deviceId = manager.currentDeviceId!!
        assertEquals(0, manager.prepareLogout())
        manager.destroyForLogout()
        // 登録の結果が分からないうちに消すと、あとから届いた登録で端末が残りうる。削除待ちに残す。
        assertTrue(api.deleted.isEmpty())
        assertEquals(listOf(PendingDeviceDeletion(SERVER, BOB, deviceId, unconfirmed = true)), pending.all())
        assertFalse(files.directory.exists())
    }

    @Test
    fun notFoundForUnconfirmedDeviceIsRetriedUpToLimit() = managerTest {
        api.registerGate = CompletableDeferred()
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val deviceId = manager.currentDeviceId!!
        manager.prepareLogout()
        manager.destroyForLogout()
        api.registerGate = null
        api.missing += deviceId

        // 登録できたか分からない端末の 404 は、すぐには消えたものとしない。次のログインごとに数える。
        for (attempt in 1 until E2eeDeviceManager.MAX_NOT_FOUND) {
            val next = manager()
            next.start()
            advanceUntilIdle()
            assertEquals(
                listOf(PendingDeviceDeletion(SERVER, BOB, deviceId, unconfirmed = true, notFound = attempt)),
                pending.all(),
            )
            next.stop()
            advanceUntilIdle()
        }
        manager().start()
        advanceUntilIdle()
        assertTrue(pending.all().isEmpty())
    }

    @Test
    fun failureOtherThanNotFoundResetsCount() = managerTest {
        val stale = "00000000-0000-4000-8000-0000000000da"
        pending.add(PendingDeviceDeletion(SERVER, BOB, stale, unconfirmed = true))
        api.missing += stale
        // 404 → 404 → 503 → 404：失敗をはさんだので、続けて 3 回にはならず、削除待ちに残る。
        val failures = listOf(null, null, ApiException(503), null)
        for ((failure, expected) in failures.zip(listOf(1, 2, 0, 1))) {
            api.deleteFailure = failure
            val next = manager()
            next.start()
            advanceUntilIdle()
            assertEquals(
                listOf(PendingDeviceDeletion(SERVER, BOB, stale, unconfirmed = true, notFound = expected)),
                pending.all(),
            )
            next.stop()
            advanceUntilIdle()
        }
    }

    @Test
    fun notFoundIsCountedOncePerLogin() = managerTest {
        val stale = "00000000-0000-4000-8000-0000000000d9"
        pending.add(PendingDeviceDeletion(SERVER, BOB, stale, unconfirmed = true))
        api.missing += stale
        // 同じログインの中で、登録が 2 回失敗して初めからやり直す（そのたびに削除待ちを消し直す）。
        api.registerTimeouts = 2
        manager().start()
        advanceUntilIdle()
        assertEquals(3, api.registerAttempts)
        assertEquals(listOf(PendingDeviceDeletion(SERVER, BOB, stale, unconfirmed = true, notFound = 1)), pending.all())
    }

    @Test
    fun requestTimeoutBacksOffAndRetries() = managerTest {
        api.registerTimeouts = 1
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        // 期限切れは取り消しではないので、処理は止まらずに試し直して登録する。
        assertEquals(2, api.registerAttempts)
        assertEquals(1, api.registered.size)

        // 止めたら、取り消しは伝わり、届いた行を処理しない。
        manager.stop()
        advanceUntilIdle()
        rows.value = listOf(ToDeviceRow("20", ALICE, "m.room.encrypted", JsonObject(emptyMap())))
        advanceUntilIdle()
        assertTrue(api.acks.isEmpty())
    }

    @Test
    fun logoutClosesStoreWhenPendingListCannotBeWritten() = managerTest {
        // 削除待ちの一覧を書けない（親がファイル）。
        val blocker = File(root, "blocker").apply { writeText("x") }
        val broken = PendingDeviceDeletions(File(blocker, "pending.json"))
        val manager = manager(broken)
        manager.start()
        advanceUntilIdle()
        val deviceId = manager.currentDeviceId!!
        assertEquals(0, manager.prepareLogout())
        manager.destroyForLogout()
        // 記録できなくても、暗号ストアは消して閉じ、端末の削除も試す。
        assertFalse(files.directory.exists())
        assertEquals(listOf(deviceId), api.deleted)
        // 同じプロセスで、また開ける。
        CryptoStore.open(files, BOB)!!.store.close()
    }

    /** 登録の本文の鍵で、Alice から Bob に Megolm のセッション鍵を送る to-device の行。 */
    private fun roomKeyFromAlice(body: JsonObject, id: String): ToDeviceRow {
        val deviceKeys = body["device_keys"]!!.jsonObject
        val bobKeys = verifyDeviceKeys(deviceKeys.toString(), BOB, deviceKeys["device_id"]!!.jsonPrimitive.content)
        val oneTimeKey = body["one_time_keys"]!!.jsonObject.values.first()
        val alice = DeviceAccount(ALICE, ALICE_DEVICE)
        val sessions = OlmSessionStore()
        val sessionId = alice.createOutboundOlmSession(sessions, bobKeys, oneTimeKey.toString())
        val content = alice.encryptRoomKey(sessions, sessionId, bobKeys, OutboundSession(CHANNEL, 1uL, 0uL))
        api.deviceKeys = buildJsonObject {
            put(ALICE, buildJsonObject { put(ALICE_DEVICE, Json.parseToJsonElement(alice.deviceKeys())) })
        }
        return ToDeviceRow(id, ALICE, "m.room.encrypted", Json.parseToJsonElement(content))
    }

    private class FakeApi : E2eeApi {
        val registered = mutableListOf<JsonObject>()
        /** 端末と、送ったワンタイム鍵の数。 */
        val uploads = mutableListOf<Pair<String, Int>>()
        val acks = mutableListOf<List<String>>()
        val deleted = mutableListOf<String>()
        val gone = mutableSetOf<String>()
        var deleteFailure: Exception? = null
        var registerGate: CompletableDeferred<Unit>? = null
        /** 404 を返す端末（サーバーにない）。 */
        val missing = mutableSetOf<String>()
        /** 登録の要求を、この回数だけ期限切れにする。 */
        var registerTimeouts = 0
        var registerAttempts = 0
        var deviceKeys = JsonObject(emptyMap())
        private var serverKeys = 0

        private fun counts() = buildJsonObject {
            put("one_time_key_counts", buildJsonObject { put("signed_curve25519", serverKeys) })
            put("unused_fallback_key_types", JsonArray(listOf(JsonPrimitive("signed_curve25519"))))
        }

        override suspend fun registerDevice(body: JsonObject): JsonObject {
            registerAttempts++
            if (registerTimeouts > 0) {
                registerTimeouts--
                withRequestTimeout(1.seconds) { awaitCancellation() }
            }
            registerGate?.await()
            registered += body
            serverKeys = body["one_time_keys"]!!.jsonObject.size
            return counts()
        }

        override suspend fun uploadKeys(deviceId: String, oneTimeKeys: JsonObject, fallbackKeys: JsonObject): JsonObject {
            if (deviceId in gone) throw ApiException(404)
            uploads += deviceId to oneTimeKeys.size
            serverKeys += oneTimeKeys.size
            return counts()
        }

        override suspend fun queryKeys(userIds: List<String>) = deviceKeys

        override suspend fun ackToDevice(deviceId: String, ids: List<String>) {
            if (deviceId in gone) throw ApiException(404)
            acks += ids
            // 使われたワンタイム鍵の分だけ、サーバーの数が減る。
            serverKeys -= 1
        }

        override suspend fun deleteDevice(deviceId: String) {
            deleteFailure?.let { throw it }
            if (deviceId in missing) throw ApiException(404)
            deleted += deviceId
        }
    }

    private companion object {
        const val SERVER = "https://zoo.example"
    }
}
