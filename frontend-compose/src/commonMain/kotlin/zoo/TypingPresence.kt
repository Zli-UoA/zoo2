package zoo

import io.ktor.http.encodeURLParameter
import kotlin.time.TimeMark
import kotlin.time.TimeSource
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.selects.onTimeout
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

/**
 * 会話・スレッドの「入力中」（Web の `useTyping`、`/api/channels/{id}/typing`）。下書きの本文は送らない。
 *
 * - 入力のたびに、2 秒に 1 回まで `active: true` を送る。最後の入力から 3 秒で、または本文が空になると止める。
 * - 他の人の入力中を 1.5 秒ごとに取得する。誰も入力していない間は、5 秒まで間隔を延ばす。
 * - [run] を取り消す（画面を離れる、非表示、オフライン）と取得をやめ、入力中だったら止める通知を送る。
 * - 書き込みは順に送り、遅れた開始が停止を追い越さないようにする。
 */
class TypingPresence(
    private val request: suspend (path: String, method: String, body: JsonElement?) -> JsonElement,
    private val sessionId: String,
    private val time: TimeSource = TimeSource.Monotonic,
) {
    private val current = MutableStateFlow<List<Row>>(emptyList())

    /** 入力中の他のユーザー（`id`・`display_name`）。 */
    val users: StateFlow<List<Row>> get() = current

    suspend fun run(channel: String, parent: String?, typed: Flow<Boolean>): Unit = coroutineScope {
        val path = "/api/channels/${channel.encodeURLParameter()}/typing"
        val query = parent?.let { "?parent_id=${it.encodeURLParameter()}" }.orEmpty()
        val writes = Mutex()
        val wake = Channel<Unit>(Channel.CONFLATED)
        var active = false
        var lastSent: TimeMark? = null
        var stopTimer: Job? = null

        var announced = false

        suspend fun write(value: Boolean) {
            request(path, "POST", buildJsonObject {
                put("parent_id", parent?.let(::JsonPrimitive) ?: JsonNull)
                put("session_id", JsonPrimitive(sessionId))
                put("active", JsonPrimitive(value))
            })
            announced = value
        }

        // 開始の通知は、送信待ちを 1 件にまとめる。応答が遅くても待ちが増えず、送る直前に間隔を確かめる。
        var startQueued = false

        suspend fun send(value: Boolean) = writes.withLock {
            if (value) {
                startQueued = false
                if (!active) return@withLock
                if (lastSent.let { it != null && it.elapsedNow().inWholeMilliseconds < SEND_INTERVAL_MS }) return@withLock
                lastSent = time.markNow()
            }
            try {
                withTimeout(WRITE_TIMEOUT_MS) { write(value) }
            } catch (failure: CancellationException) {
                if (failure is TimeoutCancellationException) return@withLock
                throw failure
            } catch (_: Exception) {
                // 入力中は短い有効期限（6 秒）で消えるので、失敗してもやり直さない。
            }
        }

        fun stop() {
            stopTimer?.cancel()
            if (active) launch { send(false) }
            active = false
            lastSent = null
        }

        launch {
            typed.collect { typing ->
                if (!typing) { stop(); return@collect }
                val started = !active
                active = true
                if (started) wake.trySend(Unit)
                if (!startQueued && lastSent.let { it == null || it.elapsedNow().inWholeMilliseconds >= SEND_INTERVAL_MS }) {
                    startQueued = true
                    launch { send(true) }
                }
                stopTimer?.cancel()
                stopTimer = launch {
                    delay(STOP_AFTER_MS)
                    stop()
                }
            }
        }

        try {
            var interval = POLL_MIN_MS
            while (isActive) {
                val found = try {
                    withTimeout(WRITE_TIMEOUT_MS) { request(path + query, "GET", null) }.rows()
                } catch (failure: CancellationException) {
                    if (failure !is TimeoutCancellationException) throw failure
                    emptyList()
                } catch (_: Exception) {
                    emptyList()
                }
                current.value = found
                interval = if (found.isNotEmpty() || active) POLL_MIN_MS else (interval * 2).coerceAtMost(POLL_MAX_MS)
                select {
                    wake.onReceive { }
                    onTimeout(interval) { }
                }
            }
        } finally {
            current.value = emptyList()
            stopTimer?.cancel()
            active = false
            // 止める通知が取り消されていても、開始を送っていれば止める。
            withContext(NonCancellable) {
                withTimeoutOrNull(WRITE_TIMEOUT_MS) {
                    writes.withLock {
                        if (announced) try { write(false) } catch (_: Exception) { }
                    }
                }
            }
        }
    }

    companion object {
        const val SEND_INTERVAL_MS = 2_000L
        const val STOP_AFTER_MS = 3_000L
        const val POLL_MIN_MS = 1_500L
        const val POLL_MAX_MS = 5_000L
        const val WRITE_TIMEOUT_MS = 5_000L

        /** 「○○、△△が入力中…」の名前の部分。 */
        fun names(users: List<Row>, fallback: (String) -> String): List<String> =
            users.map { row -> row.text("display_name").ifBlank { fallback(row.text("id")) } }
    }
}
