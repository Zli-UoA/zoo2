package zoo.crypto

import electric4k.Shape
import electric4k.ShapeClient
import electric4k.ShapeStream
import electric4k.ShapeStreamOptions
import io.ktor.client.HttpClient
import kotlin.time.Duration
import kotlin.time.Duration.Companion.minutes
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import zoo.ApiException
import zoo.Config
import zoo.Platform
import zoo.ZooApi
import zoo.endpoint
import zoo.json

/** 端末の登録と to-device に使う E2EE の API（docs/e2ee.md の「API」）。 */
interface E2eeApi {
    /** `POST /api/e2ee/devices`。応答の `one_time_key_counts` と `unused_fallback_key_types` を使う。 */
    suspend fun registerDevice(body: JsonObject): JsonObject

    /** `POST /api/e2ee/keys/upload`。鍵が空なら、今の数を読むだけになる。 */
    suspend fun uploadKeys(deviceId: String, oneTimeKeys: JsonObject, fallbackKeys: JsonObject): JsonObject

    /** `POST /api/e2ee/keys/query` の `device_keys`。 */
    suspend fun queryKeys(userIds: List<String>): JsonObject

    /** `POST /api/e2ee/to_device/ack`。 */
    suspend fun ackToDevice(deviceId: String, ids: List<String>)

    /** `DELETE /api/e2ee/devices/{device_id}`。 */
    suspend fun deleteDevice(deviceId: String)
}

/** E2EE の API が期限までに答えなかった。通信の失敗と同じく、間隔をあけて試し直す。 */
class E2eeRequestTimeoutException(cause: Throwable) : java.io.IOException("E2EE の API が時間内に答えませんでした", cause)

/**
 * 要求ごとの期限。`withTimeout` の [TimeoutCancellationException] は [CancellationException] なので、
 * そのままでは呼び出し側の取り消しと区別できない。通信の失敗（[E2eeRequestTimeoutException]）にして返す。
 */
suspend fun <T> withRequestTimeout(timeout: Duration, block: suspend () -> T): T =
    try {
        withTimeout(timeout) { block() }
    } catch (failure: TimeoutCancellationException) {
        throw E2eeRequestTimeoutException(failure)
    }

/** [ZooApi] で呼ぶ [E2eeApi]。E2EE の API には 30 秒の期限を付ける（TUI と同じ）。 */
class ZooE2eeApi(private val api: ZooApi) : E2eeApi {
    private suspend fun call(path: String, method: String, body: JsonElement? = null): JsonElement =
        withRequestTimeout(TIMEOUT) { api.request(path, method, body) }

    /** 書き込みの応答は `{"data": ...}` の封筒に入っている。 */
    private fun data(response: JsonElement): JsonObject =
        ((response as? JsonObject)?.get("data") ?: response) as? JsonObject ?: JsonObject(emptyMap())

    override suspend fun registerDevice(body: JsonObject) = data(call("/api/e2ee/devices", "POST", body))

    override suspend fun uploadKeys(deviceId: String, oneTimeKeys: JsonObject, fallbackKeys: JsonObject) =
        data(
            call(
                "/api/e2ee/keys/upload",
                "POST",
                buildJsonObject {
                    put("device_id", deviceId)
                    put("one_time_keys", oneTimeKeys)
                    put("fallback_keys", fallbackKeys)
                },
            )
        )

    override suspend fun queryKeys(userIds: List<String>): JsonObject {
        // サーバーの上限は 100 人。TUI と同じく 50 人ずつにする。
        val merged = mutableMapOf<String, JsonElement>()
        for (chunk in userIds.chunked(50)) {
            val response =
                call("/api/e2ee/keys/query", "POST", buildJsonObject { put("user_ids", JsonArray(chunk.map(::JsonPrimitive))) })
            ((response as? JsonObject)?.get("device_keys") as? JsonObject)?.let(merged::putAll)
        }
        return JsonObject(merged)
    }

    override suspend fun ackToDevice(deviceId: String, ids: List<String>) {
        for (chunk in ids.chunked(1000)) {
            call(
                "/api/e2ee/to_device/ack",
                "POST",
                buildJsonObject {
                    put("device_id", deviceId)
                    put("ids", JsonArray(chunk.map(::JsonPrimitive)))
                },
            )
        }
    }

    override suspend fun deleteDevice(deviceId: String) {
        call("/api/e2ee/devices/$deviceId", "DELETE")
    }

    private companion object {
        val TIMEOUT = 30.seconds
    }
}

