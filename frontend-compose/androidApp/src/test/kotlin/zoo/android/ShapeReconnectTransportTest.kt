package zoo.android

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import io.ktor.client.HttpClient
import io.ktor.client.plugins.HttpTimeout
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.test.fail
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import okhttp3.mockwebserver.SocketPolicy
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate
import zoo.Config
import zoo.Platform
import zoo.SyncedTable
import zoo.text

class ShapeReconnectTransportTest {
    @Test
    fun reconnectRestoresTheFirstPageAfterTheServerInvalidatesTheOldConnection() = runBlocking {
        val certificate = HeldCertificate.Builder()
            .commonName("localhost").addSubjectAlternativeName("localhost").build()
        val serverTls = HandshakeCertificates.Builder().heldCertificate(certificate).build()
        val clientTls = HandshakeCertificates.Builder().addTrustedCertificate(certificate.certificate).build()
        val invalidate = AtomicBoolean(false)
        val snapshots = AtomicInteger()
        val oldLive = CompletableDeferred<Unit>()
        val newLive = CompletableDeferred<Unit>()
        fun response(handle: String) = MockResponse()
            .setHeader("content-type", "application/json")
            .setHeader("electric-handle", handle)
            .setHeader("electric-offset", if (handle == "old") "1_0" else "2_0")
            .setHeader("electric-cursor", "same-cursor")
            .setHeader("electric-schema", """{"id":{"type":"text"}}""")
        MockWebServer().use { server ->
            server.useHttps(serverTls.sslSocketFactory(), false)
            server.dispatcher = object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse {
                    val url = checkNotNull(request.requestUrl)
                    if (url.encodedPath != "/api/shapes/messages") return MockResponse().setResponseCode(404)
                    val handle = url.queryParameter("handle")
                    if (url.queryParameter("subset__where") != null) {
                        val id = if (snapshots.incrementAndGet() == 1) "old-row" else "new-row"
                        return response(checkNotNull(handle)).setBody(
                            """{"metadata":{"xmin":"1","xmax":"1","xip_list":[],"snapshot_mark":1,"database_lsn":"1"},"data":[{"key":"$id","value":{"id":"$id"},"headers":{"operation":"insert"}}]}""",
                        )
                    }
                    if (url.queryParameter("live") == "true") {
                        if (handle == "old" && snapshots.get() > 0) oldLive.complete(Unit)
                        if (handle == "new" && snapshots.get() > 1) newLive.complete(Unit)
                        return MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE)
                    }
                    if (handle == "old" && invalidate.compareAndSet(true, false)) {
                        return MockResponse().setResponseCode(409).setHeader("electric-handle", "new").setBody("[]")
                    }
                    if (handle == "new") return response("new").setResponseCode(204)
                    return response("old").setBody("""[{"headers":{"control":"up-to-date"}}]""")
                }
            }
            val engine = SessionHttpEngine {
                config { sslSocketFactory(clientTls.sslSocketFactory(), clientTls.trustManager) }
            }
            val http = HttpClient(engine) { install(HttpTimeout) }
            val platform = object : Platform {
                override suspend fun headers() = emptyMap<String, String>()
                override suspend fun readCache(key: String): String? = null
                override suspend fun writeCache(key: String, value: String) = Unit
                override fun uuid() = "test-id"
                override fun now() = "2026-11-28T00:00:00Z"
                override fun navigate(path: String) = Unit
                override fun translate(key: String, args: Map<String, String>) = key
                override fun action(name: String) = Unit
                override fun time(timestamp: String) = timestamp
                override fun day(timestamp: String) = timestamp
            }
            val table = SyncedTable(
                "messages", Config(server.url("/").toString().trimEnd('/'), "/api/shapes", "alice", false),
                platform, http, this, mapOf("channel_id" to "test-channel"), persist = false, onDemand = true,
            )
            val activity = object : LifecycleOwner {
                override val lifecycle = LifecycleRegistry.createUnsafe(this)
            }

            val observer = NetworkLifecycleObserver()
            activity.lifecycle.addObserver(observer)
            activity.lifecycle.currentState = Lifecycle.State.RESUMED
            var visible = true
            val unbind = observer.bind(observer.reconnectRevision) { nowVisible, returning, _ ->
                visible = nowVisible
                if (returning) {
                    engine.reconnect()
                    table.refresh(reconnect = true)
                }
            }
            var phase = "first page"
            try {
                withTimeout(10_000) {
                    table.rows.first { rows -> rows.any { it.text("id") == "old-row" } }
                    phase = "original live request"
                    oldLive.await()

                    activity.lifecycle.currentState = Lifecycle.State.STARTED
                    assertTrue(visible)
                    invalidate.set(true)
                    activity.lifecycle.currentState = Lifecycle.State.RESUMED

                    phase = "replacement page"
                    val recovered = table.rows.first { rows -> rows.any { it.text("id") == "new-row" } }
                    assertEquals(listOf("new-row"), recovered.map { it.text("id") })
                    phase = "replacement live request"
                    newLive.await()
                    assertEquals(2, snapshots.get())
                    assertTrue(table.error.value == null)
                }
            } catch (timeout: TimeoutCancellationException) {
                fail("Timed out waiting for $phase (snapshots=${snapshots.get()}, error=${table.error.value})", timeout)
            } finally {
                unbind()
                table.close()
                http.close()
                engine.close()
            }
        }
    }
}
