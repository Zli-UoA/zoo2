package zoo.android

import android.net.NetworkCapabilities

/** A captive portal or a Wi-Fi network without internet must not trigger reconnect loops. */
internal fun hasInternet(capabilities: NetworkCapabilities?): Boolean =
    capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true &&
        capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
