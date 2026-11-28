package zoo.android

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import org.robolectric.RuntimeEnvironment
import kotlin.test.*
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class NetworkStatusTest {
    @Test
    @Config(sdk = [28, 35], manifest = Config.NONE)
    @OptIn(ExperimentalCoroutinesApi::class)
    fun networkRecheckBacksOffAndStopsWhenNoLongerVisibleAndOffline() = runTest {
        val visibleAndOffline = MutableStateFlow(false)
        var checks = 0
        backgroundScope.launch {
            recheckDefaultNetworkWhileOffline(visibleAndOffline) {
                checks++
                if (checks == 2) visibleAndOffline.value = false
            }
        }
        advanceTimeBy(10_000)
        assertEquals(0, checks)

        visibleAndOffline.value = true
        advanceTimeBy(2_000)
        runCurrent()
        assertEquals(1, checks)
        advanceTimeBy(3_999)
        runCurrent()
        assertEquals(1, checks)
        advanceTimeBy(1)
        runCurrent()
        assertEquals(2, checks)
        advanceTimeBy(10_000)
        assertEquals(2, checks)

        visibleAndOffline.value = true
        advanceTimeBy(1_000)
        visibleAndOffline.value = false
        advanceTimeBy(10_000)
        assertEquals(2, checks)
    }

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
