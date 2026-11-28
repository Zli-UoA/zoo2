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

    /** Audio-call signaling is a call-owned stream, independent of screen visibility. `sfu=1`: the engine can move to the SFU (docs/calls-sfu.md). */
    suspend fun streamCall(channelId: String, sessionId: String, event: suspend (String, Row) -> Unit) =
        streamEvents("/api/channels/${channelId.encodeURLPathPart()}/call/stream", HttpMethod.Get, mapOf("session_id" to sessionId, "sfu" to "1"), event)
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

/** 1 回の subset snapshot で取る行数と、オフライン用の控えの行数。Web の `MESSAGE_PAGE_SIZE` と揃える。 */
const val MESSAGE_PAGE_SIZE = 50

/** on-demand の messages で、1 ページずつ読む範囲。 */
sealed interface PageScope {
    /** チャンネル直下（`parent_id IS NULL`）。 */
    data object Main : PageScope

    /** `rootId` への返信。 */
    data class Replies(val rootId: String) : PageScope
}

/**
 * 範囲ごとの読み込み状態。`loaded` は最初のページが画面の行に反映済みであること、`hasOlder` は直前のページが
 * 満杯（= まだ古い行があるかもしれない）であること。
 */
data class PageState(
    val loaded: Boolean = false,
    val loading: Boolean = false,
    val hasOlder: Boolean = false,
    /** 取得済みの範囲の古い端（`created_at` と `id`）。 */
    val oldest: Pair<String, String>? = null,
)

/** `created_at` の降順に、同じ時刻でも順序が決まるよう `id` を足した並び。 */
internal fun compareNewestFirst(a: Row, b: Row): Int {
    val byTime = b.text("created_at").compareTo(a.text("created_at"))
    return if (byTime != 0) byTime else b.text("id").compareTo(a.text("id"))
}

/** オフライン用の控えに残す行: チャンネル直下で、削除されていない最新のページ。 */
internal fun feedCacheRows(rows: List<Row>, limit: Int = MESSAGE_PAGE_SIZE): List<Row> =
    rows
        .filter { it.text("parent_id").isEmpty() && !it.present("deleted_at") }
        .sortedWith(::compareNewestFirst)
        .take(limit)

internal data class CachedFeed(val rows: List<Row>, val page: PageState? = null) {
    fun encode(): String = buildJsonObject {
        put("rows", JsonArray(rows))
        page?.let { coverage ->
            put("has_older", coverage.hasOlder)
            coverage.oldest?.let { (time, id) ->
                put("oldest_time", time)
                put("oldest_id", id)
            }
        }
    }.toString()

    companion object {
        fun decode(value: JsonElement): CachedFeed {
            if (value is JsonArray) return CachedFeed(value.map { it.jsonObject })
            val item = value.jsonObject
            val page = (item["has_older"] as? JsonPrimitive)?.booleanOrNull?.let { hasOlder ->
                val time = item.text("oldest_time")
                val id = item.text("oldest_id")
                PageState(loaded = true, hasOlder = hasOlder,
                    oldest = if (time.isNotEmpty() && id.isNotEmpty()) time to id else null)
            }
            return CachedFeed(item.getValue("rows").jsonArray.map { it.jsonObject }, page)
        }
    }
}

internal fun cacheFeed(rows: List<Row>, page: PageState): CachedFeed {
    val window = feedCacheRows(rows, MESSAGE_PAGE_SIZE + 1)
    val retained = window.take(MESSAGE_PAGE_SIZE)
    val trimmed = retained.size < window.size
    val retainedOldest = retained.lastOrNull()?.let { it.text("created_at") to it.text("id") }
    // A focused-message snapshot outside the page history cannot extend its contiguous range.
    val oldest = if (trimmed && retainedOldest != null) {
        listOfNotNull(page.oldest, retainedOldest).maxWith(compareBy({ it.first }, { it.second }))
    } else page.oldest
    return CachedFeed(retained, page.copy(loading = false, hasOlder = page.hasOlder || trimmed, oldest = oldest))
}

