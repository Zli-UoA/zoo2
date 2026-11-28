package zoo.android

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
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
import org.robolectric.shadows.ShadowNetwork

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class NetworkStatusTest {
    @Test
    fun switchingAppsReconnectsOnTheFirstResumeEvenIfTheActivityNeverStopped() {
        val activity = object : LifecycleOwner {
            override val lifecycle = LifecycleRegistry.createUnsafe(this)
        }

        val observer = NetworkLifecycleObserver()
        activity.lifecycle.addObserver(observer)
        activity.lifecycle.currentState = Lifecycle.State.RESUMED
        var visible = true
        val reconnects = mutableListOf<Lifecycle.State>()
        observer.bind(observer.reconnectRevision) { nowVisible, returning, _ ->
            visible = nowVisible
            if (returning) reconnects += activity.lifecycle.currentState
        }
        assertTrue(reconnects.isEmpty(), "Installing the observer must not restart active connections")

        activity.lifecycle.currentState = Lifecycle.State.STARTED
        assertTrue(visible)
        activity.lifecycle.currentState = Lifecycle.State.RESUMED
        assertEquals(listOf(Lifecycle.State.RESUMED), reconnects)

        activity.lifecycle.currentState = Lifecycle.State.CREATED
        assertFalse(visible)
        activity.lifecycle.currentState = Lifecycle.State.RESUMED
        assertEquals(listOf(Lifecycle.State.RESUMED, Lifecycle.State.RESUMED), reconnects)
    }

    @Test
    fun sessionRestorationReceivesTheResumeEvenWhenItFinishesAfterThePause() {
        for (bindAfterResume in listOf(false, true)) {
            val activity = object : LifecycleOwner {
                override val lifecycle = LifecycleRegistry.createUnsafe(this)
            }

            val observer = NetworkLifecycleObserver()
            activity.lifecycle.addObserver(observer)
            activity.lifecycle.currentState = Lifecycle.State.RESUMED
            activity.lifecycle.currentState = Lifecycle.State.STARTED
            val revisionAtSessionStart = observer.reconnectRevision

            if (bindAfterResume) activity.lifecycle.currentState = Lifecycle.State.RESUMED
            var reconnects = 0
            val unbind = observer.bind(revisionAtSessionStart) { _, returning, _ ->
                if (returning) reconnects++
            }
            if (!bindAfterResume) activity.lifecycle.currentState = Lifecycle.State.RESUMED
            assertEquals(1, reconnects)

            unbind()
            activity.lifecycle.currentState = Lifecycle.State.STARTED
            activity.lifecycle.currentState = Lifecycle.State.RESUMED
            assertEquals(1, reconnects)
        }
    }

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
    fun unblockingAStillReportedDefaultNetworkSignalsAnotherReconnectOnce() {
        val connectivity = RuntimeEnvironment.getApplication().getSystemService(ConnectivityManager::class.java)
        val manager = shadowOf(connectivity)
        val network = assertNotNull(connectivity.activeNetwork)
        val capabilities = NetworkCapabilities()
        shadowOf(capabilities).addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        manager.setNetworkCapabilities(network, capabilities)
        val reconnects = mutableListOf<Network>()
        val callback = DefaultNetworkCallback { unblocked ->
            connectivity.activeInternetNetwork()?.takeIf { it == unblocked }?.let(reconnects::add)
        }
        callback.onBlockedStatusChanged(network, false)
        callback.onBlockedStatusChanged(network, true)
        assertEquals(network, connectivity.activeInternetNetwork())
        callback.onBlockedStatusChanged(network, false)
        callback.onBlockedStatusChanged(network, false)
        assertEquals(listOf(network), reconnects)

        callback.onBlockedStatusChanged(network, true)
        callback.onLost(network)
        callback.onBlockedStatusChanged(network, false)
        assertEquals(listOf(network), reconnects)

        callback.onBlockedStatusChanged(network, true)
        callback.onAvailable(ShadowNetwork.newInstance(100))
        callback.onAvailable(network)
        callback.onBlockedStatusChanged(network, false)
        assertEquals(listOf(network), reconnects)
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
