package zoo

import kotlin.time.TimeSource
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

fun clampBubbleContent(value: String): String = buildString {
    var index = 0
    var count = 0
    while (index < value.length && count++ < 1000) {
        val ch = value[index++]
        append(ch)
        if (ch.isHighSurrogate() && index < value.length && value[index].isLowSurrogate())
            append(value[index++])
    }
}

fun shouldClearBubble(
    key: String,
    ctrl: Boolean,
    meta: Boolean,
    shift: Boolean,
    alt: Boolean,
    composing: Boolean,
) = key == "Enter" && (ctrl || meta) && !shift && !alt && !composing

/** A single serialized writer owns seq and retries the latest value, never an old draft. */
class BubbleController(private val app: Session) {
    val input = MutableStateFlow("")
    val entries = MutableStateFlow<List<Row>>(emptyList())
    val sharing = MutableStateFlow(false)
    private val session = app.platform.uuid()
    private val mutex = Mutex()
    private var seq = 0
    private var offset = 0L
    private var edited = TimeSource.Monotonic.markNow()
    private var revision = 0
    private var stoppedRevision = -1

    fun edit(value: String) {
        input.value = clampBubbleContent(value)
        edited = TimeSource.Monotonic.markNow()
        revision++
    }

    private suspend fun write(value: String): Boolean =
        mutex.withLock {
            try {
                val body = buildJsonObject {
                    put("session_id", session)
                    put("seq", ++seq)
                    put("content", value)
                    put("sent_at", (timestampMillis(app.platform.now()) ?: 0L) + offset)
                }
                val exit = buildJsonObject {
                    put("session", session)
                    put("offset", offset)
                    put(
                        "body",
                        buildJsonObject {
                            put("session_id", session)
                            put("seq", seq + 1)
                            put("content", "")
                        },
                    )
                    put(
                        "headers",
                        JsonObject(app.platform.headers().mapValues { JsonPrimitive(it.value) }),
                    )
                }
                app.platform.invoke("bubbleExit", exit.toString())
                val response =
                    withTimeout(5000) { app.api.request("/api/bubble", "POST", body).jsonObject }
                response["server_time"]?.jsonPrimitive?.longOrNull?.let {
                    offset = it - (timestampMillis(app.platform.now()) ?: 0L)
                }
                response.flag("ok") && !response.flag("stale")
            } catch (e: TimeoutCancellationException) {
                false
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                (e as? ApiException)?.body?.get("server_time")?.jsonPrimitive?.longOrNull?.let {
                    offset = it - (timestampMillis(app.platform.now()) ?: 0L)
                }
                false
            }
        }

    suspend fun run(): Unit = coroutineScope {
        try {
            launch {
                var confirmed: String? = null
                var seenRevision = revision
                var idle = false
                while (isActive) {
                    if (seenRevision != revision) {
                        seenRevision = revision
                        idle = false
                    }
                    if (edited.elapsedNow().inWholeMilliseconds >= 15000) idle = true
                    val value =
                        if (idle || revision == stoppedRevision) ""
                        else input.value.takeIf { it.isNotBlank() }.orEmpty()
                    if (value != confirmed) {
                        val sentRevision = revision
                        if (write(value)) {
                            confirmed = value
                            sharing.value = value.isNotEmpty() && sentRevision == revision
                        } else {
                            sharing.value = false
                            delay(1000)
                        }
                    }
                    delay(200)
                }
            }
            var succeeded = TimeSource.Monotonic.markNow()
            while (isActive) {
                try {
                    entries.value = withTimeout(5000) { app.api.request("/api/bubble").rows() }
                    succeeded = TimeSource.Monotonic.markNow()
                } catch (e: TimeoutCancellationException) {
                    if (succeeded.elapsedNow().inWholeMilliseconds >= 15000)
                        entries.value = emptyList()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (succeeded.elapsedNow().inWholeMilliseconds >= 15000)
                        entries.value = emptyList()
                }
                delay(1000)
            }
        } finally {
            stoppedRevision = revision
            sharing.value = false
            entries.value = emptyList()
            withContext(NonCancellable) {
                withTimeoutOrNull(5000) {
                    if (write(""))
                        app.platform.invoke(
                            "bubbleExit",
                            obj("session" to session, "clear" to "true").toString(),
                        )
                }
            }
        }
    }
}
