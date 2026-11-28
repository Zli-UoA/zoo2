package zoo.android

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities

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
