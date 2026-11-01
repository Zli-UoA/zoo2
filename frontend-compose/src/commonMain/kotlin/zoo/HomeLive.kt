package zoo

import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*

internal expect fun connectHomeSocket(
    url: String,
    auth: String,
    refresh: () -> Unit,
    closed: () -> Unit,
): () -> Unit

/** Authenticated Home invalidations shared by timelines, unread badges and notification screens. */
class HomeLive(private val app: Session) {
    private val listeners = linkedMapOf<Int, () -> Unit>()
    private var serial = 0
    private var connection: Job? = null
    private var pending: Job? = null
    private var attempts = 0

    fun subscribe(refresh: () -> Unit): () -> Unit {
        val id = ++serial
        listeners[id] = refresh
        if (connection == null) start()
        return {
            listeners.remove(id)
            if (listeners.isEmpty()) stop()
        }
    }

    private fun invalidate() {
        attempts = 0
        if (pending?.isActive == true) return
        pending =
            app.scope.launch {
                delay(100)
                pending = null
                listeners.values.toList().forEach { it() }
            }
    }

    private fun start() {
        connection =
            app.scope.launch {
                coroutineScope {
                    launch {
                        snapshotFlow { app.env.online }
                            .collectLatest { online ->
                                if (online) {
                                    while (isActive) {
                                        try {
                                            val auth =
                                                buildJsonObject {
                                                        app.platform.headers().forEach {
                                                            (key, value) ->
                                                            put(key, value)
                                                        }
                                                    }
                                                    .toString()
                                            val absolute =
                                                nativeAbsoluteUrl(
                                                    endpoint(
                                                        app.config.apiBaseUrl,
                                                        "/api/home/live",
                                                    )
                                                )
                                            val url =
                                                absolute
                                                    .replaceFirst(Regex("^https:"), "wss:")
                                                    .replaceFirst(Regex("^http:"), "ws:")
                                            val done = CompletableDeferred<Unit>()
                                            val dispose =
                                                connectHomeSocket(url, auth, ::invalidate) {
                                                    done.complete(Unit)
                                                }
                                            try {
                                                done.await()
                                            } finally {
                                                dispose()
                                            }
                                            delay(
                                                (1000L * (1L shl attempts.coerceAtMost(4)))
                                                    .coerceAtMost(15_000L)
                                            )
                                            attempts++
                                        } catch (e: CancellationException) {
                                            throw e
                                        } catch (_: Exception) {
                                            delay(3000)
                                        }
                                    }
                                }
                            }
                    }
                    launch {
                        snapshotFlow { app.env.visible to app.env.online }
                            .drop(1)
                            .collect { (visible, online) -> if (visible && online) invalidate() }
                    }
                }
            }
    }

    private fun stop() {
        connection?.cancel()
        connection = null
        pending?.cancel()
        pending = null
        attempts = 0
    }

    fun close() {
        stop()
        listeners.clear()
    }
}
