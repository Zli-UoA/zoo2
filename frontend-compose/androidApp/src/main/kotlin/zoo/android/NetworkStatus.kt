package zoo.android

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collectLatest

/** Android's external validation probe can be blocked even when the Zoo server is reachable. */
internal fun hasInternet(capabilities: NetworkCapabilities?): Boolean =
    capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true

/** Android can keep the default network while changing whether this app may use it. */
internal class DefaultNetworkCallback(private val refresh: () -> Unit) : ConnectivityManager.NetworkCallback() {
    override fun onAvailable(network: Network) = refresh()

    override fun onLost(network: Network) = refresh()

    override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = refresh()

    override fun onBlockedStatusChanged(network: Network, blocked: Boolean) = refresh()
}

internal fun ConnectivityManager.activeInternetNetwork(): Network? =
    activeNetwork?.takeIf { hasInternet(getNetworkCapabilities(it)) }

internal class NetworkLifecycleObserver(
    private val isVisible: () -> Boolean,
    private val update: (visible: Boolean, returning: Boolean) -> Unit,
    private val onResume: () -> Unit,
) : LifecycleEventObserver {
    private var paused = false

    override fun onStateChanged(source: LifecycleOwner, event: Lifecycle.Event) {
        if (event == Lifecycle.Event.ON_PAUSE) paused = true
        val returning = when (event) {
            Lifecycle.Event.ON_START -> !isVisible()
            Lifecycle.Event.ON_RESUME -> paused
            else -> false
        }
        // A stopped activity reconnects at ON_START. A still-visible activity can
        // return directly to ON_RESUME, for example after leaving the app switcher.
        if (returning) paused = false
        update(source.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED), returning)
        if (event == Lifecycle.Event.ON_RESUME) onResume()
    }
}

/** Recover if a network callback is unavailable or arrives before the active network is readable. */
internal suspend fun recheckDefaultNetworkWhileOffline(visibleAndOffline: Flow<Boolean>, refresh: () -> Unit) {
    visibleAndOffline.collectLatest { shouldRecheck ->
        if (shouldRecheck) {
            var interval = 2_000L
            while (true) {
                delay(interval)
                refresh()
                interval = (interval * 2).coerceAtMost(10_000L)
            }
        }
    }
}
