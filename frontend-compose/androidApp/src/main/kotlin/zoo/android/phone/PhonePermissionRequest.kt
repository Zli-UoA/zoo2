package zoo.android.phone

import kotlinx.coroutines.CompletableDeferred

/** One system prompt can serve SIP registration and DM standby independently. */
internal class PhonePermissionRequest {
    private var pending: CompletableDeferred<Boolean>? = null

    fun complete(granted: Boolean) {
        val result = pending
        pending = null
        result?.complete(granted)
    }

    suspend fun await(launch: () -> Unit) {
        val existing = pending
        if (existing != null) {
            check(existing.await())
            return
        }
        val result = CompletableDeferred<Boolean>()
        pending = result
        try {
            launch()
        } catch (failure: Throwable) {
            if (pending === result) pending = null
            result.completeExceptionally(failure)
            throw failure
        }
        // A cancelled waiter must not detach the system callback from other waiters.
        check(result.await())
    }
}
