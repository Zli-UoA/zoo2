package zoo.android

import java.io.Closeable
import java.nio.file.Files
import kotlin.test.*
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import okhttp3.Dispatcher
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.SocketPolicy
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate

class AndroidImageCacheTest {
    internal class Fixture : Closeable {
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
        fun cache(configure: OkHttpClient.Builder.() -> Unit = {}) = AndroidImageCache(directory) {
            sslSocketFactory(clientTls.sslSocketFactory(), clientTls.trustManager)
            configure()
        }
        override fun close() {
            server.shutdown()
            directory.deleteRecursively()
        }
    }

    @Test fun cachedImageDoesNotWaitForABusyNetworkDispatcher() = runBlocking {
        Fixture().use { fixture ->
            fixture.cache { dispatcher(Dispatcher().apply { maxRequests = 1 }) }.use { cache ->
                val cached = ImageRequest(fixture.url)
                fixture.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=3600").setBody("cached"))
                cache.download(cached, 1024)
                assertNotNull(fixture.server.takeRequest(5, TimeUnit.SECONDS))
                fixture.server.enqueue(MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE))
                val pending = async { cache.download(ImageRequest(fixture.server.url("/slow.png").toString()), 1024) }
                try {
                    assertNotNull(withContext(Dispatchers.IO) { fixture.server.takeRequest(5, TimeUnit.SECONDS) })
                    val image = withTimeout(5_000) { cache.imageIfCached(cached, 1024) }
                    assertEquals("cached", image?.decodeToString())
                    assertEquals(2, fixture.server.requestCount)
                } finally {
                    pending.cancelAndJoin()
                }
            }
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

    @Test fun retryDoesNotSendTheCachedValidatorOrReuseTheBrokenBody() = runBlocking {
        Fixture().use { fixture ->
            fixture.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=3600")
                .setHeader("ETag", "\"v1\"").setBody("broken"))
            fixture.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=3600")
                .setHeader("ETag", "\"v1\"").setBody("repaired"))
            fixture.cache().use { cache ->
                val request = ImageRequest(fixture.url)
                assertEquals("broken", cache.imageFromCacheFirst(request).decodeToString())
                assertEquals("repaired", cache.download(request, 1024, bypassCache = true).decodeToString())
                assertEquals("repaired", cache.imageFromCacheFirst(request).decodeToString())
                assertEquals(2, fixture.server.requestCount)
                assertNotNull(fixture.server.takeRequest(5, TimeUnit.SECONDS))
                val retry = assertNotNull(fixture.server.takeRequest(5, TimeUnit.SECONDS))
                assertNull(retry.getHeader("If-None-Match"))
                assertEquals("no-cache", retry.getHeader("Cache-Control"))
            }
        }
    }

    @Test fun cacheFirstRevalidatesNoCacheImagesWithTheirEtag() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "public, no-cache")
                .setHeader("ETag", "\"v1\"").setBody("original"))
            f.server.enqueue(MockResponse().setResponseCode(304).setHeader("Cache-Control", "public, max-age=3600"))
            f.cache().use { cache ->
                assertEquals("original", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString())
                assertEquals("original", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString())
            }
            assertEquals(2, f.server.requestCount)
            f.server.takeRequest()
            assertEquals("\"v1\"", f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun reusesCachedRedirectAndSignedImageAfterReopeningClient() = runBlocking {
        Fixture().use { f ->
            val signed = f.server.url("/original?signature=one").toString()
            f.server.enqueue(MockResponse().setResponseCode(307)
                .setHeader("Location", signed)
                .setHeader("Cache-Control", "private, max-age=300"))
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=300").setBody("cached"))
            f.server.enqueue(MockResponse().setBody("unexpected network request"))
            f.cache().use { assertEquals("cached", it.image(f.url).decodeToString()) }
            f.cache().use { assertEquals("cached", it.image(f.url).decodeToString()) }
            assertEquals(2, f.server.requestCount)
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
    @Test fun cacheFirstDisplaysPreviouslyStoredImagesWithoutRevalidation() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=0").setBody("photo"))
            f.cache().use { cache -> assertEquals("photo", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString()) }
            f.cache().use { cache -> assertEquals("photo", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString()) }
            assertEquals(1, f.server.requestCount)
        }
    }

    @Test fun cacheFirstStillHonorsNoStore() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "no-store").setBody("first"))
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "no-store").setBody("second"))
            f.cache().use { cache ->
                assertEquals("first", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString())
                assertEquals("second", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString())
            }
            assertEquals(2, f.server.requestCount)
        }
    }

    @Test fun cachedMissingImageCanRecover() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setResponseCode(404).setHeader("Cache-Control", "public, max-age=600"))
            f.server.enqueue(MockResponse().setBody("recovered"))
            f.cache().use { cache ->
                assertFailsWith<ImageHttpFailure> { cache.image(f.url) }
                assertEquals("recovered", cache.imageFromCacheFirst(ImageRequest(f.url)).decodeToString())
                assertEquals(2, f.server.requestCount)
            }
        }
    }

    @Test fun signedImagesAreNotStoredOnDisk() = runBlocking {
        Fixture().use { f ->
            repeat(2) { f.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=600").setBody("photo")) }
            f.cache().use { cache ->
                cache.signedImage(f.url)
                assertNull(cache.imageIfCached(ImageRequest(f.url), 1024))
                cache.signedImage(f.url)
            }
            repeat(2) { assertTrue(f.server.takeRequest().getHeader("Cache-Control").orEmpty().contains("no-store")) }
        }
    }

}
