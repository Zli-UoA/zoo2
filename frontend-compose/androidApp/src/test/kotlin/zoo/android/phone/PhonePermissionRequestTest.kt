package zoo.android.phone

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class PhonePermissionRequestTest {
    @Test fun cancellingSipDoesNotOrphanTheDmWaiter() = runTest {
        val permission = PhonePermissionRequest()
        var prompts = 0
        val sip = launch { permission.await { prompts++ } }
        runCurrent()
        val dm = async { permission.await { prompts++ } }
        runCurrent()
        sip.cancelAndJoin()
        permission.complete(true)
        dm.await()
        assertEquals(1, prompts)
    }

    @Test fun denialCompletesWaitersAndTheNextRequestCanRetry() = runTest {
        val permission = PhonePermissionRequest()
        var prompts = 0
        val denied = async { runCatching { permission.await { prompts++ } } }
        runCurrent()
        permission.complete(false)
        assertIs<IllegalStateException>(denied.await().exceptionOrNull())
        val retry = async { permission.await { prompts++ } }
        runCurrent()
        permission.complete(true)
        retry.await()
        assertEquals(2, prompts)
    }

    @Test fun aFailedLaunchReleasesAllWaitersAndAllowsRetry() = runTest {
        val permission = PhonePermissionRequest()
        var joined: Deferred<Result<Unit>>? = null
        val first = runCatching {
            permission.await {
                joined = async(start = CoroutineStart.UNDISPATCHED) {
                    runCatching { permission.await { error("must share the first prompt") } }
                }
                error("launcher unavailable")
            }
        }
        assertEquals("launcher unavailable", first.exceptionOrNull()?.message)
        assertEquals("launcher unavailable", joined!!.await().exceptionOrNull()?.message)
        val retry = async { permission.await {} }
        runCurrent()
        permission.complete(true)
        retry.await()
    }
}
