package zoo

import electric4k.LogMode
import electric4k.ShapeClient
import electric4k.ShapeRequest
import electric4k.ShapeStream
import electric4k.ShapeStreamOptions
import electric4k.SubsetParams
import io.ktor.client.HttpClient
import io.ktor.client.plugins.api.createClientPlugin
import io.ktor.client.plugins.timeout
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlin.time.TimeMark
import kotlin.time.TimeSource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.updateAndGet
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** 取得を始めてから、これだけたっても終わらないときは、読み込めなかったものとする（Web と同じ 10 秒）。 */
val LINK_LOAD_TIMEOUT: Duration = 10.seconds

/** 使う行がなくなってから、購読を閉じるまでの時間。一覧を少しスクロールして戻ったときに取り直さない。 */
val LINK_RELEASE_DELAY: Duration = 30.seconds

private const val LINK_CACHE_SIZE = 256

/** 1 件の取得で、続けて要求を送る回数。失敗が続くときは一度打ち切り、ほかの取得と live に順番を譲る。 */
private const val LINK_SUBSET_ATTEMPTS = 3

/** 取得の要求 1 回の応答を待つ時間。 */
private val LINK_SUBSET_TIMEOUT = 15.seconds

/** 打ち切った取得を、次に試すまでの間隔。打ち切るたびに倍にする。 */
private val LINK_SUBSET_RETRY = 1.seconds
private val LINK_SUBSET_MAX_RETRY = 30.seconds

/** 打ち切ったあと、再開した live が追いつくのを待つ時間の上限。 */
private val LINK_LIVE_WINDOW = 5.seconds

/** 待っても live が追いつかなかったあと、次に待つまでの間隔。 */
private val LINK_LIVE_RECHECK = 30.seconds

/** 失敗が続く取得の打ち切り。Electric クライアントが再試行しない例外にする。 */
private class SubsetYield : Exception("subset の取得を一度打ち切った")

/** 同時に購読するチャンネルの数の上限。backend のアカウントごとのシェイプの上限は、ほかの画面や端末と分け合う。 */
const val LINK_MAX_CHANNELS = 8

/**
 * 端末の暗号化チャンネルの記録の読み込み。状態と読み込みは、表示するすべてのリンクで共有する。始めてから [timeout]
 * （画面を離れた間も数える）で終わらないか、失敗したら、読めないものとする。次の [states] で読み直し、読めたら
 * 表示中のリンクもすべて切り替える。
 */
class EncryptionRecordsLoader(
    private val scope: CoroutineScope,
    private val isLoaded: () -> Boolean,
    private val load: suspend () -> Boolean,
    private val timeout: Duration = LINK_LOAD_TIMEOUT,
) {
    private val state = MutableStateFlow(EncryptionRecords.Loading)
    private var loading: Job? = null

    /** 読み込みを始めずに、いま分かっている状態を返す（最初の描画用）。 */
    fun current(): EncryptionRecords = if (isLoaded()) EncryptionRecords.Loaded else state.value

    /** 状態の変化を返し続ける。読み込んでいなければ読み込みを始める（失敗したあとは読み直す）。 */
    fun states(): Flow<EncryptionRecords> = flow {
        start()
        emitAll(state)
    }

    private fun start() {
        if (isLoaded()) {
            state.value = EncryptionRecords.Loaded
            return
        }
        if (loading?.isActive == true) return
        // 読み直す間は、読み込み中に戻さず、読めないものとして出したままにする。
        loading = scope.launch {
            val result = async { load() }
            if (withTimeoutOrNull(timeout) { result.await() } == null) state.value = EncryptionRecords.Unreadable
            state.value = if (result.await()) EncryptionRecords.Loaded else EncryptionRecords.Unreadable
        }
    }
}

/** リンク先のメッセージ 1 件の取得の状態。 */
sealed interface LinkedRow {
    data object Loading : LinkedRow

    data class Found(val row: Row) : LinkedRow

    /** 取得できたが、その ID の行がない。 */
    data object Missing : LinkedRow

    /** 再試行されない失敗（認証の期限切れなどの 4xx）。 */
    data object Failed : LinkedRow
}

data class LinkedMessageState(
    val row: LinkedRow = LinkedRow.Loading,
    /** この端末が読み込んでいる、同じチャンネルの行に暗号文があるか。 */
    val hasCiphertext: Boolean = false,
    /** 取得を始めてから [LINK_LOAD_TIMEOUT] たっても終わらない。 */
    val stalled: Boolean = false,
)

