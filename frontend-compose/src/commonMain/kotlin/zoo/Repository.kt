package zoo

import electric4k.*
import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import io.ktor.client.plugins.timeout
import io.ktor.utils.io.readUTF8Line
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*

class ApiException(val status: Int, val body: JsonObject? = null) : Exception("HTTP $status")

private const val THREAD_JOIN_REQUIRED_CODE = "THREAD_JOIN_REQUIRED"
private const val THREAD_JOIN_REQUIRED_MESSAGE = "Join the thread before replying"

/**
 * 参加行があるスレッドへ、未参加・退出済みのまま返信したときの 403（backend の `create_message_impl`）。
 * 参加すれば同じ返信を送れる。投稿権限がないなど、ほかの 403 は含まない。`code` を優先し、`code` が
 * ない応答（コードを返す前のサーバー）だけ、本文で判定する。
 */
val ApiException.requiresThreadJoin: Boolean
    get() {
        if (status != 403) return false
        val code = (body?.get("code") as? JsonPrimitive)?.contentOrNull
        if (code != null) return code == THREAD_JOIN_REQUIRED_CODE
        val message = (body?.get("error") as? JsonPrimitive)?.contentOrNull
        return message?.contains(THREAD_JOIN_REQUIRED_MESSAGE) == true
    }

/** REST is the only write path. Authentication is resolved again for every request. */
class ZooApi(val config: Config, val platform: Platform, private val http: HttpClient) {
    suspend fun request(
        path: String,
        method: String = "GET",
        body: JsonElement? = null,
        /** 追加のヘッダー（E2EE の直前の再認証の `X-Zoo-Reauth` など）。 */
        extraHeaders: Map<String, String> = emptyMap(),
    ): JsonElement {
        val auth = platform.headers() + extraHeaders
        val (response, raw) =
            try {
                val result =
                    http.request(endpoint(config.apiBaseUrl, path)) {
                        this.method = HttpMethod.parse(method)
                        headers { auth.forEach { (key, value) -> append(key, value) } }
                        if (body != null) {
                            contentType(ContentType.Application.Json)
                            setBody(body.toString())
                        }
                    }
                result to result.bodyAsText()
            } catch (failure: Throwable) {
                // Wasm fetch rejects with kotlin.Error. Feature effects catch Exception;
                // letting an Error escape cancels the composition before cached rows paint.
                if (failure is Exception) throw failure
                throw IllegalStateException("HTTP transport failed", failure)
            }
        if (response.status.value !in 200..299)
            throw ApiException(
                response.status.value,
                try {
                    json.parseToJsonElement(raw) as? JsonObject
                } catch (_: Exception) {
                    null
                },
            )
        return if (raw.isBlank()) JsonNull else json.parseToJsonElement(raw)
    }

    /** Audio-call signaling is a call-owned stream, independent of screen visibility. */
    suspend fun streamCall(channelId: String, sessionId: String, event: suspend (String, Row) -> Unit) =
        streamEvents("/api/channels/${channelId.encodeURLPathPart()}/call/stream", HttpMethod.Get, mapOf("session_id" to sessionId), event)
    suspend fun streamPhone(sessionId: String, event: suspend (String, Row) -> Unit) =
        streamEvents("/api/phone/sessions/${sessionId.encodeURLPathPart()}/stream", HttpMethod.Post, emptyMap(), event)
    private suspend fun streamEvents(path: String, method: HttpMethod, parameters: Map<String, String>, event: suspend (String, Row) -> Unit) {
        val auth = platform.headers()
        http.prepareRequest(endpoint(config.apiBaseUrl, path)) {
            this.method = method
            parameters.forEach { (key, value) -> parameter(key, value) }
            headers { auth.forEach { (key, value) -> append(key, value) } }
            timeout {
                requestTimeoutMillis = io.ktor.client.plugins.HttpTimeoutConfig.INFINITE_TIMEOUT_MS
                socketTimeoutMillis = 45_000
            }
        }.execute { response ->
            if (response.status.value !in 200..299) throw ApiException(response.status.value)
            val source = response.bodyAsChannel()
            var name = "message"
            val data = StringBuilder()
            while (true) {
                val line = source.readUTF8Line(65_536) ?: break
                if (line.isEmpty()) {
                    if (data.isNotEmpty()) event(name, json.parseToJsonElement(data.toString()).jsonObject)
                    name = "message"; data.clear()
                } else if (line.startsWith("event:")) name = line.substringAfter(':').trim()
                else if (line.startsWith("data:")) {
                    if (data.isNotEmpty()) data.append('\n')
                    data.append(line.substringAfter(':').trimStart())
                    check(data.length <= 65_536) { "Call event too large" }
                }
            }
        }
    }

