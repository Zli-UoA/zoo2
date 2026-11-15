package zoo.android

import android.net.NetworkCapabilities
import kotlin.test.*
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class NetworkStatusTest {
    @Test
    fun blockedExternalValidationDoesNotDisableAReachableServer() {
        assertFalse(hasInternet(null))
        val caps = NetworkCapabilities()
        shadowOf(caps).addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        assertTrue(hasInternet(caps), "An intranet can reach Zoo without Android's validation probe")
        shadowOf(caps).addCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        assertTrue(hasInternet(caps))
        shadowOf(caps).removeCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        assertTrue(hasInternet(caps), "Validation changes must not trigger offline/online reconnect cycles")
        shadowOf(caps).removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        assertFalse(hasInternet(caps))
    }
}