/**
 * One Shape per resource/channel; its owner closes the HTTP stream on navigation or logout.
 *
 * `onDemand` の表は `log=changes_only` で購読し、行は subset snapshot（[loadOlder] / [loadAround]）で
 * 1 ページずつ取る。新着・編集・削除は、取得済みの行についてだけ live で届く。
 */
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
    private val onDemand: Boolean = false,
    initialRows: List<Row> = emptyList(),
    initialCachedPage: PageState? = null,
) {
    private val tableScope =
        CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]))
    private val serverRows = MutableStateFlow(initialRows)
    val replicatedRows: StateFlow<List<Row>> = serverRows.asStateFlow()
    internal var navigationFeed: CachedFeed? = null
        private set
    private val cachedPage = MutableStateFlow(initialCachedPage)
    val cachedMainPage: StateFlow<PageState?> = cachedPage.asStateFlow()
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
            .stateIn(tableScope, SharingStarted.Eagerly, initialRows)
    val loading = MutableStateFlow(true)
    val error = MutableStateFlow<Throwable?>(null)
    private val initialSyncCompleted = MutableStateFlow(false)
    val initialSyncReady: StateFlow<Boolean> = initialSyncCompleted.asStateFlow()
    private val initialPageFailure = MutableStateFlow(false)
    val initialPageFailed: StateFlow<Boolean> = initialPageFailure.asStateFlow()
    val persistenceError = MutableStateFlow(false)
    private val seenTxids = MutableStateFlow<Set<String>>(emptySet())
    /** 取得した subset snapshot の可視範囲。snapshot に含まれて捨てられた変更の txid を、確認済みとして扱う。 */
    private val snapshotMetas = MutableStateFlow<List<SnapshotMetadata>>(emptyList())

    suspend fun awaitTxId(txid: String, timeout: Long = 15_000) {
        val id = txid.toULongOrNull()
        withTimeout(timeout) {
            combine(seenTxids, snapshotMetas) { seen, metas ->
                    txid in seen || (id != null && metas.any { isVisibleInSnapshot(id, it) })
                }
                .first { it }
        }
    }

    /** 取得済みの範囲。取り直し（must-refetch）のとき、この単位でやり直す。`scope` か `aroundId` のどちらか。 */
    private class Requested(val scope: PageScope?, val aroundId: String?)

    /** 範囲ごとの読み込み状態（on-demand のみ）。 */
    private val pages = MutableStateFlow<Map<PageScope, PageState>>(emptyMap())
    val pageStates: StateFlow<Map<PageScope, PageState>> = pages.asStateFlow()
    /** 最初のページが画面の行に反映済みか。それまでは、控えの行を live の行で上書きしない。 */
    private val accepting = MutableStateFlow(false)
    private val requested = MutableStateFlow<List<Requested>>(emptyList())
    private var refetchPending = false
    /** must-refetch で消えた範囲を取り直している間。この間は古いページを足さない（`oldest` が先に進むのを防ぐ）。 */
    private val refetching = MutableStateFlow(false)
    /** must-refetch のたびに増える。取り直し前に始めた `loadOlder` の結果を捨てるのに使う。 */
    private var refetchEpoch = 0
    private val generation = MutableStateFlow(0)
    val pageGeneration: StateFlow<Int> = generation.asStateFlow()
    /** 取り直しのジョブと、まだ取り直していない範囲。must-refetch が続いても、範囲を引き継ぐ。 */
    private var refetchJob: Job? = null
    private var refetchTargets = emptyList<Requested>()
    private var streamReady = CompletableDeferred<ShapeStream>()
    private var acceptNow: () -> Unit = {}
    private var saveMainCache: () -> Unit = {}

    private var stream: ShapeStream? = null
    private var refreshJob: Job? = null
    private val subsetRequests = ShapeSubsetRequests()
    private val cacheKey =
        "zoo-compose:${if (onDemand) "v3" else "v2"}:${config.apiBaseUrl}:${config.shapeProxyPath}:${config.shapeMode}:${config.electricShapeUrl}:${config.userId}:$name:${params.entries.sortedBy { it.key }}"
    /** on-demand に移す前の、全件入りの控え（添付・リアクションの別 Shape の分も）。読まずに消す。 */
    private val legacyCacheKeys =
        listOf(name, "attachments", "message_reactions").map { resource ->
            "zoo-compose:v2:${config.apiBaseUrl}:${config.shapeProxyPath}:${config.shapeMode}:${config.electricShapeUrl}:${config.userId}:$resource:${params.entries.sortedBy { it.key }}"
        }
    private var worker = startSync(readCache = true)

    private fun pageSubset(scope: PageScope, before: Pair<String, String>?): SubsetParams {
        val conditions = mutableListOf<String>()
        val values = linkedMapOf<String, JsonPrimitive>()
        when (scope) {
            PageScope.Main -> conditions += "parent_id IS NULL"
            is PageScope.Replies -> {
                values["1"] = JsonPrimitive(scope.rootId)
                conditions += "parent_id = \$1"
            }
        }
        conditions += "deleted_at IS NULL"
        if (before != null) {
            val a = values.size + 1
            val b = a + 1
            values["$a"] = JsonPrimitive(before.first)
            values["$b"] = JsonPrimitive(before.second)
            conditions += "(created_at < \$$a OR (created_at = \$$a AND id < \$$b))"
        }
        return SubsetParams(
            where = conditions.joinToString(" AND "),
            params = values,
            limit = MESSAGE_PAGE_SIZE,
            orderBy = "created_at DESC, id DESC",
        )
    }

    private suspend fun fetchSubset(subset: SubsetParams): SubsetSnapshot {
        val source = streamReady.await()
        val snapshot = subsetRequests.request { source.requestSnapshot(subset) }
        snapshotMetas.update { (it + snapshot.metadata).takeLast(32) }
        // 行は snapshot の配信の中で Shape に入っている。最初のメインのページが届いたら、画面へ反映する。
        if (accepting.value) acceptNow()
        return snapshot
    }

    /**
     * 次の 1 ページを取る。最初の呼び出し（`loaded` が偽）は、最新のページ。取得済みの範囲や、同じ範囲を取っている
     * 最中は何もせず偽を返す。取り直し（must-refetch 後）の間は、終わるまで待ってから取る。
     */
    suspend fun loadOlder(scope: PageScope): Boolean {
        if (!onDemand || !synchronize) return false
        while (true) {
            refetching.first { !it }
            val epoch = refetchEpoch
            val result = loadPage(scope)
            // 取り直しが始まって、この結果が捨てられたときは、取り直しの後にやり直す。
            if (epoch == refetchEpoch) return result
        }
    }

    private suspend fun loadPage(scope: PageScope): Boolean {
        val current = pages.value[scope] ?: PageState()
        if (current.loading || (current.loaded && !current.hasOlder)) return false
        if (requested.value.none { it.scope == scope }) requested.update { it + Requested(scope, null) }
        pages.update { it + (scope to current.copy(loading = true)) }
        val epoch = refetchEpoch
        try {
            val snapshot = fetchSubset(pageSubset(scope, current.oldest))
            // 取り直しが始まった: 範囲の状態は取り直しが作り直す。この結果では進めない。
            if (epoch != refetchEpoch) return false
            val values = snapshot.data.mapNotNull { it.jsonObject["value"] as? JsonObject }
            val oldest = values.lastOrNull()?.let { it.text("created_at") to it.text("id") } ?: current.oldest
            pages.update {
                it + (scope to PageState(
                    loaded = true,
                    loading = false,
                    hasOlder = snapshot.data.size >= MESSAGE_PAGE_SIZE,
                    oldest = oldest,
                ))
            }
            if (scope == PageScope.Main) {
                initialPageFailure.value = false
                accepting.value = true
                acceptNow()
                saveMainCache()
            }
            return true
        } catch (e: CancellationException) {
            if (epoch == refetchEpoch) pages.update { it + (scope to current) }
            throw e
        } catch (_: Exception) {
            if (epoch == refetchEpoch) {
                pages.update { it + (scope to current) }
                if (scope == PageScope.Main && !current.loaded) initialPageFailure.value = true
            }
            return false
        }
    }

    /** 1 行を取る。通信の失敗は `Result.failure`、行がなかったときは `success(false)`。 */
    private suspend fun fetchAround(id: String): Result<Boolean> {
        if (requested.value.none { it.aroundId == id }) requested.update { it + Requested(null, id) }
        return try {
            val subset =
                SubsetParams(
                    where = "id = \$1",
                    params = mapOf("1" to JsonPrimitive(id)),
                    limit = 1,
                    orderBy = "created_at DESC, id DESC",
                )
            Result.success(fetchSubset(subset).data.isNotEmpty())
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    /** 通知・リンク・検索の対象や、スレッドの親を、1 行だけ取る。窓の外でもよく、削除済みの行も取る。 */
    suspend fun loadAround(id: String): Boolean {
        if (!onDemand || !synchronize) return false
        return fetchAround(id).getOrDefault(false)
    }

    private fun startSync(readCache: Boolean): Job {
        initialSyncCompleted.value = false
        initialPageFailure.value = false
        streamReady = CompletableDeferred()
        val ready = streamReady
        accepting.value = false
        refetchPending = false
        refetching.value = false
        refetchEpoch++
        refetchJob = null
        refetchTargets = emptyList()
        requested.value = emptyList()
        pages.value = emptyMap()
        generation.value = refetchEpoch
        return tableScope.launch {
            if (!synchronize) {
                loading.value = false
                return@launch
            }
            if (onDemand && persist && readCache) {
                try {
                    legacyCacheKeys.forEach { platform.removeCache(it) }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {}
            }
            try {
                (if (persist && readCache && serverRows.value.isEmpty()) platform.readCache(cacheKey) else null)?.let { raw ->
                    val value = json.parseToJsonElement(raw)
                    if (onDemand) {
                        val cache = CachedFeed.decode(value)
                        serverRows.value = cache.rows
                        cachedPage.value = cache.page
                    } else {
                        serverRows.value = value.jsonArray.map { it.jsonObject }
                    }
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
            // channel_users は channel_user、channel_notification_settings は
            // user_channel_notification_settings をそれぞれの条件で配る。設定ごとに REST を
            // 増やさず、Shape の更新を画面へ反映する。
            val table =
                when (name) {
                    "channel_users" -> "channel_user"
                    "channel_notification_settings" -> "user_channel_notification_settings"
                    else -> name
                }
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
                        if (name == "notifications" || name == "channel_notification_settings") {
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
            val source =
                ShapeStream(
                    shapeClient,
                    request =
                        if (onDemand) ShapeRequest(log = LogMode.ChangesOnly) else ShapeRequest(),
                    scope = this,
                    options = streamOptions,
                )
            stream = source
            val materialized = Shape(source)
            val pendingTxids = mutableSetOf<String>()
            fun acceptTxids() {
                seenTxids.value = (seenTxids.value.toList() + pendingTxids).takeLast(512).toSet()
                pendingTxids.clear()
            }
            val snapshots = Channel<CachedFeed>(Channel.CONFLATED)
            // Keep the cached rows and their verified page coverage in the same write.
            val cacheWriter = launch {
                for (snapshot in snapshots) {
                    try {
                        if (persist) {
                            val encoded = if (onDemand) snapshot.encode() else JsonArray(snapshot.rows).toString()
                            if (onDemand) navigationFeed =
                                snapshot.takeIf { encoded.length <= 2 * 1024 * 1024 }
                            platform.writeCache(cacheKey, encoded)
                        }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        persistenceError.value = true
                    }
                }
            }
            saveMainCache = {
                if (persist) pages.value[PageScope.Main]?.takeIf { it.loaded }?.let { page ->
                    snapshots.trySend(cacheFeed(serverRows.value, page))
                }
            }
            fun acceptSnapshot(rows: List<Row>) {
                if (onDemand) cachedPage.value = null
                serverRows.value = rows
                error.value = null
                loading.value = false
                if (onDemand) {
                    navigationFeed = null
                    saveMainCache()
                } else {
                    successfulSnapshot.value = rows.toList()
                    snapshots.trySend(CachedFeed(rows.toList()))
                }
            }
            acceptNow = { acceptSnapshot(materialized.currentRows) }
            fun control(message: JsonElement): String? =
                message.jsonObject["headers"]?.jsonObject?.get("control")?.jsonPrimitive?.content
            // Shape is subscribed first, so this observes its consistent materialized snapshot.
            val unsubscribeTx =
                source.subscribe(
                    { batch ->
                        val upToDate = batch.any { control(it) == "up-to-date" }
                        val changed =
                            batch.any { it.jsonObject["headers"]?.jsonObject?.containsKey("operation") == true }
                        if (batch.any { control(it) == "must-refetch" }) {
                            pendingTxids.clear()
                            refetchEpoch++
                            refetchPending = true
                            if (onDemand) {
                                // 表示中の行は、取り直した範囲で置き換わるまで残す。
                                accepting.value = false
                                refetching.value = true
                                // 先に世代を進めてから、進行中の取り直しを止める。止めたジョブの結果は、範囲にも
                                // `refetching` にも触れない。まだ取っていない範囲は、次の取り直しへ引き継ぐ。
                                refetchJob?.cancel()
                                refetchJob = null
                                refetchTargets = (refetchTargets + requested.value).distinctBy { it.scope to it.aroundId }
                                requested.value = emptyList()
                                pages.value = emptyMap()
                                generation.value = refetchEpoch
                                if (serverRows.value.isEmpty()) loading.value = true
                            } else {
                                serverRows.value = emptyList()
                                loading.value = true
                            }
                        }
                        val txids =
                            batch.flatMap { message ->
                                (message.jsonObject["headers"]?.jsonObject?.get("txids")
                                        as? JsonArray)
                                    ?.map { it.jsonPrimitive.content }
                                    .orEmpty()
                            }
                        pendingTxids.addAll(txids)
                        if (onDemand) {
                            if (accepting.value && (changed || upToDate)) acceptSnapshot(materialized.currentRows)
                            if (upToDate || (accepting.value && changed)) acceptTxids()
                        } else if (materialized.isUpToDate) {
                            acceptSnapshot(materialized.currentRows)
                            acceptTxids()
                        }
                    },
                    {
                        error.value = it
                        loading.value = false
                    },
                )
            materialized.subscribe { change ->
                val failure = materialized.error
                if (failure != null) {
                    error.value = failure
                    loading.value = false
                } else if (!onDemand && materialized.isUpToDate) {
                    acceptSnapshot(change.rows)
                }
            }
            // Catch-up can complete with HTTP 204 or a replayed control message.
            // Electric updates the stream status in both cases without delivering an up-to-date batch.
            val refetchWatcher = launch {
                source.status.collect {
                    val status = source.status.value
                    if (!refetchPending || !status.isUpToDate || status.paused) return@collect
                    if (!onDemand) {
                        val epoch = refetchEpoch
                        val rows = try {
                            materialized.awaitRows()
                        } catch (cancelled: CancellationException) {
                            throw cancelled
                        } catch (_: Exception) {
                            return@collect // The stream and materializer subscribers report the failure.
                        }
                        if (epoch == refetchEpoch) {
                            refetchPending = false
                            if (!materialized.isUpToDate) {
                                acceptSnapshot(rows)
                                acceptTxids()
                            }
                        }
                        return@collect
                    }
                    refetchPending = false
                    val targets = (refetchTargets + requested.value).distinctBy { it.scope to it.aroundId }
                    refetchTargets = targets
                    requested.value = emptyList()
                    pages.value = emptyMap()
                    val epoch = refetchEpoch
                    refetchJob =
                        launch {
                            var attempt = 0
                            // 取り直しが済むまで、`loadOlder` は何もしない。失敗したら、成功するまで
                            // 間隔を空けて（最大 10 秒）やり直す。画面を離れる・ログアウト・次の
                            // must-refetch で取り消される。
                            suspend fun retry(block: suspend () -> Boolean) {
                                while (!block()) {
                                    attempt++
                                    delay(minOf(1_000L * attempt, 10_000L))
                                }
                            }
                            val scopes = (targets.mapNotNull { it.scope } + PageScope.Main).distinct()
                            scopes.forEach { scope -> retry { loadPage(scope) } }
                            // 対象行は、通信の失敗だけやり直す（行がなくなっていた場合は、そのまま終える）。
                            targets.mapNotNull { it.aroundId }.forEach { id ->
                                retry { fetchAround(id).isSuccess }
                            }
                            // 取り消しは協調的なので、世代も確かめる。次の取り直しが始まっていたら、解除しない。
                            if (epoch == refetchEpoch) {
                                refetchTargets = emptyList()
                                refetching.value = false
                            }
                        }
                }
            }
            try {
                materialized.awaitRows()
                initialSyncCompleted.value = true
                if (onDemand) {
                    // changes_only の最初の up-to-date は空。行は subset snapshot が届くまで反映しない。
                    ready.complete(source)
                    // 最初のページが取れるまで、間隔を空けて（最大 10 秒）やり直す。遅い応答は待つ。
                    launch {
                        var attempt = 0
                        while (pages.value[PageScope.Main]?.loaded != true) {
                            refetching.first { !it }
                            if (pages.value[PageScope.Main]?.loaded == true || loadPage(PageScope.Main)) break
                            attempt++
                            delay(minOf(1_000L * attempt, 10_000L))
                        }
                    }
                } else {
                    acceptSnapshot(materialized.currentRows)
                }
                awaitCancellation()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error.value = e
                loading.value = false
            } finally {
                ready.completeExceptionally(IllegalStateException("Shape stream closed"))
                acceptNow = {}
                saveMainCache = {}
                cacheWriter.cancel()
                refetchWatcher.cancel()
                unsubscribeTx()
                materialized.close()
                source.close()
                shapeClient.close()
            }
        }
    }

    suspend fun write(
        api: ZooApi,
        path: String,
        method: String,
        body: JsonElement? = null,
        row: Row? = null,
        key: String? = row?.let(::rowKey),
        onAccepted: (() -> Unit)? = null,
    ): JsonElement {
        // changes_only は購読を始めた時点より前の変更を送らない。最初のページが反映されるまで待つ。
        // 待った後で、キーの保留を検査してから登録する（間に中断点を置かない）。
        if (onDemand && synchronize) withTimeoutOrNull(10_000) { accepting.first { it } }
        check(key == null || !optimistic.value.containsKey(key)) { "Write already pending" }
        if (key != null) optimistic.update { it + (key to row) }
        try {
            val result = api.request(path, method, body)
            // Android renders a server-acknowledged row until replicatedRows confirms its ID.
            if (onAccepted != null) {
                onAccepted()
                return result
            }
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

    fun refresh(reconnect: Boolean = false) {
        val previous = refreshJob
        if (!reconnect && previous?.isActive == true) return
        refreshJob = tableScope.launch {
            if (reconnect) previous?.cancelAndJoin()
            val source = stream
            if (synchronize && (source?.error != null || !worker.isActive)) {
                // A terminal Electric4k runner cannot be restarted by forceDisconnectAndRefresh.
                worker.cancelAndJoin()
                stream = null
                loading.value = true
                error.value = null
                worker = startSync(readCache = false)
                return@launch
            }
            // A subset read pauses live and resumes it when the read is retried.
            if (reconnect && subsetRequests.reconnect()) return@launch
            if (source == null || source.isPaused() || (!reconnect && loading.value)) return@launch
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
    val notificationSettings = SyncedTable("channel_notification_settings", config, platform, http, scope, synchronize = synchronize)
    val notifications = SyncedTable("notifications", config, platform, http, scope, synchronize = synchronize)
    private val dynamic = mutableMapOf<Pair<String, String>, SyncedTable>()
    private val recentMessages = linkedMapOf<String, CachedFeed>()
    var navigationCacheLimit: Int = 8
        set(value) {
            field = value.coerceIn(0, 16)
            trimRecentMessages()
        }

    private fun trimRecentMessages() {
        while (recentMessages.size > navigationCacheLimit) recentMessages.remove(recentMessages.keys.first())
    }


    /** Read an existing subscription without starting a channel stream for an OS notification. */
    fun cachedMessages(channelId: String): List<Row> =
        dynamic["messages" to channelId]?.replicatedRows?.value.orEmpty()

    private val linked = lazy {
        LinkedMessages(
            scope,
            openRows = { channelId -> dynamic["messages" to channelId]?.replicatedRows },
            openSource = { channelId -> LinkedChannelTable(channelId, config, platform, http, scope) },
        )
    }
    /** 本文のリンクが指すメッセージ（開いていないチャンネルの 1 件を取る）。 */
    val linkedMessages: LinkedMessages get() = linked.value

    fun table(name: String, channelId: String = ""): SyncedTable =
        when (name) {
            "channels" -> channels
            "users" -> users
            "channel_users" -> members
            "channel_notification_settings" -> notificationSettings
            "notifications" -> notifications
            "messages" ->
                dynamic.getOrPut(name to channelId) {
                    require(channelId.isNotBlank())
                    val cached = recentMessages.remove(channelId)
                    SyncedTable(
                        name,
                        config,
                        platform,
                        http,
                        scope,
                        mapOf("channel_id" to channelId),
                        synchronize = synchronize,
                        onDemand = true,
                        initialRows = cached?.rows.orEmpty(),
                        initialCachedPage = cached?.page,
                    )
                }
            else -> error("Unknown Shape resource")
        }

    fun releaseChannel(channelId: String) {
        dynamic.keys
            .filter { it.second == channelId }
            .toList()
            .forEach { key ->
                val table = dynamic.remove(key) ?: return@forEach
                if (navigationCacheLimit > 0 && table.error.value == null && table.pageStates.value[PageScope.Main]?.loaded == true) {
                    val cached = table.navigationFeed
                    if (cached != null && cached.rows.isNotEmpty()) {
                        recentMessages[channelId] = cached
                        trimRecentMessages()
                    }
                }
                table.close()
            }
    }

    fun messages(channelId: String) = table("messages", channelId)

    fun refresh(reconnect: Boolean = false) {
        (listOf(channels, users, members, notificationSettings, notifications) + dynamic.values).forEach { it.refresh(reconnect) }
        if (reconnect && linked.isInitialized()) linked.value.refresh()
    }

    fun close() {
        if (linked.isInitialized()) linked.value.close()
        (listOf(channels, users, members, notificationSettings, notifications) + dynamic.values).forEach { it.close() }
        dynamic.clear()
        recentMessages.clear()
        http.close()
    }
}