    /**
     * 署名付きの URL（S3 の presign）に `bytes` を PUT する。認証ヘッダーは付けない（S3 に渡さないため）。
     * 暗号化した添付（#364）の暗号文を上げるのに使う。
     */
    suspend fun putBytes(url: String, bytes: ByteArray, contentType: String) {
        val response =
            http.request(url) {
                method = HttpMethod.Put
                setBody(io.ktor.http.content.ByteArrayContent(bytes, ContentType.parse(contentType)))
            }
        if (response.status.value !in 200..299) throw ApiException(response.status.value)
    }

    /** 署名付きの URL（S3 の presign）から取得する。認証ヘッダーは付けない。 */
    suspend fun getBytes(url: String): ByteArray {
        val response = http.request(url) { method = HttpMethod.Get }
        if (response.status.value !in 200..299) throw ApiException(response.status.value)
        return response.readRawBytes()
    }
}

/** One Shape per resource/channel; its owner closes the HTTP stream on navigation or logout. */
class SyncedTable(
    private val name: String,
    private val config: Config,
    private val platform: Platform,
    private val http: HttpClient,
    scope: CoroutineScope,
    private val params: Map<String, String> = emptyMap(),
    private val streamOptions: ShapeStreamOptions = ShapeStreamOptions(liveSse = true),
    private val synchronize: Boolean = true,
    private val persist: Boolean = true,
) {
    private val tableScope =
        CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]))
    private val serverRows = MutableStateFlow<List<Row>>(emptyList())
    val replicatedRows: StateFlow<List<Row>> = serverRows.asStateFlow()
    private val successfulSnapshot = MutableStateFlow<List<Row>?>(null)
    /** Only committed network snapshots; cache loads, resets and failures never publish here. */
    val synchronizedRows: StateFlow<List<Row>?> = successfulSnapshot.asStateFlow()
    private val optimistic = MutableStateFlow<Map<String, Row?>>(emptyMap())
    val optimisticChanges = optimistic.asStateFlow()
    val rows =
        combine(serverRows, optimistic) { rows, pending ->
                val merged = rows.associateBy(::rowKey).toMutableMap()
                pending.forEach { (key, row) ->
                    if (row == null) merged.remove(key) else merged[key] = row
                }
                merged.values.toList()
            }
            .stateIn(tableScope, SharingStarted.Eagerly, emptyList())
    val loading = MutableStateFlow(true)
    val error = MutableStateFlow<Throwable?>(null)
    val persistenceError = MutableStateFlow(false)
    private val seenTxids = MutableStateFlow<Set<String>>(emptySet())

    suspend fun awaitTxId(txid: String, timeout: Long = 15_000) {
        withTimeout(timeout) { seenTxids.first { txid in it } }
    }

    private var stream: ShapeStream? = null
    private var refreshJob: Job? = null
    private val cacheKey =
        "zoo-compose:v2:${config.apiBaseUrl}:${config.shapeProxyPath}:${config.shapeMode}:${config.electricShapeUrl}:${config.userId}:$name:${params.entries.sortedBy { it.key }}"
    private var worker = startSync(readCache = true)

    private fun startSync(readCache: Boolean): Job =
        tableScope.launch {
            if (!synchronize) {
                loading.value = false
                return@launch
            }
            try {
                (if (persist && readCache) platform.readCache(cacheKey) else null)?.let { raw ->
                    serverRows.value = json.parseToJsonElement(raw).jsonArray.map { it.jsonObject }
                    loading.value = false
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                persistenceError.value = true
            }
            val direct = config.shapeMode == "direct"
            val path =
                if (direct) config.electricShapeUrl
                else endpoint(config.apiBaseUrl, "${config.shapeProxyPath}/$name")
            val table = if (name == "channel_users") "channel_user" else name
            val shapeParams =
                if (!direct) params
                else
                    buildMap {
                        put("table", table)
                        params["channel_id"]?.let { channelId ->
                            put(
                                "where",
                                if (name == "message_reactions")
                                    "message_id IN (SELECT id FROM messages WHERE channel_id = $1 AND deleted_at IS NULL)"
                                else "channel_id = $1",
                            )
                            put("params[1]", channelId)
                        }
                        if (name == "notifications") {
                            put("where", "user_id = $1")
                            put("params[1]", config.userId)
                        }
                    }
            val shapeClient =
                ShapeClient(
                    path,
                    httpClient = http,
                    headers = { if (direct) emptyMap() else platform.headers() },
                    queryParams = { shapeParams },
                )
            val source = ShapeStream(shapeClient, scope = this, options = streamOptions)
            stream = source
            val materialized = Shape(source)
            val pendingTxids = mutableSetOf<String>()
            val snapshots = Channel<List<Row>>(Channel.CONFLATED)
            // Enqueue committed snapshots even before the writer starts, including an empty shape.
            // Observing serverRows with drop(1) could skip an already received initial snapshot.
            val cacheWriter = launch {
                for (rows in snapshots) {
                    try {
                        if (persist) platform.writeCache(cacheKey, JsonArray(rows).toString())
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        persistenceError.value = true
                    }
                }
            }
            fun acceptSnapshot(rows: List<Row>) {
                serverRows.value = rows
                error.value = null
                successfulSnapshot.value = rows.toList()
                loading.value = false
                snapshots.trySend(rows.toList())
            }
            // Shape is subscribed first, so this observes its consistent materialized snapshot.
            val unsubscribeTx =
                source.subscribe(
                    { batch ->
                        if (
                            batch.any {
                                it.jsonObject["headers"]
                                    ?.jsonObject
                                    ?.get("control")
                                    ?.jsonPrimitive
                                    ?.content == "must-refetch"
                            }
                        ) {
                            pendingTxids.clear()
                            serverRows.value = emptyList()
                            loading.value = true
                        }
                        val txids =
                            batch.flatMap { message ->
                                (message.jsonObject["headers"]?.jsonObject?.get("txids")
                                        as? JsonArray)
                                    ?.map { it.jsonPrimitive.content }
                                    .orEmpty()
                            }
                        pendingTxids.addAll(txids)
                        if (materialized.isUpToDate) {
                            acceptSnapshot(materialized.currentRows)
                            seenTxids.value =
                                (seenTxids.value.toList() + pendingTxids).takeLast(512).toSet()
                            pendingTxids.clear()
                        }
                    },
                    {
                        error.value = it
                        loading.value = false
                    },
                )
            materialized.subscribe { change ->
                if (materialized.isUpToDate && materialized.error == null) {
                    acceptSnapshot(change.rows)
                }
            }
            try {
                materialized.awaitRows()
                acceptSnapshot(materialized.currentRows)
                awaitCancellation()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error.value = e
                loading.value = false
            } finally {
                cacheWriter.cancel()
                unsubscribeTx()
                materialized.close()
                source.close()
                shapeClient.close()
            }
        }

    suspend fun write(
        api: ZooApi,
        path: String,
        method: String,
        body: JsonElement? = null,
        row: Row? = null,
        key: String? = row?.let(::rowKey),
    ): JsonElement {
        check(key == null || !optimistic.value.containsKey(key)) { "Write already pending" }
        if (key != null) optimistic.update { it + (key to row) }
        try {
            val result = api.request(path, method, body)
            val txid = (result as? JsonObject)?.get("txid")?.jsonPrimitive?.contentOrNull
            if (txid == null && config.txidMode != "awaitMatch") return result
            if (txid != null && config.txidMode == "awaitTxId") {
                awaitTxId(txid)
                // The materializer may receive the same batch after the txid subscriber.
                yield()
            } else if (config.txidMode == "awaitMatch") {
                val saved = (result as? JsonObject)?.get("data") as? JsonObject
                // The server may normalize parent_id (thread migration), so match the saved value.
                val savedParent = saved?.get("parent_id") as? JsonPrimitive
                val expected =
                    if (row != null && savedParent != null)
                        JsonObject(row + ("parent_id" to savedParent))
                    else row ?: saved
                val matchKey = key ?: expected?.let(::rowKey)
                if (matchKey != null)
                    withTimeout(15_000) {
                        serverRows.first { rows ->
                            if (method == "DELETE") rows.none { rowKey(it) == matchKey }
                            else
                                rows.any { current ->
                                    rowKey(current) == matchKey &&
                                        expected
                                            .orEmpty()
                                            .filterKeys { field ->
                                                field !in
                                                    setOf("created_at", "updated_at", "joined_at")
                                            }
                                            .all { (field, value) ->
                                                current[field] == value ||
                                                    (current[field] as? JsonPrimitive)
                                                        ?.contentOrNull ==
                                                        (value as? JsonPrimitive)?.contentOrNull
                                            }
                                }
                        }
                    }
            }
            return result
        } finally {
            if (key != null) optimistic.update { it - key }
        }
    }

    fun close() {
        tableScope.cancel()
    }

    fun refresh() {
        if (refreshJob?.isActive == true) return
        refreshJob = tableScope.launch {
            val source = stream ?: return@launch
            if (source.error != null) {
                // A terminal Electric4k runner cannot be restarted by forceDisconnectAndRefresh.
                worker.cancelAndJoin()
                loading.value = true
                worker = startSync(readCache = false)
                loading.first { !it }
                return@launch
            }
            if (source.isPaused()) return@launch
            try {
                source.forceDisconnectAndRefresh()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Visibility may pause the stream between the check and the refresh call.
                if (!source.isPaused() || source.error != null) {
                    error.value = e
                    loading.value = false
                }
            }
        }
    }
}