/**
 * 自分の端末あての to-device メッセージの Shape（`/api/shapes/e2ee_to_device?device_id=`）を、
 * 残っている行の一覧（ID の昇順）の流れにする。サーバーは ack で行を消すので、Shape には未処理の
 * メッセージだけが残る。永続化はしない（処理済みの印は暗号ストアに置く）。
 */
fun toDeviceShape(config: Config, platform: Platform, http: HttpClient, deviceId: String): Flow<List<ToDeviceRow>> =
    callbackFlow {
        val client =
            ShapeClient(
                endpoint(config.apiBaseUrl, "${config.shapeProxyPath}/e2ee_to_device"),
                httpClient = http,
                headers = { platform.headers() },
                queryParams = { mapOf("device_id" to deviceId) },
            )
        val stream = ShapeStream(client, scope = this, options = ShapeStreamOptions(liveSse = true))
        val shape = Shape(stream)
        fun emit(rows: List<JsonObject>) {
            trySend(rows.map(::toDeviceRow).sortedWith(compareBy<ToDeviceRow> { it.id.length }.thenBy { it.id }))
        }
        val unsubscribeErrors = stream.subscribe({}, { failure -> close(failure) })
        val unsubscribe = shape.subscribe { change -> if (shape.isUpToDate) emit(change.rows) }
        launch {
            emit(shape.awaitRows())
            shape.error?.let { close(it) }
        }
        awaitClose {
            unsubscribe()
            unsubscribeErrors()
            shape.close()
            stream.close()
        }
    }.conflate()

/**
 * Shape の行を読む。Shape は bigint の列を文字列で配り、jsonb の列は JSON か文字列で届くので、
 * `content` は文字列なら JSON として読み直す（読めなければ `null` にして、復号できないメッセージとして捨てる）。
 */
fun toDeviceRow(row: JsonObject): ToDeviceRow {
    fun text(column: String) =
        when (val value = row[column]) {
            null, JsonNull -> ""
            is JsonPrimitive -> value.contentOrNull ?: value.toString()
            else -> value.toString()
        }
    val content =
        when (val value = row["content"]) {
            null -> JsonNull
            is JsonPrimitive ->
                if (value.isString) {
                    try {
                        json.parseToJsonElement(value.content)
                    } catch (_: Exception) {
                        JsonNull
                    }
                } else value
            else -> value
        }
    return ToDeviceRow(text("id"), text("sender_user_id"), text("type"), content)
}

/**
 * ログイン後の端末の登録と、鍵の補充、to-device の処理（#359。React の `deviceManager.ts`、TUI の
 * `e2ee/manager.rs` と同じ流れ）。
 *
 * 1. 暗号ストアを開く。端末がなければ作る。
 * 2. 登録していなければ `POST /api/e2ee/devices` で登録する（応答をなくしても同じ鍵で送り直す）。
 * 3. サーバーのワンタイム鍵が最大数に足りなければ作って送る。フォールバック鍵が使われたら作り直す。
 * 4. to-device の Shape を購読し、処理したメッセージを ack で消す。Olm セッションを新しく作った
 *    （ワンタイム鍵を使われた）ら、3 をもう一度行う。
 *
 * 失敗は表示せず、暗号ストアを閉じてから、間隔をのばしながら（5 秒から 5 分まで）初めからやり直す
 * （開き直すので、保存より先に進んだメモリーの状態を使い続けない）。登録の 409 `E2EE_DEVICE_EXISTS`、
 * 鍵のアップロードや ack の 404 は、端末が別の端末から削除された印なので、暗号ストアを作り直して
 * 新しい端末として登録する。
 */
