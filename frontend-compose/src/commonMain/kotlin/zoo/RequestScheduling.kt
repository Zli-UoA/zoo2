package zoo

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*

/** One active request and one trailing invalidation, without restarting slow requests. */
internal class CoalescedRefresh(
    scope: CoroutineScope,
    private val interval: Long = 5000,
    private val allowed: () -> Boolean = { true },
    private val load: suspend () -> Unit,
) {
    private val pending = Channel<Unit>(Channel.CONFLATED)
    private val worker = scope.launch {
        for (event in pending) {
            if (allowed()) {
                try {
                    load()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    // Retry on the next invalidation.
                }
            }
            delay(interval)
        }
    }

    fun request() { pending.trySend(Unit) }
    fun close() { pending.close(); worker.cancel() }
}

/** Serial polling: idle work backs off; visibility and explicit changes wake it immediately. */
internal suspend fun pollWhileVisible(
    visibility: Flow<Boolean>,
    changes: Flow<Unit> = emptyFlow(),
    interval: Long,
    maximum: Long = interval,
    load: suspend () -> Boolean,
) = coroutineScope {
    val pending = Channel<Unit>(Channel.CONFLATED)
    val watcher = launch { changes.collect { pending.trySend(Unit) } }
    try {
        visibility.distinctUntilChanged().collectLatest { visible ->
            if (visible) {
                // The foreground load already includes changes queued while hidden.
                while (pending.tryReceive().isSuccess) { }
                var wait = interval
                while (isActive) {
                    val active = try {
                        withTimeout(10000) { load() }
                    } catch (_: TimeoutCancellationException) {
                        false
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        false
                    }
                    wait = if (active) interval else (wait * 2).coerceAtMost(maximum)
                    if (withTimeoutOrNull(wait) { pending.receive(); true } == true) {
                        wait = interval
                    }
                }
            }
        }
    } finally {
        watcher.cancel()
        pending.close()
    }
}
