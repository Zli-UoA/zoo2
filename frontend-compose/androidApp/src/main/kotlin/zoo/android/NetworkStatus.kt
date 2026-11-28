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
internal class DefaultNetworkCallback(private val refresh: (unblockedNetwork: Network?) -> Unit) : ConnectivityManager.NetworkCallback() {
    private var blockedNetwork: Network? = null

    override fun onAvailable(network: Network) {
        // A newly reported default has its own initial blocked-status notification.
        blockedNetwork = null
        refresh(null)
    }

    override fun onLost(network: Network) {
        if (blockedNetwork == network) blockedNetwork = null
        refresh(null)
    }

    override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = refresh(null)

    override fun onBlockedStatusChanged(network: Network, blocked: Boolean) {
        val unblocked = !blocked && blockedNetwork == network
        if (blocked) blockedNetwork = network
        if (unblocked) blockedNetwork = null
        refresh(network.takeIf { unblocked })
    }
}

internal fun ConnectivityManager.activeInternetNetwork(): Network? =
    activeNetwork?.takeIf { hasInternet(getNetworkCapabilities(it)) }

internal class NetworkLifecycleObserver : LifecycleEventObserver {
    private var paused = false
    private var stopped = false
    private var visible = false
    private var resumed = false
    private var listener: ((Boolean, Boolean, Boolean) -> Unit)? = null
    var reconnectRevision = 0L
        private set

    fun bind(since: Long, update: (visible: Boolean, returning: Boolean, resumed: Boolean) -> Unit): () -> Unit {
        listener = update
        update(visible, reconnectRevision != since, resumed)
        return { if (listener === update) listener = null }
    }

    override fun onStateChanged(source: LifecycleOwner, event: Lifecycle.Event) {
        if (event == Lifecycle.Event.ON_PAUSE) paused = true
        if (event == Lifecycle.Event.ON_STOP) stopped = true
        val returning = when (event) {
            Lifecycle.Event.ON_START -> stopped
            Lifecycle.Event.ON_RESUME -> paused
            else -> false
        }
        // A stopped activity reconnects at ON_START. A still-visible activity can
        // return directly to ON_RESUME, for example after leaving the app switcher.
        if (returning) {
            stopped = false
            paused = false
            reconnectRevision++
        }
        visible = source.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        val resumeEvent = event == Lifecycle.Event.ON_RESUME
        resumed = source.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
        listener?.invoke(visible, returning, resumeEvent)
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
