package zoo.android

import java.io.Closeable
import java.nio.file.Files
import kotlin.test.*
import java.util.concurrent.TimeUnit
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
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
        fun cache(
            account: String = "account",
            now: () -> Long = System::currentTimeMillis,
            configure: OkHttpClient.Builder.() -> Unit = {},
        ) = AndroidImageCache(directory.resolve("$account/images"), now = now) {
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
            val signed = f.server.url("/original?X-Amz-Signature=one").toString()
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


    @Test fun revalidatesAnOriginalWhenTheSignedUrlChanges() = runBlocking {
        Fixture().use { f ->
            f.server.enqueue(MockResponse().setHeader("ETag", "\"one\"").setBody("original"))
            f.server.enqueue(MockResponse().setResponseCode(304))
            f.server.enqueue(MockResponse().setHeader("ETag", "\"two\"").setBody("replacement"))
            f.server.enqueue(MockResponse().setBody("other account"))
            val key = "id:original"
            f.cache().use { cache ->
                assertEquals("original", cache.signedImage("${f.url}?X-Amz-Signature=old", key).decodeToString())
            }
            f.cache().use { cache ->
                assertEquals("original", cache.signedImage("${f.url}?X-Amz-Signature=new", key).decodeToString())
                assertEquals("replacement", cache.signedImage("${f.url}?X-Amz-Signature=newer", key).decodeToString())
            }
            f.cache(account = "other").use { cache ->
                assertEquals("other account", cache.signedImage("${f.url}?X-Amz-Signature=newer", key).decodeToString())
            }
            assertEquals(4, f.server.requestCount)
            assertNull(f.server.takeRequest().getHeader("If-None-Match"))
            repeat(2) { assertEquals("\"one\"", f.server.takeRequest().getHeader("If-None-Match")) }
            assertNull(f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun previewsOldAttachmentsUnlessTheServerRequiresRevalidation() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60").setBody("image"))
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60, must-revalidate").setBody("image"))
            f.cache(now = { time }).use { cache ->
                cache.signedImage(f.url, "id:thumbnail")
                cache.signedImage(f.url, "id:original")
            }
            f.cache(now = { time }).use { cache ->
                assertEquals("image", cache.cachedAttachmentPreview("id:thumbnail")?.decodeToString())
                assertEquals("image", cache.cachedAttachmentPreview("id:original")?.decodeToString())
                time += TimeUnit.DAYS.toMillis(7)
                assertEquals("image", cache.cachedAttachmentPreview("id:thumbnail")?.decodeToString())
                assertNull(cache.cachedAttachmentPreview("id:original"))
                cache.removeAttachmentImages("id")
                time = 1_000L
                assertNull(cache.cachedAttachmentPreview("id:thumbnail"))
                assertNull(cache.cachedAttachmentPreview("id:original"))
            }
            assertEquals(2, f.server.requestCount)
        }
    }

    @Test fun honorsAnExplicitStalePreviewWindowAcrossRestarts() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            val key = "id:thumbnail"
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60, stale-while-revalidate=30")
                .setBody("image"))
            f.cache(now = { time }).use { cache -> cache.signedImage(f.url, key) }
            f.cache(now = { time }).use { cache ->
                assertEquals("image", cache.cachedAttachmentPreview(key)?.decodeToString())
                time += 89_999
                assertEquals("image", cache.cachedAttachmentPreview(key)?.decodeToString())
                time++
                assertNull(cache.cachedAttachmentPreview(key))
            }
            assertEquals(1, f.server.requestCount)
        }
    }

    @Test fun responseAgeReducesTheRemainingStalePreviewWindow() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            val key = "id:thumbnail"
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60, stale-while-revalidate=30")
                .setHeader("Age", "80").setBody("image"))
            f.cache(now = { time }).use { cache -> cache.signedImage(f.url, key) }
            f.cache(now = { time }).use { cache ->
                time += 9_999
                assertEquals("image", cache.cachedAttachmentPreview(key)?.decodeToString())
                time++
                assertNull(cache.cachedAttachmentPreview(key))
            }
            assertEquals(1, f.server.requestCount)
        }
    }

    @Test fun theNetworkRevalidationCapDoesNotShortenTheStoredPreviewWindow() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            val key = "id:thumbnail"
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=600, stale-while-revalidate=30")
                .setHeader("ETag", "\"image\"").setBody("image"))
            f.server.enqueue(MockResponse().setResponseCode(304))
            f.cache(now = { time }).use { cache -> cache.signedImage(f.url, key) }
            f.cache(now = { time }).use { cache ->
                time += 360_000
                assertEquals("image", cache.cachedAttachmentPreview(key)?.decodeToString())
                assertEquals(1, f.server.requestCount)
                assertEquals("image", cache.signedImage(f.url, key).decodeToString())
                assertEquals(2, f.server.requestCount)
                time += 629_999
                assertEquals("image", cache.cachedAttachmentPreview(key)?.decodeToString())
                time++
                assertNull(cache.cachedAttachmentPreview(key))
            }
        }
    }

    @Test fun failedCapacityCleanupCanBeRetriedWithTheSameSize() = runBlocking {
        Fixture().use { f ->
            val cachePath = f.directory.resolve("account/images-attachment-bodies-v3")
            assertTrue(cachePath.parentFile.mkdirs())
            cachePath.writeText("unreadable as a directory")
            f.cache().use { cache ->
                assertFalse(cache.resizeAttachmentCache(0))
                assertTrue(cachePath.delete())
                assertTrue(cache.resizeAttachmentCache(0))
            }
        }
    }

    @Test fun refreshesLegacyEntriesOnceToLearnTheirPreviewPolicy() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            repeat(2) {
                f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60")
                    .setHeader("ETag", "\"image\"").setBody("image"))
            }
            val key = "legacy:thumbnail"
            f.cache(now = { time }).use { cache -> cache.signedImage(f.url, key) }
            val file = f.directory.resolve("account/images-attachment-bodies-v3")
                .resolve(AttachmentImageDiskCache.digest(key))
            val (hash, expiresAt, remainingHeaderAndBody) = java.io.DataInputStream(file.inputStream()).use {
                val hash = it.readUTF().removePrefix("http:preview:")
                val expiresAt = it.readLong()
                it.readLong()
                Triple(hash, expiresAt, it.readBytes())
            }
            java.io.DataOutputStream(file.outputStream()).use {
                it.writeUTF(hash)
                it.writeLong(expiresAt)
                it.write(remainingHeaderAndBody)
            }
            time += 60_000
            f.cache(now = { time }).use { cache ->
                assertNull(cache.cachedAttachmentPreview(key))
                assertEquals("image", cache.signedImage(f.url, key).decodeToString())
                time += TimeUnit.DAYS.toMillis(7)
                assertEquals("image", cache.cachedAttachmentPreview(key)?.decodeToString())
            }
            assertEquals(2, f.server.requestCount)
            f.server.takeRequest()
            assertNull(f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun resizingKeepsRecentlyUsedAttachmentsAndDisablingClearsTheCache() = runBlocking {
        Fixture().use { f ->
            val image = "x".repeat(512)
            repeat(3) { f.server.enqueue(MockResponse().setBody(image)) }
            f.cache().use { cache ->
                cache.signedImage(f.url, "keep:thumbnail")
                cache.signedImage(f.url, "evict:thumbnail")
                val directory = f.directory.resolve("account/images-attachment-bodies-v3")
                val old = directory.resolve(AttachmentImageDiskCache.digest("evict:thumbnail"))
                assertTrue(old.setLastModified(1_000L))
                assertEquals(image, cache.cachedAttachmentPreview("keep:thumbnail")?.decodeToString())

                cache.resizeAttachmentCache(1_000)
                assertNull(cache.cachedAttachmentPreview("evict:thumbnail"))
                assertEquals(image, cache.cachedAttachmentPreview("keep:thumbnail")?.decodeToString())
                cache.resizeAttachmentCache(0)
                assertNull(cache.cachedAttachmentPreview("keep:thumbnail"))
                assertEquals(image, cache.signedImage(f.url, "keep:thumbnail").decodeToString())
                assertNull(cache.cachedAttachmentPreview("keep:thumbnail"))
            }
            f.cache().use { cache ->
                assertNull(cache.cachedAttachmentPreview("keep:thumbnail"))
            }
            assertEquals(3, f.server.requestCount)
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

    @Test fun revocationDuringACacheHitPreventsReturningTheCachedImage() = runBlocking {
        Fixture().use { f ->
            val pause = AtomicBoolean(false)
            val afterReading = CountDownLatch(1)
            val finishReading = CountDownLatch(1)
            val key = "id:thumbnail"
            f.server.enqueue(MockResponse().setBody("cached"))
            f.cache(now = {
                if (pause.get()) {
                    afterReading.countDown()
                    check(finishReading.await(5, TimeUnit.SECONDS))
                }
                1_000L
            }).use { cache ->
                assertEquals("cached", cache.signedImage(f.url, key).decodeToString())
                pause.set(true)
                val pending = async {
                    assertFailsWith<ImageHttpFailure> { cache.signedImage(f.url, key) }
                }
                try {
                    assertTrue(withContext(Dispatchers.IO) { afterReading.await(5, TimeUnit.SECONDS) })
                    cache.removeAttachmentImages("id")
                } finally {
                    finishReading.countDown()
                }
                assertEquals(403, pending.await().status)
                assertNull(cache.cachedAttachmentPreview(key))
            }
            assertEquals(1, f.server.requestCount)
        }
    }

    @Test fun reusesAThumbnailAcrossClientRestartsUntilItExpires() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=300")
                .setHeader("ETag", "\"thumbnail\"").setHeader("Age", "30").setBody("thumbnail"))
            f.server.enqueue(MockResponse().setBody("updated"))
            val key = "id:thumbnail"
            f.cache(now = { time }).use { cache ->
                assertEquals("thumbnail", cache.signedImage("${f.url}?X-Amz-Signature=old", key).decodeToString())
            }
            f.cache(now = { time }).use { cache ->
                time += 269_999
                assertEquals("thumbnail", cache.signedImage("${f.url}?X-Amz-Signature=new", key).decodeToString())
                assertEquals(1, f.server.requestCount)
                time++
                assertEquals("updated", cache.signedImage("${f.url}?X-Amz-Signature=new", key).decodeToString())
            }
            assertEquals(2, f.server.requestCount)
            f.server.takeRequest()
            assertEquals("\"thumbnail\"", f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun cachesRedirectedThumbnailsWithoutSendingTheFinalEtagToTheEntryUrl() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            repeat(2) {
                f.server.enqueue(MockResponse().setResponseCode(307).setHeader("Location", "/final.png")
                    .setHeader("Cache-Control", "private, max-age=30"))
                f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60")
                    .setHeader("ETag", "\"final\"").setBody("thumbnail"))
            }
            f.cache(now = { time }).use { cache ->
                val key = "id:thumbnail"
                assertEquals("thumbnail", cache.signedImage("${f.url}?X-Amz-Signature=old", key).decodeToString())
                assertEquals("thumbnail", cache.signedImage("${f.url}?X-Amz-Signature=new", key).decodeToString())
                assertEquals(2, f.server.requestCount)
                time += 30_000
                assertEquals("thumbnail", cache.signedImage("${f.url}?X-Amz-Signature=new", key).decodeToString())
            }
            assertEquals(4, f.server.requestCount)
            repeat(4) { assertNull(f.server.takeRequest().getHeader("If-None-Match")) }
        }
    }

    @Test fun doesNotReuseTheFinalImageWhenARedirectCannotBeCached() = runBlocking {
        Fixture().use { f ->
            val conditions = listOf("Cache-Control" to "no-store", "Vary" to "*")
            f.cache().use { cache ->
                for ((name, value) in conditions) {
                    val key = "$name:thumbnail"
                    for (target in listOf("first", "second")) {
                        f.server.enqueue(MockResponse().setResponseCode(307)
                            .setHeader("Location", "/$target.png").setHeader(name, value))
                        f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=300").setBody(target))
                        assertEquals(target, cache.signedImage(f.url, key).decodeToString())
                    }
                }
            }
            assertEquals(8, f.server.requestCount)
        }
    }

    @Test fun retainsFreshnessAndValidatorAcrossFailureAndHeaderlessNotModified() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=300")
                .setHeader("ETag", "\"thumbnail\"").setBody("thumbnail"))
            f.server.enqueue(MockResponse().setResponseCode(503))
            f.server.enqueue(MockResponse().setResponseCode(304))
            f.server.enqueue(MockResponse().setBody("unexpected"))
            val key = "id:thumbnail"
            f.cache(now = { time }).use { cache ->
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
                time += 300_000
                assertEquals(503, assertFailsWith<ImageHttpFailure> { cache.signedImage(f.url, key) }.status)
            }
            f.cache(now = { time }).use { cache ->
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
            }
            f.cache(now = { time }).use { cache ->
                time += 299_999
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
            }
            assertEquals(3, f.server.requestCount)
            assertNull(f.server.takeRequest().getHeader("If-None-Match"))
            repeat(2) { assertEquals("\"thumbnail\"", f.server.takeRequest().getHeader("If-None-Match")) }
        }
    }

    @Test fun maxAgeTakesPrecedenceOverInvalidExpiresOnOkAndNotModified() = runBlocking {
        Fixture().use { f ->
            var time = 1_000L
            f.server.enqueue(MockResponse().setHeader("Cache-Control", "private, max-age=60")
                .setHeader("Expires", "0").setHeader("ETag", "\"thumbnail\"").setBody("thumbnail"))
            f.server.enqueue(MockResponse().setResponseCode(304).setHeader("Expires", "0"))
            f.server.enqueue(MockResponse().setBody("unexpected download"))
            val key = "id:thumbnail"
            f.cache(now = { time }).use { cache ->
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
            }
            f.cache(now = { time }).use { cache ->
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
                assertEquals(1, f.server.requestCount)
                time += 60_000
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
            }
            f.cache(now = { time }).use { cache ->
                assertEquals("thumbnail", cache.signedImage(f.url, key).decodeToString())
            }
            assertEquals(2, f.server.requestCount)
            f.server.takeRequest()
            assertEquals("\"thumbnail\"", f.server.takeRequest().getHeader("If-None-Match"))
        }
    }

    @Test fun signedAttachmentsSkipDiskForNoStoreNoCacheOrVary() = runBlocking {
        Fixture().use { f ->
            val policies = listOf(
                "Cache-Control" to "no-store",
                "Cache-Control" to "no-cache",
                "Vary" to "*",
                "Vary" to "Accept",
            )
            f.cache().use { cache ->
                for ((index, header) in policies.withIndex()) {
                    val (name, value) = header
                    val key = "$index:thumbnail"
                    f.server.enqueue(MockResponse().setHeader(name, value)
                        .setHeader("ETag", "\"variant\"").setBody("first"))
                    f.server.enqueue(MockResponse().setBody("second"))
                    assertEquals("first", cache.signedImage("${f.url}?X-Amz-Signature=old", key).decodeToString())
                    assertEquals("second", cache.signedImage("${f.url}?X-Amz-Signature=new", key).decodeToString())
                    assertNull(f.server.takeRequest().getHeader("If-None-Match"))
                    assertNull(f.server.takeRequest().getHeader("If-None-Match"))
                }
            }
            assertEquals(policies.size * 2, f.server.requestCount)
        }
    }
}
