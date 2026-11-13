@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.test.*

class RequestSchedulingTest {
    @Test fun slowRefreshesAreSerialAndInvalidationsConflate() = runTest {
        var calls = 0
        val finish = CompletableDeferred<Unit>()
        val updates = CoalescedRefresh(backgroundScope) { calls++; if (calls == 1) finish.await() }
        updates.request(); runCurrent()
        repeat(20) { updates.request() }
        advanceTimeBy(10000); runCurrent()
        assertEquals(1, calls)
        finish.complete(Unit); runCurrent()
        advanceTimeBy(5000); runCurrent()
        assertEquals(2, calls)
        updates.close()
        advanceTimeBy(10000); runCurrent()
        assertEquals(2, calls)
    }

    @Test fun idlePollingBacksOffAndForegroundingWakesIt() = runTest {
        val visible = MutableStateFlow(true)
        val changes = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
        var calls = 0
        var active = false
        val job = backgroundScope.launch {
            pollWhileVisible(visible, changes, 1000, 4000) { calls++; active }
        }
        runCurrent(); assertEquals(1, calls)
        advanceTimeBy(2000); runCurrent(); assertEquals(2, calls)
        advanceTimeBy(4000); runCurrent(); assertEquals(3, calls)
        visible.value = false; runCurrent()
        advanceTimeBy(30000); runCurrent(); assertEquals(3, calls)
        visible.value = true; runCurrent(); assertEquals(4, calls)
        active = true
        changes.emit(Unit); runCurrent(); assertEquals(5, calls)
        advanceTimeBy(1000); runCurrent(); assertEquals(6, calls)
        job.cancel()
    }
    @Test fun hidingCancelsSlowPollingAndQueuedChangesDoNotOverlapIt() = runTest {
        val visible = MutableStateFlow(true)
        val changes = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
        val finish = CompletableDeferred<Unit>()
        var calls = 0
        var running = 0
        var maximum = 0
        val job = backgroundScope.launch {
            pollWhileVisible(visible, changes, 1000, 4000) {
                calls++; running++; maximum = maxOf(maximum, running)
                try { finish.await(); false } finally { running-- }
            }
        }
        runCurrent()
        repeat(20) { changes.emit(Unit); runCurrent() }
        assertEquals(1, calls)
        visible.value = false; runCurrent()
        assertEquals(0, running)
        visible.value = true; runCurrent()
        assertEquals(2, calls)
        finish.complete(Unit); runCurrent()
        assertEquals(2, calls)
        assertEquals(1, maximum)
        job.cancel()
    }

}