/** リンク先のメッセージを取る、1 チャンネル分の購読。 */
interface LinkedChannelSource {
    data class State(
        val rows: Map<String, Row> = emptyMap(),
        /** 取得が終わった ID。行がなければ、その ID のメッセージはない。 */
        val fetched: Set<String> = emptySet(),
        val failed: Set<String> = emptySet(),
        /** 購読そのものの失敗。 */
        val failure: Throwable? = null,
        /** 届いた行に暗号文があったか。[rows] に残さない行も含む。 */
        val hasCiphertext: Boolean = false,
    )

    val state: StateFlow<State>

    /** [messageId] の 1 件を取る。同じ ID の 2 回目以降は何もしない。 */
    fun request(messageId: String)

    fun refresh() {}

    fun close()
}

/**
 * `messages` Shape を `log=changes_only` で購読し、行は subset snapshot で 1 件ずつ取る（Web の on-demand の
 * `messages` と同じ経路・権限）。チャンネルの全件は同期せず、端末にも保存しない。取得した行の編集・削除は
 * live で届く。
 *
 * Electric クライアントは、取得を再試行している間、live の読み取りとほかの取得を待たせる。サーバーのエラーなどで
 * 失敗が続く取得は、[LINK_SUBSET_ATTEMPTS] 回で打ち切り、live が追いつくのを待ってから次の取得に進む（[awaitLive]）。
 * 打ち切った取得は、間隔を空けて取り直す。
 */
