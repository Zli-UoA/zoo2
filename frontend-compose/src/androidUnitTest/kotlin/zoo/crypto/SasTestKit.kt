package zoo.crypto

import java.io.File
import java.nio.file.Files
import javax.crypto.KeyGenerator
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import zoo.ApiException

/**
 * 他のユーザーの検証（#368）の単体テストに使う、状態を持つ模擬サーバーと端末（[EncryptedDmTest] と同じ考え方）。
 *
 * ユーザーごとの初回設定は [FakeSetupServer]、to-device は端末ごとの受信箱、`keys/claim` は相手の暗号ストアの
 * ワンタイム鍵、ピンと署名はユーザーごとに置く。時刻は [now] を進めて変える。
 */
internal class SasWorld : AutoCloseable {
    val root: File = Files.createTempDirectory("zoo-e2ee-sas").toFile()
    private val lifetime = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val opened = mutableListOf<CryptoStore>()
    private val controllers = mutableListOf<SasController>()

    @Volatile
    var now: Long = System.currentTimeMillis()

    val setups = mutableMapOf<String, FakeSetupServer>()
    val devices = mutableMapOf<String, MutableList<SasDevice>>()
    val inbox = mutableMapOf<String, MutableList<ToDeviceRow>>()

    /** ユーザーごとに、置いたピン（相手の `user_id` ごと）。 */
    val pins = mutableMapOf<String, MutableMap<String, JsonElement>>()

    class PinUpload(val caller: String, val pins: JsonArray, val replace: Boolean, val reauth: String?)
    class SignatureUpload(val caller: String, val signatures: JsonArray, val reauth: String?)

    val pinUploads = mutableListOf<PinUpload>()
    val signatureUploads = mutableListOf<SignatureUpload>()

    /** 置くと、送信ユーザーを渡して真を返した `sendToDevice` の宛先を、すべて届けずに `undelivered` で返す。 */
    @Volatile
    var refuseSend: ((caller: String) -> Boolean)? = null

    /** 置くと、送信ユーザーを渡して真を返した `sendToDevice` を、キューに入れたあとで応答をなくした（例外）ことにする。 */
    @Volatile
    var loseResponse: ((caller: String) -> Boolean)? = null

    /** 置くと、`sendToDevice` はこれが終わるまで待つ（待ち始めたら [sendEntered] を終える）。 */
    @Volatile
    var sendGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    val sendEntered = kotlinx.coroutines.CompletableDeferred<Unit>()

    /** 受け付けた `(送信端末, txn_id, 宛先ユーザー, 宛先端末)`。 */
    private val toDeviceTxns = mutableSetOf<List<String>>()
    var sendRequests = 0

    /** 失敗させる `keys/query` の残りの回数。 */
    var failKeysQuery = 0
    var keysQueries = 0

    override fun close() {
        controllers.forEach { it.dispose() }
        lifetime.cancel()
        opened.forEach { it.close() }
        root.deleteRecursively()
    }

    /** `caller` から見た `users` の `keys/query` の応答（呼び出したユーザーも入れる）。 */
    @Synchronized
    fun keysQuery(caller: String, users: List<String>): JsonObject {
        val all = (users + caller).distinct().sorted()
        return buildJsonObject {
            put("user_ids", JsonArray(all.map(::JsonPrimitive)))
            put("device_keys", buildJsonObject {
                for (user in all) put(user, buildJsonObject {
                    devices[user].orEmpty().forEach { put(it.store.deviceId, Json.parseToJsonElement(it.store.deviceKeys())) }
                })
            })
            for ((field, target) in listOf("master_key" to "master_keys", "self_signing_key" to "self_signing_keys")) {
                put(target, buildJsonObject { all.forEach { user -> setups[user]?.keys?.get(field)?.let { put(user, it) } } })
            }
            put("user_signing_keys", buildJsonObject { setups[caller]?.keys?.get("user_signing_key")?.let { put(caller, it) } })
            put("signatures", JsonArray(all.flatMap { setups[it]?.signatures.orEmpty() }))
            put("tofu_pins", JsonObject(pins[caller].orEmpty()))
        }
    }

