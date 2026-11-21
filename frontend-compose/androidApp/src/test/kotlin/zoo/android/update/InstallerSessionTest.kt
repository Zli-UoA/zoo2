package zoo.android.update

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.runTest

class InstallerSessionTest {
    @Test
    fun restartsExactlyOnceOnlyWhenPackageManagerConfirmsSuccess() {
        var relaunches = 0
        val restart = { relaunches++; Unit }
        assertContains(completeInstall(1, "Success", restart), "インストールできませんでした")
        assertContains(completeInstall(0, "Not a Success", restart), "インストールできませんでした")
        assertEquals(0, relaunches)
        assertEquals("Success", completeInstall(0, "Performing Streamed Install\nSuccess", restart))
        assertEquals(1, relaunches)
        assertContains(restartCommand(10).windowed(2), listOf("--user", "10"))
        assertEquals("works.zli.zoo/zoo.android.MainActivity", restartCommand(10).last())
    }

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
