package zoo

import electric4k.SubsetSnapshot
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.async
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.supervisorScope

/**
 * Resume pending subset reads on the new connection. The owning coroutine keeps its page
 * or link request and its cancellation still stops the read.
 */
internal class ShapeSubsetRequests {
    private class Reconnect : CancellationException("Shape connection changed")

    private val active = MutableStateFlow<Set<Deferred<SubsetSnapshot>>>(emptySet())

    fun reconnect(): Boolean {
        val pending = active.value.filterNot { it.isCompleted }
        pending.forEach { it.cancel(Reconnect()) }
        return pending.isNotEmpty()
    }

    suspend fun request(fetch: suspend () -> SubsetSnapshot): SubsetSnapshot = supervisorScope {
        var snapshot: SubsetSnapshot? = null
        while (snapshot == null) {
            val attempt = async(start = CoroutineStart.LAZY) { fetch() }
            active.update { it + attempt }
            try {
                snapshot = attempt.await()
            } catch (_: Reconnect) {
                ensureActive()
            } finally {
                active.update { it - attempt }
            }
        }
        snapshot
    }
}