class LinkedChannelTable(
    channelId: String,
    config: Config,
    platform: Platform,
    http: HttpClient,
    scope: CoroutineScope,
    options: ShapeStreamOptions = ShapeStreamOptions(liveSse = true),
    private val clock: TimeSource = TimeSource.Monotonic,
) : LinkedChannelSource {
    private val tableScope =
        CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]))
    private val values = MutableStateFlow(LinkedChannelSource.State())
    override val state: StateFlow<LinkedChannelSource.State> = values.asStateFlow()
    private val requested = mutableSetOf<String>()
    /** 受け取った up-to-date の数。 */
    private val caughtUp = MutableStateFlow(0)
    /** 待っても live が追いつかなかった時刻。追いつけば消す。 */
    private var liveStuck: TimeMark? = null
    private var generation = 0
    private var refetching = false
    /** subset の取得は 1 件ずつ。[sent] は、いまの取得で送った要求の数。 */
    private val fetching = Mutex()
    private val subsetRequests = ShapeSubsetRequests()
    private val sent = MutableStateFlow(0)
    private val requests = http.config {
        install(createClientPlugin("ZooLinkedSubset") {
            onRequest { request, _ ->
                if (request.url.parameters.contains("subset__where")) {
                    if (sent.updateAndGet { it + 1 } > LINK_SUBSET_ATTEMPTS) throw SubsetYield()
                    request.timeout { requestTimeoutMillis = LINK_SUBSET_TIMEOUT.inWholeMilliseconds }
                }
            }
        })
    }

    private val direct = config.shapeMode == "direct"
    private val client = ShapeClient(
        if (direct) config.electricShapeUrl
        else endpoint(config.apiBaseUrl, "${config.shapeProxyPath}/messages"),
        httpClient = requests,
        headers = { if (direct) emptyMap() else platform.headers() },
        queryParams = {
            if (direct) mapOf("table" to "messages", "where" to "channel_id = \$1", "params[1]" to channelId)
            else mapOf("channel_id" to channelId)
        },
    )
    private val stream = ShapeStream(
        client, request = ShapeRequest(log = LogMode.ChangesOnly), scope = tableScope, options = options,
    )
    /** 取得を頼んだ行（シェイプのキーごと）。チャンネルのほかの行は持たない。 */
    private val kept = mutableMapOf<String, Row>()

    private val unsubscribe =
        stream.subscribe({ batch -> accept(batch) }) { failure -> values.update { it.copy(failure = failure) } }

    private fun accept(batch: JsonArray) {
        fun control(message: JsonElement) =
            message.jsonObject["headers"]?.jsonObject?.get("control")?.jsonPrimitive?.content
        if (batch.any { control(it) == "must-refetch" }) {
            kept.clear()
            generation++
            refetching = true
            values.value = LinkedChannelSource.State(hasCiphertext = values.value.hasCiphertext)
        }
        batch.forEach(::keep)
        // 保持しない行でも、暗号文が届いたことは覚える（暗号化の格下げの判定に使う）。
        val ciphertext = batch.any { (it.jsonObject["value"] as? JsonObject)?.present("ciphertext") == true }
        val upToDate = batch.any { control(it) == "up-to-date" }
        if (upToDate) {
            liveStuck = null
            caughtUp.update { it + 1 }
        }
        values.update { it.copy(rows = currentRows(), hasCiphertext = it.hasCiphertext || ciphertext) }
        if (upToDate && refetching) {
            refetching = false
            requested.toList().forEach(::fetch)
        }
    }

    /** 取得を頼んだ ID の行だけを反映する。`update` は、持っている行に変わった列を重ねる。 */
    private fun keep(message: JsonElement) {
        val fields = message.jsonObject
        val operation = fields["headers"]?.jsonObject?.get("operation")?.jsonPrimitive?.content ?: return
        val key = fields["key"]?.jsonPrimitive?.content ?: return
        val value = fields["value"] as? JsonObject
        when (operation) {
            "insert" -> if (value != null && value.text("id") in requested) kept[key] = value
            "update" -> kept[key]?.let { kept[key] = JsonObject(it + value.orEmpty()) }
            "delete" -> kept.remove(key)
        }
    }

    private fun currentRows(): Map<String, Row> = kept.values.associateBy { it.text("id") }

    override fun request(messageId: String) {
        if (!requested.add(messageId)) return
        // 取り直しの最中は、up-to-date のあとでまとめて取る。
        if (!refetching) fetch(messageId)
    }

    private fun fetch(messageId: String) {
        val startedAt = generation
        tableScope.launch {
            try {
                // Electric can suppress a replayed up-to-date message while updating the stream status.
                combine(stream.status, values) { status, current ->
                    status.isUpToDate || current.failure != null
                }.first { it }
                var wait = LINK_SUBSET_RETRY
                // 取り直し（must-refetch）になったら、そちらの取得に任せる。
                while (startedAt == generation && values.value.failure == null) {
                    val done = fetching.withLock {
                        val before = caughtUp.value
                        try {
                            subsetRequests.request {
                                sent.value = 0
                                stream.requestSnapshot(
                                    SubsetParams(where = "id = \$1", params = mapOf("1" to JsonPrimitive(messageId)))
                                )
                            }
                            true
                        } catch (_: SubsetYield) {
                            awaitLive(before)
                            false
                        }
                    }
                    if (done) {
                        if (startedAt == generation) {
                            values.update { it.copy(rows = currentRows(), fetched = it.fetched + messageId) }
                        }
                        return@launch
                    }
                    delay(wait)
                    wait = minOf(wait * 2, LINK_SUBSET_MAX_RETRY)
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                if (startedAt == generation) values.update { it.copy(failed = it.failed + messageId) }
            }
        }
    }

    /**
     * 打ち切ったあと、次の取得が live をすぐにまた止めないように、up-to-date が [before] 件より増えるのを待つ。
     * 待っても追いつかなければ（サーバーの障害や接続の断）、[LINK_LIVE_RECHECK] の間は待たずに次の取得に進む。
     */
    private suspend fun awaitLive(before: Int) {
        if (liveStuck?.let { it.elapsedNow() < LINK_LIVE_RECHECK } == true) return
        if (withTimeoutOrNull(LINK_LIVE_WINDOW) { caughtUp.first { it > before } } == null) {
            liveStuck = clock.markNow()
        }
    }

    private var refreshJob: Job? = null

    override fun refresh() {
        refreshJob?.cancel()
        refreshJob = tableScope.launch {
            if (subsetRequests.reconnect()) return@launch
            if (stream.isPaused() || stream.error != null) return@launch
            try {
                stream.forceDisconnectAndRefresh()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // The subscriber reports stream failures. A subset fetch may also pause live.
            }
        }
    }

    override fun close() {
        unsubscribe()
        stream.close()
        client.close()
        requests.close()
        tableScope.cancel()
    }
}

