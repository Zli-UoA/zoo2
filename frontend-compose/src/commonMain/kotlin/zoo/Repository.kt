package zoo

import electric4k.*
import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*

class ApiException(val status: Int, val body: JsonObject? = null) : Exception("HTTP $status")

/** REST is the only write path. Authentication is resolved again for every request. */
class ZooApi(val config: Config, val platform: Platform, private val http: HttpClient) {
    suspend fun request(
        path: String,
        method: String = "GET",
        body: JsonElement? = null,
    ): JsonElement {
        val auth = platform.headers()
        val response =
            http.request(endpoint(config.apiBaseUrl, path)) {
                this.method = HttpMethod.parse(method)
                headers { auth.forEach { (key, value) -> append(key, value) } }
                if (body != null) {
                    contentType(ContentType.Application.Json)
                    setBody(body.toString())
                }
            }
        val raw = response.bodyAsText()
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
) {
    private val tableScope =
        CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]))
    private val serverRows = MutableStateFlow<List<Row>>(emptyList())
    val replicatedRows: StateFlow<List<Row>> = serverRows.asStateFlow()
    private val optimistic = MutableStateFlow<Map<String, Row?>>(emptyMap())
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

    private var shape: Shape<JsonObject>? = null
    private var stream: ShapeStream? = null
    private var client: ShapeClient? = null
    private var unsubscribeTx: (() -> Unit)? = null
    private val cacheKey =
        "zoo-compose:v1:${config.apiBaseUrl}:${config.shapeProxyPath}:${config.shapeMode}:${config.electricShapeUrl}:${config.userId}:$name:${params.entries.sortedBy { it.key }}"
    private val worker = tableScope.launch {
        try {
            platform.readCache(cacheKey)?.let { raw ->
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
        client = shapeClient
        val source = ShapeStream(shapeClient, scope = tableScope, options = streamOptions)
        stream = source
        val materialized = Shape(source)
        shape = materialized
        val pendingTxids = mutableSetOf<String>()
        // Shape is subscribed first, so this observes its consistent materialized snapshot.
        unsubscribeTx =
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
                    val txids = batch.flatMap { message ->
                        (message.jsonObject["headers"]?.jsonObject?.get("txids") as? JsonArray)
                            ?.map { it.jsonPrimitive.content }
                            .orEmpty()
                    }
                    pendingTxids.addAll(txids)
                    if (materialized.isUpToDate) {
                        serverRows.value = materialized.currentRows
                        loading.value = false
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
            if (materialized.isUpToDate) {
                serverRows.value = change.rows
                loading.value = false
                error.value = materialized.error
            }
        }
        // Serial persistence prevents an older save overtaking a newer snapshot.
        launch {
            serverRows.drop(1).collect { rows ->
                if (materialized.isUpToDate) {
                    try {
                        platform.writeCache(cacheKey, JsonArray(rows).toString())
                    } catch (_: Exception) {
                        persistenceError.value = true
                    }
                }
            }
        }
        try {
            materialized.awaitRows()
            serverRows.value = materialized.currentRows
            loading.value = false
            awaitCancellation()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error.value = e
            loading.value = false
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
                val expected = row ?: (result as? JsonObject)?.get("data") as? JsonObject
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
        shape?.close()
        unsubscribeTx?.invoke()
        stream?.close()
        client?.close()
    }

    fun refresh() {
        tableScope.launch { stream?.forceDisconnectAndRefresh() }
    }
}

class Repository(
    val config: Config,
    val platform: Platform,
    private val http: HttpClient,
    private val scope: CoroutineScope,
) {
    val api = ZooApi(config, platform, http)
    val channels = SyncedTable("channels", config, platform, http, scope)
    val users = SyncedTable("users", config, platform, http, scope)
    val members = SyncedTable("channel_users", config, platform, http, scope)
    val notifications = SyncedTable("notifications", config, platform, http, scope)
    private val dynamic = mutableMapOf<Pair<String, String>, SyncedTable>()

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
