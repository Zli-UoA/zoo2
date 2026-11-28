package zoo.android

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
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
