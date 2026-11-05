package zoo.android.update

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.runTest

class InstallerSessionTest {
    @Test
    fun preservesResultAcrossSuspendingCleanup() = runTest {
        var disconnected = false
        val result =
            installerSession(
                connect = {
                    delay(1)
                    Unit
                },
                install = {
                    delay(1)
                    "Success"
                },
                disconnect = {
                    delay(1)
                    disconnected = true
                },
            )
        assertEquals("Success", result)
        assertTrue(disconnected)
    }

    @Test
    fun rejectsMissingBinderResponseAndStillDisconnects() = runTest {
        for (response in listOf(null, "", " ")) {
            var disconnected = false
            val failure =
                assertFailsWith<IllegalStateException> {
                    installerSession(
                        connect = { Unit },
                        install = { response },
                        disconnect = {
                            delay(1)
                            disconnected = true
                        },
                    )
                }
            assertContains(failure.message.orEmpty(), "Shizuku")
            assertTrue(disconnected)
        }
    }

    @Test
    fun cancellationWaitsForCleanup() = runTest {
        val started = CompletableDeferred<Unit>()
        var disconnected = false
        val job = launch {
            installerSession(
                connect = { Unit },
                install = {
                    started.complete(Unit)
                    awaitCancellation()
                },
                disconnect = {
                    delay(1)
                    disconnected = true
                },
            )
        }
        started.await()
        job.cancelAndJoin()
        assertTrue(disconnected)
        assertTrue(job.isCancelled)
    }

    @Test
    fun cleanupFailureDoesNotHideInstallerFailure() = runTest {
        val failure = IllegalStateException("installer failed")
        assertSame(
            failure,
            assertFailsWith<IllegalStateException> {
                installerSession<Unit>(
                    connect = { throw failure },
                    install = { "Success" },
                    disconnect = { error("cleanup failed") },
                )
            },
        )
    }
}
