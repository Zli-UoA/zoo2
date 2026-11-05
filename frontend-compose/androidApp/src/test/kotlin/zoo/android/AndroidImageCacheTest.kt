package zoo.android

import java.io.Closeable
import java.nio.file.Files
import kotlin.test.*
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate

class AndroidImageCacheTest {
    private class Fixture : Closeable {
        val directory = Files.createTempDirectory("zoo-image-cache").toFile()
        private val certificate = HeldCertificate.Builder()
            .commonName("localhost").addSubjectAlternativeName("localhost").build()
        private val serverTls = HandshakeCertificates.Builder().heldCertificate(certificate).build()
        private val clientTls = HandshakeCertificates.Builder().addTrustedCertificate(certificate.certificate).build()
        val server = MockWebServer().apply {
            useHttps(serverTls.sslSocketFactory(), false)
            start()
        }
        val url get() = server.url("/image.png").toString()
        fun cache() = AndroidImageCache(directory) {
            sslSocketFactory(clientTls.sslSocketFactory(), clientTls.trustManager)
        }
        override fun close() {
            server.shutdown()
            directory.deleteRecursively()
        }
    }

    @Test fun reusesImagesAfterClosingAndReopeningTheClient() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=3600").setBody("cached"))
            f.server.enqueue(MockResponse().setBody("unexpected network request"))
            f.cache().use { cache ->
                assertEquals("cached", cache.image(f.url).decodeToString())
                assertEquals("cached", cache.image(f.url).decodeToString())
            }
            f.cache().use { cache -> assertEquals("cached", cache.image(f.url).decodeToString()) }
            assertEquals(1, f.server.requestCount)
            assertNull(f.server.takeRequest().getHeader("Authorization"))
        }
    }

    @Test fun revalidatesExpiredImagesWithTheirEtag() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=0")
                .setHeader("ETag", "\"v1\"").setBody("original"))
            f.server.enqueue(MockResponse().setResponseCode(304).setHeader("Cache-Control", "public, max-age=3600"))
            f.cache().use { cache ->
                assertEquals("original", cache.image(f.url).decodeToString())
                assertEquals("original", cache.image(f.url).decodeToString())
            }
            assertEquals(2, f.server.requestCount)
            f.server.takeRequest()
            assertEquals("\"v1\"", f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun doesNotPersistNoStoreResponses() = runBlocking {
        Fixture().use { f ->
            for (body in listOf("first", "second")) {
                f.server.enqueue(MockResponse().setHeader("Cache-Control", "no-store").setBody(body))
            }
            f.cache().use { cache -> assertEquals("first", cache.image(f.url).decodeToString()) }
            f.cache().use { cache -> assertEquals("second", cache.image(f.url).decodeToString()) }
            assertEquals(2, f.server.requestCount)
        }
    }
}
