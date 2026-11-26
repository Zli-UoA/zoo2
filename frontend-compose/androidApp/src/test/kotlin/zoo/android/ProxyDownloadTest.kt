package zoo.android

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.test.*

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class ProxyDownloadTest {
    @Test fun retriesTransientResponsesOnlyOnce() = runTest {
        for (status in listOf(429, 503, 404)) {
            var attempts = 0
            val failure = assertFailsWith<ImageHttpFailure> {
                downloadProxyImage(push = false) { attempts++; throw ImageHttpFailure(status) }
            }
            assertEquals(status, failure.status)
            assertEquals(if (status == 404) 1 else 2, attempts)
        }
    }

    @Test fun foregroundIsLimitedAndPushDoesNotWaitForAPermit() = runTest {
        val release = CompletableDeferred<Unit>()
        var started = 0
        repeat(3) { launch { downloadProxyImage(push = false) { started++; release.await() } } }
        runCurrent()
        assertEquals(2, started)
        var pushStarted = false
        assertFailsWith<IllegalStateException> { downloadProxyImage(push = true) { pushStarted = true } }
        assertFalse(pushStarted)
        release.complete(Unit)
        runCurrent()
        assertEquals(3, started)
    }
}
