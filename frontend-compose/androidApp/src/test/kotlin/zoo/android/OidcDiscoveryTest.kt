package zoo.android

import java.net.InetAddress
import java.net.Proxy
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlinx.coroutines.runBlocking
import okhttp3.Dns
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate
import zoo.text

class OidcDiscoveryTest {
    @Test
    fun retriesASecondAddressBeforeLaunchingTheBrowser() = runBlocking {
        val address = InetAddress.getByName("127.0.0.1")
        val unavailable = InetAddress.getByName("127.0.0.2")
        val certificate = HeldCertificate.Builder()
            .commonName("localhost").addSubjectAlternativeName("localhost").build()
        val serverTls = HandshakeCertificates.Builder().heldCertificate(certificate).build()
        val clientTls = HandshakeCertificates.Builder().addTrustedCertificate(certificate.certificate).build()
        MockWebServer().use { server ->
            server.useHttps(serverTls.sslSocketFactory(), false)
            server.start(address, 0)
            val issuer = "https://localhost:${server.port}"
            server.enqueue(MockResponse().setBody("""{"issuer":"$issuer"}"""))

            val discovery = readOidcDiscovery(issuer) {
                proxy(Proxy.NO_PROXY)
                dns(object : Dns {
                    override fun lookup(hostname: String) = listOf(unavailable, address)
                })
                sslSocketFactory(clientTls.sslSocketFactory(), clientTls.trustManager)
            }
            assertEquals(issuer, discovery.text("issuer"))
            assertEquals("/.well-known/openid-configuration", server.takeRequest().path)
        }
    }
}
