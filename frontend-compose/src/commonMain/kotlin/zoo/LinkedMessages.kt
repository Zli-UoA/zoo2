package zoo

import electric4k.LogMode
import electric4k.ShapeClient
import electric4k.ShapeRequest
import electric4k.ShapeStream
import electric4k.ShapeStreamOptions
import electric4k.SubsetParams
import io.ktor.client.HttpClient
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlin.time.TimeMark
import kotlin.time.TimeSource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
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
import kotlinx.coroutines.launch
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

/** 同時に購読するチャンネルの数の上限。backend のアカウントごとのシェイプの上限は、ほかの画面や端末と分け合う。 */
const val LINK_MAX_CHANNELS = 8

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
    )

    val state: StateFlow<State>

    /** [messageId] の 1 件を取る。同じ ID の 2 回目以降は何もしない。 */
    fun request(messageId: String)

    fun close()
}

/**
 * `messages` Shape を `log=changes_only` で購読し、行は subset snapshot で 1 件ずつ取る（Web の on-demand の
 * `messages` と同じ経路・権限）。チャンネルの全件は同期せず、端末にも保存しない。取得した行の編集・削除は
 * live で届く。
 */
class LinkedChannelTable(
    channelId: String,
    config: Config,
    platform: Platform,
    http: HttpClient,
    scope: CoroutineScope,
    options: ShapeStreamOptions = ShapeStreamOptions(liveSse = true),
) : LinkedChannelSource {
    private val tableScope =
        CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]))
    private val values = MutableStateFlow(LinkedChannelSource.State())
    override val state: StateFlow<LinkedChannelSource.State> = values.asStateFlow()
    private val requested = mutableSetOf<String>()
    /** 最初の up-to-date（`changes_only` では空）を受け取ったか。subset はそのあとに取る。 */
    private val synced = MutableStateFlow(false)
    private var generation = 0
    private var refetching = false

    private val direct = config.shapeMode == "direct"
    private val client = ShapeClient(
        if (direct) config.electricShapeUrl
        else endpoint(config.apiBaseUrl, "${config.shapeProxyPath}/messages"),
        httpClient = http,
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
            synced.value = false
            values.value = LinkedChannelSource.State()
        }
        batch.forEach(::keep)
        val upToDate = batch.any { control(it) == "up-to-date" }
        if (upToDate) synced.value = true
        values.update { it.copy(rows = currentRows()) }
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
                combine(synced, values) { ready, current -> ready || current.failure != null }.first { it }
                if (values.value.failure != null) return@launch
                stream.requestSnapshot(
                    SubsetParams(where = "id = \$1", params = mapOf("1" to JsonPrimitive(messageId)))
                )
                if (startedAt == generation) {
                    values.update { it.copy(rows = currentRows(), fetched = it.fetched + messageId) }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                if (startedAt == generation) values.update { it.copy(failed = it.failed + messageId) }
            }
        }
    }

    override fun close() {
        unsubscribe()
        stream.close()
        client.close()
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
    /** 使う行がない購読ができるたびに進め、空きを待つ取得に知らせる。 */
    private val vacancies = MutableStateFlow(0)

    /** 要求を出さずに、いま分かっている状態を返す（最初の描画用）。 */
    fun peek(channelId: String, messageId: String): LinkedMessageState {
        val rows = openRows(channelId)?.value
        val open = rows?.firstOrNull { it.text("id") == messageId }
        return if (open != null) LinkedMessageState(LinkedRow.Found(open), hasCiphertext(rows))
        else LinkedMessageState(
            results[Key(channelId, messageId)] ?: LinkedRow.Loading,
            hasCiphertext(rows) || hasCiphertext(entries[channelId]?.source?.state?.value?.rows?.values),
        )
    }

    fun observe(channelId: String, messageId: String): Flow<LinkedMessageState> {
        val loaded = openRows(channelId) ?: return fetched(channelId, messageId)
        return loaded
            .map { rows -> rows.firstOrNull { it.text("id") == messageId } to hasCiphertext(rows) }
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
            emit(LinkedMessageState(results[key] ?: LinkedRow.Failed))
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
        val current = when {
            row != null -> LinkedRow.Found(row)
            key.messageId in state.fetched -> LinkedRow.Missing
            key.messageId in state.failed || state.failure != null -> LinkedRow.Failed
            else -> null
        }
        if (current is LinkedRow.Found || current == LinkedRow.Missing) {
            started.remove(key)
            remember(key, current!!)
        }
        // 取り直しに失敗したときは、前に取得できた結果を残す。
        val previous = results[key]
        val shown = if (current == null || current == LinkedRow.Failed) previous ?: current else current
        return LinkedMessageState(
            shown ?: LinkedRow.Loading,
            hasCiphertext(state.rows.values),
            stalled = late && current == null,
        )
    }

    private fun remember(key: Key, row: LinkedRow) {
        results.remove(key)
        results[key] = row
        while (results.size > LINK_CACHE_SIZE) results.remove(results.keys.first())
    }

    private fun hasCiphertext(rows: Collection<Row>?): Boolean = rows?.any { it.present("ciphertext") } == true

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
        entry.source.close()
    }

    fun close() {
        val open = entries.values.toList()
        entries.clear()
        results.clear()
        started.clear()
        open.forEach {
            it.release?.cancel()
            it.source.close()
        }
    }
}