class Repository(
    val config: Config,
    val platform: Platform,
    private val http: HttpClient,
    private val scope: CoroutineScope,
    private val synchronize: Boolean = true,
) {
    val api = ZooApi(config, platform, http)
    val channels = SyncedTable("channels", config, platform, http, scope, synchronize = synchronize)
    val users = SyncedTable("users", config, platform, http, scope, synchronize = synchronize)
    val members = SyncedTable("channel_users", config, platform, http, scope, synchronize = synchronize)
    val notifications = SyncedTable("notifications", config, platform, http, scope, synchronize = synchronize)
    private val dynamic = mutableMapOf<Pair<String, String>, SyncedTable>()

    /** Read an existing subscription without starting a channel stream for an OS notification. */
    fun cachedMessages(channelId: String): List<Row> =
        dynamic["messages" to channelId]?.replicatedRows?.value.orEmpty()

    fun table(name: String, channelId: String = ""): SyncedTable =
        when (name) {
            "channels" -> channels
            "users" -> users
            "channel_users" -> members
            "notifications" -> notifications
            "messages",
            "attachments",
            "message_reactions" ->
                dynamic.getOrPut(name to channelId) {
                    require(channelId.isNotBlank())
                    SyncedTable(
                        name,
                        config,
                        platform,
                        http,
                        scope,
                        mapOf("channel_id" to channelId),
                        synchronize = synchronize,
                    )
                }
            else -> error("Unknown Shape resource")
        }

    fun releaseChannel(channelId: String) {
        dynamic.keys
            .filter { it.second == channelId }
            .toList()
            .forEach { dynamic.remove(it)?.close() }
    }

    fun messages(channelId: String) = table("messages", channelId)

    fun attachments(channelId: String) = table("attachments", channelId)

    fun reactions(channelId: String) = table("message_reactions", channelId)

    fun refresh() {
        (listOf(channels, users, members, notifications) + dynamic.values).forEach { it.refresh() }
    }

    fun close() {
        (listOf(channels, users, members, notifications) + dynamic.values).forEach { it.close() }
        dynamic.clear()
        http.close()
    }
}
