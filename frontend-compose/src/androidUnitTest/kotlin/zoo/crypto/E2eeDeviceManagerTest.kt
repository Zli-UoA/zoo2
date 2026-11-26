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

    private fun TestScope.manager(
        pending: PendingDeviceDeletions = this@E2eeDeviceManagerTest.pending,
        setupApi: E2eeSetupApi? = null,
    ) =
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
            setupApi = setupApi,
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
    fun keyBackupRunsBesideToDeviceAndRetriesAfterFailures() = managerTest {
        val server = FakeSetupServer(BOB)
        val manager = manager(setupApi = server)
        manager.start()
        advanceUntilIdle()
        val body = api.registered.single()
        // この端末で初回設定を終える（解錠済みになる）。
        val setup = manager.setup!!
        val start = setup.begin()
        setup.completeSetup(start.session, "id-token")
        start.session.close()
        advanceUntilIdle()

        // 鍵バックアップへの送信が長引いても、to-device の処理と ack は止まらない。
        val hold = CompletableDeferred<Unit>()
        server.holdBackup = hold
        rows.value = listOf(roomKeyFromAlice(body, "10"))
        advanceUntilIdle()
        assertEquals(listOf(listOf("10")), api.acks)
        rows.value = listOf(roomKeyFromAlice(body, "11", keyIndex = 1))
        advanceUntilIdle()
        assertEquals(listOf(listOf("10"), listOf("11")), api.acks)
        assertTrue(server.backupUploads.isEmpty())

        // 通信の失敗は、間隔をあけて試し直す（新しい鍵が届かなくても）。
        server.failNext += "backup_keys"
        server.holdBackup = null
        hold.complete(Unit)
        runCurrent()
        assertTrue(server.backupUploads.isEmpty())
        advanceTimeBy(6.seconds)
        runCurrent()
        assertEquals(listOf(2), server.backupUploads)
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
    fun replacesOnlyDeviceWhenDeletedElsewhere() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val first = manager.currentDeviceId!!
        // 鍵バックアップに送っていない受信用セッションを 1 つ持つ。
        rows.value = listOf(roomKeyFromAlice(api.registered.single(), "10"))
        advanceUntilIdle()
        // 別の端末から消された：ack と鍵のアップロードが 404 `E2EE_DEVICE_NOT_FOUND` になり、一覧にもない。
        api.gone += first
        rows.value = listOf(ToDeviceRow("11", ALICE, "m.room.encrypted", JsonObject(emptyMap())))
        advanceUntilIdle()
        val second = manager.currentDeviceId!!
        assertNotEquals(first, second)
        assertEquals(1, api.listCalls)
        assertEquals(2, api.registered.size)
        assertEquals(second, api.registeredIds.last())
        assertEquals(listOf(first, second), subscribed.distinct())
        assertEquals(listOf("11"), api.acks.last())
        // 暗号ストアは消さず、古い端末の削除 API も呼ばない。受信用セッションは残る。
        assertTrue(api.deleted.isEmpty())
        assertTrue(pending.all().isEmpty())
        assertEquals(1, manager.prepareLogout())
    }

    @Test
    fun replacesDeviceWhenRegistrationSaysDeletedDeviceId() = managerTest {
        // 登録の 409 `E2EE_DEVICE_EXISTS`（削除済みの device_id）。一覧にもない。
        api.registerFailures += conflict("E2EE_DEVICE_EXISTS")
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        assertEquals(1, api.listCalls)
        assertEquals(2, api.attemptedIds.size)
        assertNotEquals(api.attemptedIds[0], api.attemptedIds[1])
        assertEquals(listOf(manager.currentDeviceId!!), api.registeredIds)
    }

    @Test
    fun notFoundOrConflictWithoutCodeIsOrdinaryFailure() = managerTest {
        // `code` のない 409：作り直さず、同じ端末で登録を試し直す。
        api.registerFailures += ApiException(409)
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val first = manager.currentDeviceId!!
        assertEquals(listOf(first, first), api.attemptedIds)
        assertEquals(listOf(first), api.registeredIds)

        // `code` のない 404：作り直さず、一覧も見ない。
        api.goneCode = null
        api.gone += first
        rows.value = listOf(ToDeviceRow("11", ALICE, "m.room.encrypted", JsonObject(emptyMap())))
        runCurrent()
        advanceTimeBy(6.seconds)
        runCurrent()
        assertEquals(first, manager.currentDeviceId)
        assertEquals(1, api.registered.size)
        assertEquals(0, api.listCalls)
        assertTrue(files.directory.exists())
        assertEquals(first, CryptoStore.storedDeviceId(files, BOB))
    }

    @Test
    fun keepsDeviceWhenListStillHasIt() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val first = manager.currentDeviceId!!
        // 404 `E2EE_DEVICE_NOT_FOUND` でも、一覧に今の端末があれば消されていない（ふつうの失敗）。
        api.gone += first
        api.listedGone = true
        rows.value = listOf(ToDeviceRow("11", ALICE, "m.room.encrypted", JsonObject(emptyMap())))
        runCurrent()
        advanceTimeBy(6.seconds)
        runCurrent()
        assertEquals(first, manager.currentDeviceId)
        assertEquals(1, api.registered.size)
        assertTrue(api.listCalls >= 2)
        assertEquals(first, CryptoStore.storedDeviceId(files, BOB))
    }

    @Test
    fun keepsDeviceWhenListFailsAndChecksAgainLater() = managerTest {
        val manager = manager()
        manager.start()
        advanceUntilIdle()
        val first = manager.currentDeviceId!!
        // 一覧を取れなければ、作り直さずに試し直す。
        api.gone += first
        api.listFailure = IOException("offline")
        rows.value = listOf(ToDeviceRow("11", ALICE, "m.room.encrypted", JsonObject(emptyMap())))
        runCurrent()
        advanceTimeBy(6.seconds)
        runCurrent()
        assertEquals(first, manager.currentDeviceId)
        assertEquals(1, api.registered.size)
        assertTrue(api.listCalls >= 2)

        // 一覧を取れるようになったら、次の試し直しで確かめて作り直す。
        api.listFailure = null
        advanceTimeBy(11.seconds)
        runCurrent()
        assertNotEquals(first, manager.currentDeviceId)
        assertEquals(2, api.registered.size)
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
    private fun roomKeyFromAlice(body: JsonObject, id: String, keyIndex: Int = 0): ToDeviceRow {
        val deviceKeys = body["device_keys"]!!.jsonObject
        val bobKeys = verifyDeviceKeys(deviceKeys.toString(), BOB, deviceKeys["device_id"]!!.jsonPrimitive.content)
        val oneTimeKey = body["one_time_keys"]!!.jsonObject.values.elementAt(keyIndex)
        val alice = DeviceAccount(ALICE, ALICE_DEVICE)
        val sessions = OlmSessionStore()
        val sessionId = alice.createOutboundOlmSession(sessions, bobKeys, oneTimeKey.toString())
        val content = alice.encryptRoomKey(sessions, sessionId, bobKeys, OutboundSession(CHANNEL, 1uL, 0uL))
        api.deviceKeys = buildJsonObject {
            put(ALICE, buildJsonObject { put(ALICE_DEVICE, Json.parseToJsonElement(alice.deviceKeys())) })
        }
        return ToDeviceRow(id, ALICE, "m.room.encrypted", Json.parseToJsonElement(content))
    }

    private fun conflict(code: String) = ApiException(409, buildJsonObject { put("code", code) })

    private class FakeApi : E2eeApi {
        val registered = mutableListOf<JsonObject>()
        /** 登録した端末。 */
        val registeredIds get() = registered.map { bodyDeviceId(it) }
        /** 登録を試した端末（失敗したものも）。 */
        val attemptedIds = mutableListOf<String>()
        /** 登録の要求を、順にこの失敗にする。 */
        val registerFailures = ArrayDeque<Exception>()
        /** [gone] の端末の 404 に付ける `code`。 */
        var goneCode: String? = "E2EE_DEVICE_NOT_FOUND"
        /** 真なら、[gone] の端末も `GET /api/e2ee/devices` の一覧に入れる。 */
        var listedGone = false
        var listFailure: Exception? = null
        var listCalls = 0
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
            attemptedIds += bodyDeviceId(body)
            registerFailures.removeFirstOrNull()?.let { throw it }
            if (registerTimeouts > 0) {
                registerTimeouts--
                withRequestTimeout(1.seconds) { awaitCancellation() }
            }
            registerGate?.await()
            registered += body
            serverKeys = body["one_time_keys"]!!.jsonObject.size
            return counts()
        }

        private fun notFound() = ApiException(404, goneCode?.let { code -> buildJsonObject { put("code", code) } })

        override suspend fun uploadKeys(deviceId: String, oneTimeKeys: JsonObject, fallbackKeys: JsonObject): JsonObject {
            if (deviceId in gone) throw notFound()
            uploads += deviceId to oneTimeKeys.size
            serverKeys += oneTimeKeys.size
            return counts()
        }

        override suspend fun queryKeys(userIds: List<String>) = KeysQuery(deviceKeys)

        override suspend fun ackToDevice(deviceId: String, ids: List<String>) {
            if (deviceId in gone) throw notFound()
            acks += ids
            // 使われたワンタイム鍵の分だけ、サーバーの数が減る。
            serverKeys -= 1
        }

        override suspend fun deleteDevice(deviceId: String) {
            deleteFailure?.let { throw it }
            if (deviceId in missing) throw ApiException(404)
            deleted += deviceId
        }

        override suspend fun listDeviceIds(): List<String> {
            listCalls++
            listFailure?.let { throw it }
            return registeredIds.distinct().filter { listedGone || it !in gone }
        }

        companion object {
            fun bodyDeviceId(body: JsonObject) = body["device_keys"]!!.jsonObject["device_id"]!!.jsonPrimitive.content
        }
    }

    private companion object {
        const val SERVER = "https://zoo.example"
    }
}
