package zoo.android.update

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class BinderReadinessTest {
    @Test
    fun waitsForBinderDeliveredAfterWorkerStarts() = runTest {
        var alive = false
        var listener: (() -> Unit)? = null
        var removed = false
        val result = async {
            awaitServiceBinder(
                isAlive = { alive },
                subscribe = { listener = it },
                unsubscribe = { removed = it === listener },
            )
        }
        runCurrent()
        assertFalse(result.isCompleted)
        alive = true
        requireNotNull(listener).invoke()
        assertTrue(result.await())
        assertTrue(removed)
    }

    @Test
    fun handlesBinderArrivingBetweenCheckAndSubscription() = runTest {
        var alive = false
        var removed = false
        assertTrue(
            awaitServiceBinder(
                isAlive = { alive },
                subscribe = { alive = true },
                unsubscribe = { removed = true },
            )
        )
        assertTrue(removed)
    }

    @Test
    fun unavailableServiceTimesOutAndRemovesListener() = runTest {
        var removed = false
        assertFalse(
            awaitServiceBinder(
                isAlive = { false },
                subscribe = {},
                unsubscribe = { removed = true },
            )
        )
        assertEquals(5_000L, currentTime)
        assertTrue(removed)
    }

    @Test
    fun cancellationRemovesListenerAndRemainsCancellation() = runTest {
        var removed = false
        val job = launch {
            awaitServiceBinder(
                isAlive = { false },
                subscribe = {},
                unsubscribe = { removed = true },
            )
        }
        runCurrent()
        job.cancelAndJoin()
        assertTrue(job.isCancelled)
        assertTrue(removed)
    }

    @Test
    fun livingBinderDoesNotSubscribeOrWait() = runTest {
        assertTrue(
            awaitServiceBinder(
                isAlive = { true },
                subscribe = { error("already connected") },
                unsubscribe = { error("not subscribed") },
            )
        )
        assertEquals(0L, currentTime)
    }
}
