package zoo.android

import android.net.NetworkCapabilities

/** Android's external validation probe can be blocked even when the Zoo server is reachable. */
internal fun hasInternet(capabilities: NetworkCapabilities?): Boolean =
    capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
