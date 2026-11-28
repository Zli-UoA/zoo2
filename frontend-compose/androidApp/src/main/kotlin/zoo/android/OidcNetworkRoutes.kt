package zoo.android

import java.net.InetAddress
import java.net.Proxy
import java.util.concurrent.ConcurrentHashMap
import okhttp3.Call
import okhttp3.Connection
import okhttp3.Dns
import okhttp3.EventListener

/** Reuse the successful discovery address only while it remains in the host's DNS answers. */
internal class OidcNetworkRoutes(private val delegate: Dns = Dns.SYSTEM) : Dns {
    private val working = ConcurrentHashMap<String, InetAddress>()

    val discoveryEvents = object : EventListener() {
        override fun connectionAcquired(call: Call, connection: Connection) {
            val route = connection.route()
            if (route.proxy.type() == Proxy.Type.DIRECT) {
                route.socketAddress.address?.let { working[route.address.url.host] = it }
            }
        }
    }

    override fun lookup(hostname: String): List<InetAddress> {
        val addresses = delegate.lookup(hostname)
        val preferred = working[hostname] ?: return addresses
        return if (preferred in addresses) listOf(preferred) + addresses.filterNot { it == preferred } else addresses
    }

    suspend fun discover(issuer: String) = readOidcDiscovery(issuer) {
        dns(this@OidcNetworkRoutes)
        eventListener(discoveryEvents)
    }
}