/**
 * 本文のリンクが指すメッセージの取得（Web の `useLinkedMessageRows`）。
 *
 * - 開いているチャンネルの同期済みの行にあれば、それを使い、要求を出さない。
 * - ないものは、チャンネルごとに 1 本の購読を共有し、同じメッセージは 1 回だけ取る。
 * - 使う行（[observe] の収集）がなくなって [releaseAfter] たったら閉じる。失敗した購読は、閉じたあとの
 *   [observe] で作り直す。
 * - 同時に購読するチャンネルは [maxChannels] まで。超えるときは使う行がない購読を先に閉じ、なければ
 *   読み込めなかったものとして、空くのを待つ。
 * - 暗号文の行を一度でも見たチャンネルは、購読を閉じたあとも覚える。覚えた結果の平文を出さないようにする。
 * - メッセージの ID の形でない値（[fetchable]）は取得せず、ないものとする。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LinkedMessages(
    private val scope: CoroutineScope,
    private val openRows: (channelId: String) -> StateFlow<List<Row>>?,
    private val openSource: (channelId: String) -> LinkedChannelSource,
    private val stallAfter: Duration = LINK_LOAD_TIMEOUT,
    private val releaseAfter: Duration = LINK_RELEASE_DELAY,
    private val clock: TimeSource = TimeSource.Monotonic,
    private val maxChannels: Int = LINK_MAX_CHANNELS,
    private val fetchable: (messageId: String) -> Boolean = ::isMessageId,
) {
    private class Entry(val source: LinkedChannelSource) {
        var users = 0
        var release: Job? = null
        /** 使う行がなくなった時刻。 */
        var idleSince: TimeMark? = null
    }

    private data class Key(val channelId: String, val messageId: String)

    private val entries = mutableMapOf<String, Entry>()
    /** 取得できた結果。画面に戻ったときに、枠を経由せず表示する。 */
    private val results = LinkedHashMap<Key, LinkedRow>()
    private val started = mutableMapOf<Key, TimeMark>()
    /** 暗号文の行を見たチャンネル。増えたら、表示中のリンクも判定し直す。 */
    private val ciphertextChannels = MutableStateFlow(emptySet<String>())
    /** 使う行がない購読ができるたびに進め、空きを待つ取得に知らせる。 */
    private val vacancies = MutableStateFlow(0)

    /** 要求を出さずに、いま分かっている状態を返す（最初の描画用）。 */
    fun peek(channelId: String, messageId: String): LinkedMessageState {
        if (!fetchable(messageId)) return LinkedMessageState(LinkedRow.Missing)
        val rows = openRows(channelId)?.value
        val source = entries[channelId]?.source?.state?.value
        val ciphertext = sawCiphertext(channelId, rows, source)
        val open = rows?.firstOrNull { it.text("id") == messageId }
        if (open != null) return LinkedMessageState(LinkedRow.Found(open), ciphertext)
        // 拒否された取得は、画面に戻っても読み込み中に戻さない。
        if (source?.rejects(messageId) == true) return LinkedMessageState(LinkedRow.Failed, ciphertext)
        val key = Key(channelId, messageId)
        // 購読の上限で取得できないリンクも、読み込み中に戻さない。
        val row = results[key] ?: if (isFull(channelId)) LinkedRow.Failed else LinkedRow.Loading
        val stalled = row == LinkedRow.Loading && started[key]?.let { it.elapsedNow() >= stallAfter } == true
        return LinkedMessageState(row, ciphertext, stalled)
    }

    fun observe(channelId: String, messageId: String): Flow<LinkedMessageState> {
        if (!fetchable(messageId)) return flowOf(LinkedMessageState(LinkedRow.Missing))
        // 暗号文は、同じチャンネルのほかのリンクの取得で見つかることもある。
        return combine(rowState(channelId, messageId), ciphertextChannels) { state, seen ->
            if (channelId in seen) state.copy(hasCiphertext = true) else state
        }.distinctUntilChanged()
    }

    private fun rowState(channelId: String, messageId: String): Flow<LinkedMessageState> {
        val loaded = openRows(channelId) ?: return fetched(channelId, messageId)
        return loaded
            .map { rows -> rows.firstOrNull { it.text("id") == messageId } to sawCiphertext(channelId, rows) }
            .distinctUntilChanged()
            .flatMapLatest { (row, ciphertext) ->
                if (row != null) flowOf(LinkedMessageState(LinkedRow.Found(row), ciphertext))
                else fetched(channelId, messageId).map { it.copy(hasCiphertext = it.hasCiphertext || ciphertext) }
            }
    }

    private fun fetched(channelId: String, messageId: String): Flow<LinkedMessageState> = flow {
        val key = Key(channelId, messageId)
        var acquired = acquire(channelId)
        while (acquired == null) {
            val seen = vacancies.value
            emit(LinkedMessageState(results[key] ?: LinkedRow.Failed, sawCiphertext(channelId)))
            vacancies.first { it != seen }
            acquired = acquire(channelId)
        }
        val entry = acquired
        try {
            val since = started.getOrPut(key) { clock.markNow() }
            entry.source.request(messageId)
            val stalled = flow {
                val remaining = stallAfter - since.elapsedNow()
                if (remaining.isPositive()) {
                    emit(false)
                    delay(remaining)
                }
                emit(true)
            }
            emitAll(combine(entry.source.state, stalled) { state, late -> result(key, state, late) }
                .distinctUntilChanged())
        } finally {
            release(channelId, entry)
        }
    }

    private fun result(key: Key, state: LinkedChannelSource.State, late: Boolean): LinkedMessageState {
        val row = state.rows[key.messageId]
        // 再試行されない失敗（認証や権限の 4xx）は、その前に取得できた行があっても出さない。
        val current = when {
            state.rejects(key.messageId) -> LinkedRow.Failed
            row != null -> LinkedRow.Found(row)
            key.messageId in state.fetched -> LinkedRow.Missing
            else -> null
        }
        if (current is LinkedRow.Found || current == LinkedRow.Missing) {
            started.remove(key)
            remember(key, current!!)
        }
        discardRejected(key.channelId, state)
        // 取り直しや再試行の間は、前に取得できた結果を出す。
        val shown = current ?: results[key]
        return LinkedMessageState(
            shown ?: LinkedRow.Loading,
            sawCiphertext(key.channelId, source = state),
            stalled = late && current == null,
        )
    }

    /** 購読そのものか、その ID の取得が、再試行されない失敗になったか。 */
    private fun LinkedChannelSource.State.rejects(messageId: String): Boolean =
        failure != null || messageId in failed

    /** 拒否された取得の、前の結果を捨てる。購読そのものの失敗では、そのチャンネルのすべてを捨てる。 */
    private fun discardRejected(channelId: String, state: LinkedChannelSource.State) {
        results.keys.removeAll { it.channelId == channelId && state.rejects(it.messageId) }
    }

    private fun remember(key: Key, row: LinkedRow) {
        results.remove(key)
        results[key] = row
        while (results.size > LINK_CACHE_SIZE) results.remove(results.keys.first())
    }

    /** [channelId] で暗号文の行を見たことがあるか。[rows] と購読の状態に暗号文があれば覚える。 */
    private fun sawCiphertext(
        channelId: String, rows: Collection<Row>? = null, source: LinkedChannelSource.State? = null,
    ): Boolean {
        fun encrypted(candidates: Collection<Row>?) = candidates?.any { it.present("ciphertext") } == true
        if (source?.hasCiphertext == true || encrypted(rows) || encrypted(source?.rows?.values)) {
            ciphertextChannels.update { it + channelId }
        }
        return channelId in ciphertextChannels.value
    }

    /** [channelId] の購読がなく、上限まで使っていて空けられないか。 */
    private fun isFull(channelId: String): Boolean =
        channelId !in entries && entries.size >= maxChannels && entries.values.none { it.users == 0 }

    /** 購読を使い始める。上限まで使っていて空けられなければ `null`。 */
    private fun acquire(channelId: String): Entry? {
        val entry = entries[channelId] ?: run {
            if (entries.size >= maxChannels && !closeIdle()) return null
            Entry(openSource(channelId)).also { entries[channelId] = it }
        }
        entry.release?.cancel()
        entry.release = null
        entry.idleSince = null
        entry.users++
        return entry
    }

    private fun release(channelId: String, entry: Entry) {
        if (--entry.users > 0 || entries[channelId] !== entry) return
        entry.idleSince = clock.markNow()
        entry.release = scope.launch {
            delay(releaseAfter)
            if (entry.users == 0 && entries[channelId] === entry) remove(channelId, entry)
        }
        vacancies.update { it + 1 }
    }

    /** 使う行がない購読のうち、いちばん前に使わなくなったものを閉じる。なければ `false`。 */
    private fun closeIdle(): Boolean {
        val idle = entries.entries.filter { it.value.users == 0 }
            .maxByOrNull { it.value.idleSince?.elapsedNow() ?: Duration.ZERO } ?: return false
        idle.value.release?.cancel()
        remove(idle.key, idle.value)
        return true
    }

    private fun remove(channelId: String, entry: Entry) {
        entries.remove(channelId)
        // 閉じたあとの取得は、改めて時間を数える。
        started.keys.removeAll { it.channelId == channelId }
        sawCiphertext(channelId, source = entry.source.state.value)
        discardRejected(channelId, entry.source.state.value)
        entry.source.close()
    }

    fun refresh() {
        entries.values.toList().forEach { it.source.refresh() }
    }

    fun close() {
        val open = entries.values.toList()
        entries.clear()
        results.clear()
        started.clear()
        ciphertextChannels.value = emptySet()
        open.forEach {
            it.release?.cancel()
            it.source.close()
        }
    }
}
