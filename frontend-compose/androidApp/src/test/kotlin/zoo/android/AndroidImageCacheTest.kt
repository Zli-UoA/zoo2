package zoo.android

import java.io.Closeable
import java.nio.file.Files
import kotlin.test.*
import java.util.concurrent.TimeUnit
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
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
        fun cache(
            account: String = "account",
            configure: OkHttpClient.Builder.() -> Unit = {},
        ) = AndroidImageCache(directory.resolve("$account/images")) {
            sslSocketFactory(clientTls.sslSocketFactory(), clientTls.trustManager)
            configure()
        }
        override fun close() {
            server.shutdown()
            directory.deleteRecursively()
        }
    }

    @Test fun differentImagesOnTheSameHostReuseTheConnection() = runBlocking {
        Fixture().use { fixture ->
            repeat(2) { fixture.server.enqueue(MockResponse().setBody("emoji")) }
            fixture.cache().use { cache ->
                for (path in listOf("/first.png", "/second.png")) {
                    assertEquals("emoji", cache.download(ImageRequest(fixture.server.url(path).toString()), 1024).decodeToString())
                }
            }
            assertEquals(0, assertNotNull(fixture.server.takeRequest(5, TimeUnit.SECONDS)).sequenceNumber)
            assertEquals(1, assertNotNull(fixture.server.takeRequest(5, TimeUnit.SECONDS)).sequenceNumber)
        }
    }

    @Test fun keepsHostSlotsUntilTheResponseBodiesFinish() = runBlocking {
        Fixture().use { fixture ->
            repeat(FOREGROUND_IMAGE_PARALLELISM) {
                fixture.server.enqueue(MockResponse().setBody("image").setBodyDelay(2, TimeUnit.SECONDS))
            }
            fixture.server.enqueue(MockResponse().setBody("last"))
            fixture.cache().use { cache ->
                val images = List(FOREGROUND_IMAGE_PARALLELISM + 1) { index ->
                    async { cache.download(ImageRequest(fixture.server.url("/$index.png").toString()), 1024) }
                }
                withContext(Dispatchers.IO) {
                    repeat(FOREGROUND_IMAGE_PARALLELISM) {
                        assertNotNull(fixture.server.takeRequest(5, TimeUnit.SECONDS))
                    }
                    assertNull(fixture.server.takeRequest(300, TimeUnit.MILLISECONDS))
                }
                withTimeout(10_000) { images.awaitAll() }
                assertEquals(FOREGROUND_IMAGE_PARALLELISM + 1, fixture.server.requestCount)
            }
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
                    val image = withTimeout(5_000) { cache.cachedPublicImage(cached.url, 1024) }
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
                assertEquals("cached", cache.download(ImageRequest(f.url), 1024).decodeToString())
                assertEquals("cached", cache.download(ImageRequest(f.url), 1024).decodeToString())
            }
            f.cache().use { cache -> assertEquals("cached", cache.download(ImageRequest(f.url), 1024).decodeToString()) }
            assertEquals(1, f.server.requestCount)
            assertNull(f.server.takeRequest().getHeader("Authorization"))
        }
    }

    @Test fun reusesCachedRedirectAndSignedImageAfterReopeningClient() = runBlocking {
        Fixture().use { f ->
            val signed = f.server.url("/original?X-Amz-Signature=one").toString()
            f.server.enqueue(MockResponse().setResponseCode(307)
                .setHeader("Location", signed)
                .setHeader("Cache-Control", "private, max-age=300"))
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=300").setBody("cached"))
            f.server.enqueue(MockResponse().setBody("unexpected network request"))
            f.cache().use { assertEquals("cached", it.download(ImageRequest(f.url), 1024).decodeToString()) }
            f.cache().use { assertEquals("cached", it.download(ImageRequest(f.url), 1024).decodeToString()) }
            assertEquals(2, f.server.requestCount)
        }
    }

    @Test fun cacheFirstDisplaysPreviouslyStoredImagesWithoutRevalidation() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=0").setBody("photo"))
            f.cache().use { cache -> assertEquals("photo", cache.download(ImageRequest(f.url), 1024).decodeToString()) }
            f.cache().use { cache -> assertEquals("photo", cache.download(ImageRequest(f.url), 1024).decodeToString()) }
            assertEquals(1, f.server.requestCount)
        }
    }

    @Test fun cachedMissingImageCanRecover() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setResponseCode(404).setHeader("Cache-Control", "public, max-age=600"))
            f.server.enqueue(MockResponse().setBody("recovered"))
            f.cache().use { cache ->
                assertFailsWith<ImageHttpFailure> { cache.download(ImageRequest(f.url), 1024) }
                assertEquals("recovered", cache.download(ImageRequest(f.url), 1024).decodeToString())
                assertEquals(2, f.server.requestCount)
            }
        }
    }

    @Test fun failedCapacityCleanupCanBeRetriedWithTheSameSize() = runBlocking {
        Fixture().use { f ->
            val cachePath = f.directory.resolve("account/images-attachment-bodies-v3")
            assertTrue(requireNotNull(cachePath.parentFile).mkdirs())
            cachePath.writeText("unreadable as a directory")
            f.cache().use { cache ->
                assertFalse(cache.resizeAttachmentCache(0))
                assertTrue(cachePath.delete())
                assertTrue(cache.resizeAttachmentCache(0))
            }
        }
    }

    @Test fun revocationPreventsAnInFlightDownloadFromRestoringTheCachedImage() = runBlocking {
        Fixture().use { f ->
            val releaseResponse = CountDownLatch(1)
            val firstRequest = AtomicBoolean(true)
            f.server.dispatcher = object : okhttp3.mockwebserver.Dispatcher() {
                override fun dispatch(request: okhttp3.mockwebserver.RecordedRequest): MockResponse =
                    if (firstRequest.getAndSet(false)) {
                        check(releaseResponse.await(5, TimeUnit.SECONDS))
                        MockResponse().setBody("old")
                    } else MockResponse().setBody("new")
            }
            val key = "id:original"
            f.cache().use { cache ->
                val pending = async {
                    assertFailsWith<ImageHttpFailure> { cache.signedImage(f.url, key) }
                }
                try {
                    assertNotNull(withContext(Dispatchers.IO) { f.server.takeRequest(5, TimeUnit.SECONDS) })
                    cache.removeAttachmentImages("id")
                    assertNull(cache.cachedAttachmentPreview(key))
                    assertEquals(403, assertFailsWith<ImageHttpFailure> { cache.signedImage(f.url, key) }.status)
                    cache.allowAttachmentImages("id")
                } finally {
                    releaseResponse.countDown()
                }
                assertEquals(403, pending.await().status)
                assertNull(cache.cachedAttachmentPreview(key))
                assertEquals("new", cache.signedImage(f.url, key).decodeToString())
            }
            f.cache().use { cache ->
                assertEquals("new", cache.cachedAttachmentPreview(key)?.decodeToString())
            }
            assertEquals(2, f.server.requestCount)
        }
    }


    @Test fun usesSameCapacityForSignedAndPublicImagesAcrossClients() = runBlocking {
        Fixture().use { f ->
            val image = "x".repeat(512)
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "no-store").setBody(image))
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "no-cache, must-revalidate")
                .setHeader("Age", "10000000").setHeader("Vary", "*").setBody(image))
            f.cache().use { foreground ->
                f.cache().use { push ->
                    foreground.signedImage("${f.url}?token=old", "id:original")
                    push.download(ImageRequest(f.url), 1024)
                    assertEquals(image, push.cachedAttachmentPreview("id:original")?.decodeToString())
                    assertEquals(image, foreground.cachedPublicImage(f.url, 1024)?.decodeToString())
                    assertTrue(push.resizeAttachmentCache(1000))
                    assertNull(foreground.cachedAttachmentPreview("id:original"))
                    assertTrue(foreground.resizeAttachmentCache(0))
                    assertNull(push.cachedPublicImage(f.url, 1024))
                }
            }
            assertEquals(2, f.server.requestCount)
        }
    }

    @Test fun signedImagesSurviveRestartAndSignedUrlRotation() = runBlocking {
        Fixture().use { f ->
            val key = "id:thumbnail"
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "no-store, must-revalidate")
                .setHeader("Vary", "*").setBody("cached"))
            f.cache().use { it.signedImage("${f.url}?token=old", key) }
            f.cache().use {
                assertEquals("cached", it.cachedAttachmentPreview(key)?.decodeToString())
                assertEquals("cached", it.signedImage("${f.url}?token=new", key).decodeToString())
            }
            assertEquals(1, f.server.requestCount)
        }
    }

    @Test fun explicitRefreshUsesValidatorsOnlyAtTheMatchingRedirectDestination() = runBlocking {
        Fixture().use { f ->
            val lastModified = "Wed, 21 Oct 2015 07:28:00 GMT"
            repeat(2) { attempt ->
                f.server.enqueue(MockResponse().setResponseCode(307).setHeader("Location", "/final"))
                f.server.enqueue(if (attempt == 0) MockResponse().setBody("cached")
                    .setHeader("Cache-Control", "no-store").setHeader("Last-Modified", lastModified)
                else MockResponse().setResponseCode(304))
            }
            val route = ImageRequest(f.url, proxied = true)
            val source = "https://example.org/emoji.png"
            f.cache().use { it.download(route, 1024, source = source) }
            f.cache().use {
                assertEquals("cached", it.cachedPublicImage(source, 1024)?.decodeToString())
                assertEquals("cached", it.download(route, 1024, bypassCache = true, source = source).decodeToString())
            }
            assertEquals(4, f.server.requestCount)
            repeat(3) { assertNull(f.server.takeRequest().getHeader("If-Modified-Since")) }
            assertEquals(lastModified, f.server.takeRequest().getHeader("If-Modified-Since"))
        }
    }

    @Test fun etagRefreshReusesBodyAfterANetworkFailure() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("ETag", "\"v1\"").setBody("cached"))
            f.server.enqueue(MockResponse().setResponseCode(503))
            f.server.enqueue(MockResponse().setResponseCode(304))
            val key = "id:original"
            f.cache().use {
                it.signedImage("${f.url}?X-Amz-Signature=first", key)
                assertFailsWith<ImageHttpFailure> { it.signedImage(f.url, key, retry = true) }
                assertEquals("cached", it.signedImage(f.url, key).decodeToString())
                assertEquals("cached", it.signedImage("${f.url}?X-Amz-Signature=second", key, retry = true).decodeToString())
            }
            assertEquals(3, f.server.requestCount)
            assertNull(f.server.takeRequest().getHeader("If-None-Match"))
            repeat(2) { assertEquals("\"v1\"", f.server.takeRequest().getHeader("If-None-Match")) }
        }
    }

    @Test fun discardingAnUndecodableBodyRemovesItsValidator() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("ETag", "broken").setBody("invalid"))
            f.server.enqueue(MockResponse().setBody("replacement"))
            f.cache().use { cache ->
                val route = ImageRequest(f.url)
                val bytes = cache.download(route, 1024)
                cache.discardUndecodableImage(f.url, null, bytes)
                assertEquals("replacement", cache.download(route, 1024, bypassCache = true).decodeToString())
            }
            f.server.takeRequest()
            assertNull(f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun retainsLegacyAttachmentBodyWithAnExpiredDeadline() = runBlocking {
        Fixture().use { f ->
            val key = "legacy:thumbnail"
            val legacy = f.directory.resolve("legacy/images")
            val directory = legacy.resolveSibling("images-attachment-bodies-v3")
            assertTrue(directory.mkdirs())
            val file = directory.resolve(AttachmentImageDiskCache.digest(key))
            java.io.DataOutputStream(file.outputStream()).use {
                it.writeUTF("http:fresh:old-hash")
                it.writeLong(0)
                it.writeLong(0)
                it.writeUTF("")
                it.writeLong(Long.MIN_VALUE)
                it.writeLong(Long.MIN_VALUE)
                it.writeBytes("old image")
            }
            AndroidImageCache(f.directory.resolve("account/images"), legacyDirectory = legacy).use {
                assertEquals("old image", it.cachedAttachmentPreview(key)?.decodeToString())
                assertFalse(file.exists())
            }
        }
    }
}
