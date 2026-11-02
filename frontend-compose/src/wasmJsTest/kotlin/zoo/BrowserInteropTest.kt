@file:OptIn(
    kotlin.js.ExperimentalWasmJsInterop::class,
    kotlinx.coroutines.ExperimentalCoroutinesApi::class,
)

package zoo

import kotlin.JsFun
import kotlin.js.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@JsFun("() => Promise.reject(new Error('public transport offline'))")
private external fun rejectedRequest(): Promise<JsString>

@JsFun("() => new Promise(() => {})") private external fun pendingRequest(): Promise<JsString>

class BrowserInteropTest {
    @Test
    fun rejectedBrowserRequestsReachFeatureExceptionHandlers() = runTest {
        val failure = assertFailsWith<IllegalStateException> { rejectedRequest().awaitBrowser() }
        assertEquals("public transport offline", failure.message)
        assertIs<JsException>(failure.cause)
    }

    @Test
    fun cancelledBrowserRequestsRemainCancellable() = runTest {
        val job = launch { pendingRequest().awaitBrowser() }
        runCurrent()
        job.cancelAndJoin()
        assertTrue(job.isCancelled)
    }
}
