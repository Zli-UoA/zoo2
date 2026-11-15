package zoo.android.update

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.withTimeoutOrNull

/** Subscribe before waiting so a Binder delivered during startup cannot be missed. */
internal suspend fun awaitServiceBinder(
    isAlive: () -> Boolean,
    subscribe: (() -> Unit) -> Unit,
    unsubscribe: (() -> Unit) -> Unit,
    timeoutMillis: Long = 5_000,
): Boolean {
    if (isAlive()) return true
    val received = CompletableDeferred<Unit>()
    val listener: () -> Unit = { received.complete(Unit) }
    try {
        subscribe(listener)
        return withTimeoutOrNull(timeoutMillis) {
            // Also cover arrival between the initial check and registration.
            if (!isAlive()) received.await()
            isAlive()
        } ?: false
    } finally {
        unsubscribe(listener)
    }
}
