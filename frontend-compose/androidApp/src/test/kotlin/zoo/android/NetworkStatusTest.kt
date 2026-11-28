package zoo.android

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import org.robolectric.RuntimeEnvironment
import kotlin.test.*
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class NetworkStatusTest {
    @Test
    @Config(sdk = [35], manifest = Config.NONE)
    fun unblockingTheSameDefaultNetworkRefreshesConnectivityWithoutAnAvailableEvent() {
        val connectivity = RuntimeEnvironment.getApplication()
            .getSystemService(ConnectivityManager::class.java)
        val manager = shadowOf(connectivity)
        val network = assertNotNull(connectivity.activeNetwork)
        val networkInfo = assertNotNull(connectivity.activeNetworkInfo)
        val caps = NetworkCapabilities()
        shadowOf(caps).addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        manager.setNetworkCapabilities(network, caps)
        val observed = mutableListOf<Network?>()
        val callback = DefaultNetworkCallback { observed += connectivity.activeInternetNetwork() }
        connectivity.registerDefaultNetworkCallback(callback)
        try {
            manager.setActiveNetworkInfo(null)
            assertNull(connectivity.activeInternetNetwork())
            manager.setActiveNetworkInfo(networkInfo)

            // Android can unblock the existing default network without sending onAvailable.
            manager.networkCallbacks.single().onBlockedStatusChanged(network, false)
            assertEquals(listOf<Network?>(network), observed)
        } finally {
            connectivity.unregisterNetworkCallback(callback)
        }
    }

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
