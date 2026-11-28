package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.statement.bodyAsText
import io.ktor.http.HttpProtocolVersion
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertFalse
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate

class SessionHttpEngineTest {
    @Test
    fun reconnectUsesANewHttp2ConnectionWhileAnOldRequestFinishes() = runBlocking {
        val certificate = HeldCertificate.Builder()
            .commonName("localhost").addSubjectAlternativeName("localhost").build()
        val serverTls = HandshakeCertificates.Builder().heldCertificate(certificate).build()
        val clientTls = HandshakeCertificates.Builder().addTrustedCertificate(certificate.certificate).build()
        val release = CountDownLatch(1)
        MockWebServer().use { server ->
            server.useHttps(serverTls.sslSocketFactory(), false)
            server.dispatcher = object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse {
                    if (request.path == "/in-flight" && !release.await(10, TimeUnit.SECONDS)) {
                        return MockResponse().setResponseCode(500)
                    }
                    return MockResponse().setBody(request.path.orEmpty())
                }
            }
            val engine = SessionHttpEngine {
                config { sslSocketFactory(clientTls.sslSocketFactory(), clientTls.trustManager) }
            }
            val http = HttpClient(engine)
            try {
                val inFlight = async { http.get(server.url("/in-flight").toString()) }
                val first = withContext(Dispatchers.IO) { server.takeRequest(5, TimeUnit.SECONDS) }
                assertEquals(0, assertNotNull(first).sequenceNumber)

                engine.reconnect()
                val resumed = http.get(server.url("/resumed").toString())
                assertEquals(HttpProtocolVersion.HTTP_2_0, resumed.version)
                assertEquals("/resumed", resumed.bodyAsText())
                val next = assertNotNull(withContext(Dispatchers.IO) { server.takeRequest(5, TimeUnit.SECONDS) })
                assertEquals(0, next.sequenceNumber)
                assertFalse(inFlight.isCompleted)

                release.countDown()
                assertEquals("/in-flight", inFlight.await().bodyAsText())
            } finally {
                release.countDown()
                http.close()
                engine.close()
            }
        }
    }
}
