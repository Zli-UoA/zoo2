package zoo.android

import android.net.NetworkCapabilities
import kotlin.test.*
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class NetworkStatusTest {
    @Test
    fun onlyValidatedInternetConnectionsAreOnline() {
        assertFalse(hasInternet(null))
        val caps = NetworkCapabilities()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        assertFalse(hasInternet(caps))
        caps.addCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        assertTrue(hasInternet(caps))
        caps.removeCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        assertFalse(hasInternet(caps))
    }
}