class E2eeDeviceManager(
    private val api: E2eeApi,
    private val server: String,
    private val userId: String,
    private val displayName: String,
    private val files: CryptoStoreFiles,
    private val pending: PendingDeviceDeletions,
    private val toDevice: (deviceId: String) -> Flow<List<ToDeviceRow>>,
    scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val firstRetryDelay: Duration = 5.seconds,
    private val maxRetryDelay: Duration = 5.minutes,
    /** ログアウトで、登録の要求の結果と、端末の削除 API を待つ時間。 */
    private val settleTimeout: Duration = 5.seconds,
    /** 失敗の通知（ログに出す）。秘密は入らない。 */
    private val onError: (Throwable) -> Unit = {},
) {
    private val background = CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]))
    private val control = Mutex()
    private var loop: Job? = null

    /** 処理に使っている暗号ストア。処理のコルーチン（[loop]）だけが触り、止まるときに閉じる。 */
    @Volatile
    private var store: CryptoStore? = null

    /** ログアウトのために開いた暗号ストア。 */
    @Volatile
    private var logoutStore: CryptoStore? = null

    /** 最後に開いた端末。暗号ストアを開けなくても、ログアウトで消すのに使う。 */
    @Volatile
    var currentDeviceId: String? = null
        private set

    /**
     * 結果がまだ分からない登録の要求。処理を止めても中止しない（中止しても、サーバーが受け取って
     * いれば端末はできる）。ログアウトはこの結果を待ってから端末を消す。
     */
    @Volatile
    private var registration: Deferred<JsonObject>? = null

    /** 登録の要求が成功したと分かった端末（ログアウトで、404 を消えたものとしてよいか決める）。 */
    private val confirmedRegistrations: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

    /**
     * この manager がすでに 404 を数えた端末。[runOnce] は失敗のたびにやり直すので、ログインごとに
     * 1 回だけ数える（同じログインの試し直しで上限に達して、削除待ちを早く外さないため）。
     */
    private val countedNotFound: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

    /** 保存鍵を失って作り直した前の端末（ログアウトでも消す）。 */
    private val replacedDevices = mutableListOf<PendingDeviceDeletion>()

    /** 処理を始める。動いていれば何もしない。 */
    fun start() {
        if (loop?.isActive == true) return
        loop = background.launch { runLoop() }
    }

    /** 処理を止めて、暗号ストアを閉じる（画面を閉じたとき）。 */
    fun stop() {
        background.coroutineContext[Job]?.cancel()
        logoutStore?.close()
        logoutStore = null
    }

    private suspend fun runLoop() {
        var backoff = firstRetryDelay
        while (true) {
            var connected = false
            try {
                runOnce { connected = true }
            } catch (failure: CancellationException) {
                // この処理が取り消されたのなら止める。内側の期限切れなど、取り消されていないのに
                // 届いたものは、ほかの失敗と同じく間隔をあけて試し直す。
                currentCoroutineContext().ensureActive()
                onError(failure)
            } catch (failure: Exception) {
                onError(failure)
            } finally {
                store?.close()
                store = null
            }
            // 登録と補充を終えて Shape を受け取れていたら、そのあとの切断は連続した失敗ではないので、
            // 最初の間隔から数え直す。続けて失敗したら倍にする。
            val wait = if (connected) firstRetryDelay else backoff
            backoff = minOf(wait * 2, maxRetryDelay)
            delay(wait)
        }
    }

    private suspend fun open(): CryptoStore {
        // 開いたものは、取り消されても閉じられるよう、開いたその場で覚える。
        val opened =
            withContext(io) {
                CryptoStore.open(files, userId) { replaced, registered ->
                    // 記録できなければ開かない（前の端末の ID を失わないため）。
                    pending.add(entry(replaced, unconfirmed = !registered))
                }!!.also { store = it.store }
            }
        currentDeviceId = opened.store.deviceId
        opened.replacedDeviceId?.let { replaced ->
            deleteDeviceOrRemember(rememberedEntry(replaced) ?: entry(replaced, unconfirmed = true))
        }
        return opened.store
    }

    private suspend fun runOnce(connected: () -> Unit) {
        retryPendingDeletions()
        var store = open()
        val counts =
            try {
                ensureRegistered(store)
            } catch (failure: ApiException) {
                if (!isDeviceGone(failure)) throw failure
                // 別の端末から消された。暗号ストアを作り直して、新しい端末として登録する。
                withContext(io) { store.destroy() }
                this.store = null
                store = open()
                ensureRegistered(store)
            }
        replenish(store, counts)
        val deviceId = store.deviceId
        // ack を送れた行は、Shape から消えるまで処理し直さない（ack より先に次の一覧が届くため）。
        val acknowledged = mutableSetOf<String>()
        toDevice(deviceId).collect { rows ->
            connected()
            acknowledged.retainAll(rows.mapTo(mutableSetOf()) { it.id })
            val waiting = rows.filter { it.id !in acknowledged }
            if (waiting.isNotEmpty()) acknowledged += process(store, deviceId, waiting)
        }
        error("to-device の Shape が終わりました")
    }

    private suspend fun ensureRegistered(store: CryptoStore): KeyCounts {
        val deviceId = store.deviceId
        // 補充に要る今の数を、空のアップロードで読む。
        if (store.isRegistered) return KeyCounts(api.uploadKeys(deviceId, EMPTY, EMPTY))
        val body = JsonObject(withContext(io) { store.registrationRequest() } + ("display_name" to JsonPrimitive(displayName)))
        val request = background.async { api.registerDevice(body) }
        request.invokeOnCompletion { cause -> if (cause == null) confirmedRegistrations += deviceId }
        registration = request
        val response = request.await()
        withContext(io) { store.markRegistered() }
        return KeyCounts(response)
    }

    private suspend fun replenish(store: CryptoStore, counts: KeyCounts) {
        val upload =
            withContext(io) { store.prepareKeyUpload(counts.oneTimeKeys, counts.hasUnusedFallbackKey) } ?: return
        api.uploadKeys(store.deviceId, upload.oneTimeKeys, upload.fallbackKeys)
        withContext(io) { store.markKeysPublished() }
    }

    /** 行を処理して ack を送り、ack を送れた ID を返す。 */
    private suspend fun process(store: CryptoStore, deviceId: String, rows: List<ToDeviceRow>): List<String> {
        val senders = rows.map { it.senderUserId }.distinct().sorted()
        val deviceKeys = api.queryKeys(senders)
        val result = withContext(io) { store.processToDevice(rows, deviceKeys) }
        if (result.processed.isNotEmpty()) {
            api.ackToDevice(deviceId, result.processed)
            withContext(io) { store.acknowledged(result.processed) }
        }
        if (result.newOlmSessions > 0) replenish(store, KeyCounts(api.uploadKeys(deviceId, EMPTY, EMPTY)))
        return result.processed
    }

    /**
     * ログアウトの確認の前に呼ぶ。to-device の処理を止め（処理中のものは終わらせる）、暗号ストアを
     * 開いて、鍵バックアップに送れていない鍵の数を返す。数えられなければ -1（「確かめられなかった」と
     * 示して確かめる）。暗号ストアがなければ 0。
     */
    suspend fun prepareLogout(): Int =
        control.withLock {
            loop?.cancelAndJoin()
            loop = null
            if (logoutStore == null) {
                try {
                    val opened =
                        withContext(io) {
                            CryptoStore.open(files, userId, create = false) { replaced, registered ->
                                val replacedEntry = entry(replaced, unconfirmed = !registered)
                                pending.add(replacedEntry)
                                synchronized(replacedDevices) { replacedDevices += replacedEntry }
                            }?.also { logoutStore = it.store }
                        } ?: return@withLock 0
                    currentDeviceId = opened.store.deviceId
                } catch (failure: CancellationException) {
                    throw failure
                } catch (failure: Exception) {
                    onError(failure)
                    return@withLock -1
                }
            }
            logoutStore?.let { withContext(io) { it.pendingBackupCount() } } ?: -1
        }

    /** ログアウトを取りやめたら、to-device の処理に戻る。 */
    suspend fun cancelLogout() {
        control.withLock {
            logoutStore?.close()
            logoutStore = null
            start()
        }
    }

    /**
     * ログアウト：暗号ストアを消してから、端末の削除 API を呼ぶ（docs/e2ee.md の「ログアウトと
     * 端末の削除」）。API が失敗したら削除待ちに残し、同じユーザーの次のログインで消し直す。
     *
     * 登録の要求は、結果を [settleTimeout] まで待ってから端末を消す（先に消すと、あとから届いた登録で
     * 端末ができてしまうため）。それでも分からなければ、いまは消さずに削除待ちに残す。
     */
    suspend fun destroyForLogout() {
        control.withLock {
            loop?.cancelAndJoin()
            loop = null
            val request = registration
            val registrationUnknown =
                request != null && withTimeoutOrNull(settleTimeout) { request.join() } == null && request.isActive
            val store = logoutStore
            try {
                val deviceId =
                    store?.deviceId ?: currentDeviceId ?: withContext(io) { CryptoStore.storedDeviceId(files, userId) }
                // 登録できたと分かっていなければ、404 でもすぐには消えたものとしない。
                val target =
                    deviceId?.let {
                        entry(it, unconfirmed = store?.isRegistered != true && it !in confirmedRegistrations)
                    }
                // 記録に失敗しても、暗号ストアは消す（API の結果を待たずに、先に消す）。
                target?.let { remember(it) }
                try {
                    withContext(io) { if (store != null) store.destroy() else CryptoStore.destroy(files) }
                } catch (failure: CancellationException) {
                    throw failure
                } catch (failure: Exception) {
                    // 消せなかったファイルは、次に同じユーザーでログインしたときに開かれ、削除待ちの端末を
                    // 消したあとで、新しい端末として登録し直す。
                    onError(failure)
                }
                if (target != null && !registrationUnknown) deleteDeviceOrRemember(target)
                synchronized(replacedDevices) { replacedDevices.toList() }.forEach { deleteDeviceOrRemember(it) }
            } finally {
                // 途中で失敗しても閉じる（同じプロセスで開き直せるように）。消したあとなら何もしない。
                store?.close()
                logoutStore = null
                background.coroutineContext[Job]?.cancel()
            }
        }
    }

    /** 前のログアウトで消せなかった端末を、同じ接続先とユーザーで消し直す。 */
    private suspend fun retryPendingDeletions() {
        val all =
            try {
                withContext(io) { pending.all() }
            } catch (failure: CancellationException) {
                throw failure
            } catch (failure: Exception) {
                // 一覧を読めなければ、何も消さない（ファイルはそのまま残す）。
                onError(failure)
                return
            }
        all.filter { it.server == server && it.userId == userId }.forEach { deleteDeviceOrRemember(it) }
    }

    private enum class DeleteOutcome { DELETED, NOT_FOUND, FAILED }

    /**
     * 端末を消す。失敗したら削除待ちに残す。404 は、登録できたと分かっている端末なら、消すものが
     * ないので済んだものとする。登録できたか分からない端末（[PendingDeviceDeletion.unconfirmed]）は、
     * あとから届いた登録で端末ができうるので残し、続けて [MAX_NOT_FOUND] 回 404 なら済んだものとする。
     * 404 以外の失敗では、回数を 0 に戻す（React の `deleteOrKeep`、TUI の `delete_device_or_remember` と同じ）。
     */
    private suspend fun deleteDeviceOrRemember(target: PendingDeviceDeletion) {
        remember(target)
        val outcome =
            withTimeoutOrNull(settleTimeout) {
                try {
                    api.deleteDevice(target.deviceId)
                    DeleteOutcome.DELETED
                } catch (failure: ApiException) {
                    if (failure.status == 404) DeleteOutcome.NOT_FOUND else DeleteOutcome.FAILED
                } catch (failure: CancellationException) {
                    throw failure
                } catch (_: Exception) {
                    DeleteOutcome.FAILED
                }
            } ?: DeleteOutcome.FAILED
        val current = rememberedEntry(target.deviceId) ?: target
        when {
            outcome == DeleteOutcome.DELETED -> forget(current)
            outcome == DeleteOutcome.NOT_FOUND && !current.unconfirmed -> forget(current)
            outcome == DeleteOutcome.NOT_FOUND ->
                if (countedNotFound.add(current.deviceId)) {
                    if (current.notFound + 1 >= MAX_NOT_FOUND) forget(current)
                    else pendingWrite { pending.update(current.copy(notFound = current.notFound + 1)) }
                }
            // 404 が続いていない。数え直す（間に失敗をはさんだ 404 は、続けて受けたものとしない）。
            current.notFound != 0 -> pendingWrite { pending.update(current.copy(notFound = 0)) }
            else -> Unit
        }
    }

    /** 削除待ちに足す。書けなくても続ける（通知だけする）。 */
    private suspend fun remember(target: PendingDeviceDeletion) = pendingWrite { pending.add(target) }

    private suspend fun forget(target: PendingDeviceDeletion) = pendingWrite { pending.remove(target) }

    private suspend fun rememberedEntry(deviceId: String): PendingDeviceDeletion? =
        try {
            withContext(io) { pending.all() }.find { it.sameDevice(entry(deviceId)) }
        } catch (failure: CancellationException) {
            throw failure
        } catch (_: Exception) {
            null
        }

    private suspend fun pendingWrite(write: () -> Unit) {
        try {
            withContext(io) { write() }
        } catch (failure: CancellationException) {
            throw failure
        } catch (failure: Exception) {
            onError(failure)
        }
    }

    private fun entry(deviceId: String, unconfirmed: Boolean = false) =
        PendingDeviceDeletion(server, userId, deviceId, unconfirmed)

    private class KeyCounts(response: JsonObject) {
        val oneTimeKeys: Int =
            ((response["one_time_key_counts"] as? JsonObject)?.get("signed_curve25519")?.jsonPrimitive?.longOrNull ?: 0L)
                .toInt()
        val hasUnusedFallbackKey: Boolean =
            (response["unused_fallback_key_types"] as? JsonArray)?.any {
                (it as? JsonPrimitive)?.contentOrNull == "signed_curve25519"
            } == true
    }

    companion object {
        private val EMPTY = JsonObject(emptyMap())

        /** 登録できたか分からない端末を、404 が続いたら消えたものとする回数（次のログインごとに 1 回）。 */
        const val MAX_NOT_FOUND = 3

        /** 登録の 409 `E2EE_DEVICE_EXISTS`（削除済みの device_id）と、鍵のアップロードや ack の 404。 */
        fun isDeviceGone(failure: ApiException): Boolean =
            failure.status == 404 ||
                (failure.status == 409 &&
                    (failure.body?.get("code") as? JsonPrimitive)?.contentOrNull == "E2EE_DEVICE_EXISTS")
    }
}