    fun api(caller: String): SasApi = object : SasApi {
        override suspend fun queryKeys(userIds: List<String>): KeysQuery = synchronized(this@SasWorld) {
            keysQueries++
            if (failKeysQuery > 0) {
                failKeysQuery--
                throw java.io.IOException("keys/query に失敗しました")
            }
            val raw = keysQuery(caller, userIds)
            KeysQuery(raw.getValue("device_keys").jsonObject, raw.getValue("signatures") as JsonArray, raw)
        }

        override suspend fun claimKeys(oneTimeKeys: JsonObject): JsonObject = synchronized(this@SasWorld) {
            buildJsonObject {
                for ((user, wanted) in oneTimeKeys) put(user, buildJsonObject {
                    for (deviceId in wanted.jsonObject.keys) {
                        val device = devices[user].orEmpty().first { it.store.deviceId == deviceId }
                        val key = device.store.prepareKeyUpload(0, true)!!.oneTimeKeys.entries.first()
                        // 払い出した鍵は 2 回は使わない（次の払い出しでは新しい鍵を作る）。
                        device.store.markKeysPublished()
                        put(deviceId, JsonObject(mapOf(key.key to key.value)))
                    }
                })
            }
        }

        override suspend fun sendToDevice(deviceId: String, messages: JsonObject): List<Pair<String, String>> =
            sendToDevice(deviceId, java.util.UUID.randomUUID().toString(), messages)

        /** 実際のサーバーと同じく、同じ送信端末・`txn_id`・宛先の再送は 1 件にまとめる。 */
        override suspend fun sendToDevice(deviceId: String, txnId: String, messages: JsonObject): List<Pair<String, String>> {
            sendGate?.let { gate ->
                sendEntered.complete(Unit)
                gate.await()
            }
            val lost = synchronized(this@SasWorld) {
                sendRequests++
                if (refuseSend?.invoke(caller) == true) {
                    return messages.flatMap { (user, targets) -> targets.jsonObject.keys.map { user to it } }
                }
                for ((user, targets) in messages) {
                    for ((target, content) in targets.jsonObject) {
                        if (!toDeviceTxns.add(listOf(deviceId, txnId, user, target))) continue
                        val box = inbox.getOrPut("$user|$target") { mutableListOf() }
                        box += ToDeviceRow("$caller-${box.size}-${System.nanoTime()}", caller, "m.room.encrypted", content.jsonObject)
                    }
                }
                loseResponse?.invoke(caller) == true
            }
            if (lost) throw java.io.IOException("応答をなくしました")
            return emptyList()
        }

        override suspend fun uploadPins(pins: JsonArray, replace: Boolean, reauth: String?) = synchronized(this@SasWorld) {
            if (replace && reauth == null) throw ApiException(403, buildJsonObject { put("code", "E2EE_REAUTH_REQUIRED") })
            pinUploads += PinUpload(caller, pins, replace, reauth)
            for (pin in pins) {
                val target = pin.jsonObject.getValue("target_user_id").jsonPrimitive.content
                val placed = this@SasWorld.pins.getOrPut(caller) { mutableMapOf() }
                if (!replace && placed[target] != null && placed[target] != pin) {
                    throw ApiException(409, buildJsonObject { put("code", "E2EE_PIN_CONFLICT") })
                }
                placed[target] = pin
            }
        }

        override suspend fun uploadSignaturesWithReauth(signatures: JsonArray, reauth: String?) = synchronized(this@SasWorld) {
            if (reauth == null) throw ApiException(403, buildJsonObject { put("code", "E2EE_REAUTH_REQUIRED") })
            signatureUploads += SignatureUpload(caller, signatures, reauth)
        }
    }

    /**
     * `user` の端末。`setUp` が真なら、初回設定を済ませて解錠した端末にする。`expireInterval` は期限切れを確かめる
     * 間隔（0 なら確かめない。テストが [SasController.expire] を呼ぶ）。
     */
    fun device(user: String, name: String, setUp: Boolean = true, expireInterval: Duration = Duration.ZERO): SasDevice {
        val setup = synchronized(this) { setups.getOrPut(user) { FakeSetupServer(user) } }
        val device = SasDevice(user, name, expireInterval)
        if (setUp) runBlocking {
            val e2ee = E2eeSetup(setup, user, store = { device.store })
            val start = e2ee.begin()
            e2ee.completeSetup(start.session, "setup")
            start.session.close()
        }
        return device
    }

    inner class SasDevice(val user: String, name: String, expireInterval: Duration) {
        private val wrapping = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
        private val files = CryptoStoreFiles(File(root, name), { wrapping }) {}
        val store: CryptoStore = CryptoStore.open(files, user)!!.store.also {
            if (!it.isRegistered) it.markRegistered()
            opened += it
            synchronized(this@SasWorld) { devices.getOrPut(user) { mutableListOf() } += this }
        }
        val sas = SasController(api(user), MutableStateFlow(store), lifetime, now = { now }, expireInterval = expireInterval)
            .also { controllers += it }

        /**
         * 届いた to-device を処理し（Shape で受け取ったものと同じ。[E2eeDeviceManager] と同じく、送信ユーザーと自分の
         * `keys/query` を使う）、検証のメッセージを [SasController] に渡して、処理を終えるまで待つ。
         */
        fun receive(): ToDeviceResult = runBlocking {
            val rows = synchronized(this@SasWorld) { inbox.remove("$user|${store.deviceId}").orEmpty() }
            val keys = keysQuery(user, rows.map { it.senderUserId })
            val result = store.processToDevice(rows, keys.getValue("device_keys").jsonObject, keys.getValue("signatures") as JsonArray, keys)
            store.acknowledged(result.processed)
            if (result.verifications.isNotEmpty()) sas.handleMessages(result.verifications).join()
            result
        }

        /** [receive] と同じく処理するが、検証のメッセージの処理を待たずに、その [kotlinx.coroutines.Job] を返す。 */
        fun receiveInBackground(): kotlinx.coroutines.Job {
            val rows = synchronized(this@SasWorld) { inbox.remove("$user|${store.deviceId}").orEmpty() }
            val keys = keysQuery(user, rows.map { it.senderUserId })
            val result = store.processToDevice(rows, keys.getValue("device_keys").jsonObject, keys.getValue("signatures") as JsonArray, keys)
            store.acknowledged(result.processed)
            return sas.handleMessages(result.verifications)
        }

        /** この端末あての to-device が届くまで待つ（裏で送るメッセージを待つため）。 */
        fun awaitInbox() = runBlocking {
            withTimeout(30.seconds) {
                while (synchronized(this@SasWorld) { inbox["$user|${store.deviceId}"].isNullOrEmpty() }) {
                    kotlinx.coroutines.delay(10)
                }
            }
        }

        /** `txnId` の検証の表示（なければ失敗）。 */
        fun entry(txnId: String): SasEntry = sas.entries.value.single { it.view.transactionId == txnId }

        /** `txnId` の検証が `predicate` を満たすまで待つ（保存は裏で行うため）。 */
        fun await(txnId: String, predicate: (SasEntry) -> Boolean): SasEntry = runBlocking {
            withTimeout(30.seconds) {
                sas.entries.first { list -> list.any { it.view.transactionId == txnId && predicate(it) } }
                    .single { it.view.transactionId == txnId }
            }
        }
    }
}
