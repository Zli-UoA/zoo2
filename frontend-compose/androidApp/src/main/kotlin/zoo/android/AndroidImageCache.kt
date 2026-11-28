package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.statement.HttpResponse
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.fromHttpToGmtDate
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.Cache
import okhttp3.CacheControl
import okhttp3.Headers
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Dispatcher
import okhttp3.OkHttpClient
import zoo.android.AttachmentImageDiskCache.Freshness

/** Account-local persistent HTTP cache. Public media never receives API authentication. */
internal class AndroidImageCache(
    directory: File,
    maxBytes: Long = 512L * 1024 * 1024,
    private val timeoutMillis: Long = 120_000,
    private val now: () -> Long = System::currentTimeMillis,
    attachmentCacheBytes: Long = 512L * 1024 * 1024,
    configure: OkHttpClient.Builder.() -> Unit = {},
) : java.io.Closeable {
    private val attachments = AttachmentImageDiskCache(directory.resolveSibling("${directory.name}-attachment-bodies-v3"), attachmentCacheBytes)
    private val disk = Cache(directory, maxBytes)
    private val client = OkHttpClient.Builder()
        .apply(configure)
        .cache(disk)
        .followRedirects(false)
        .followSslRedirects(false)
        .build()
    // Cache-only reads must be able to run while the network dispatcher is full.
    private val cacheReader = client.newBuilder().dispatcher(Dispatcher()).build()
    private val http = imageHttpClient(client)
    private val cacheHttp = imageHttpClient(cacheReader)

    private fun imageHttpClient(client: OkHttpClient) = HttpClient(OkHttp) {
        // loadPublicImage validates every redirect, including HTTPS-only destinations.
        followRedirects = false
        engine {
            preconfigured = client
            // Ktor replaces the dispatcher before applying this configuration.
            config { dispatcher(client.dispatcher) }
        }
        install(HttpTimeout) {
            requestTimeoutMillis = timeoutMillis
            socketTimeoutMillis = minOf(timeoutMillis, 90_000)
        }
    }

    suspend fun image(url: String): ByteArray = loadPublicImage(http, url)

    suspend fun cachedAttachmentPreview(cacheKey: String): ByteArray? = withContext(Dispatchers.IO) {
        attachments.read(cacheKey, previewAt = now())?.bytes
    }

    suspend fun resizeAttachmentCache(bytes: Long) = withContext(Dispatchers.IO) {
        attachments.resize(bytes)
    }

    suspend fun removeAttachmentImages(id: String) = withContext(Dispatchers.IO) {
        attachments.revoke("$id:thumbnail")
        attachments.revoke("$id:original")
    }

    suspend fun allowAttachmentImages(id: String) = withContext(Dispatchers.IO) {
        attachments.allow("$id:thumbnail")
        attachments.allow("$id:original")
    }

    // Network loads still use an authorized link.
    suspend fun signedImage(url: String, cacheKey: String? = null, retry: Boolean = false): ByteArray {
        if (cacheKey == null) return loadPublicImage(http, url, noStore = true)
        return withContext(Dispatchers.IO) {
            val token = attachments.beginLoad(cacheKey) ?: throw ImageHttpFailure(403)
            try {
                val urlHash = AttachmentImageDiskCache.digest(url)
                val signedUrl = url.toHttpUrl()
                val resource = signedUrl.newBuilder().fragment(null).apply {
                    signedUrl.queryParameterNames
                        .filter { it.startsWith("X-Amz-", ignoreCase = true) }
                        .forEach { removeAllQueryParameters(it) }
                }.build()
                val resourceHash = AttachmentImageDiskCache.digest(resource.toString())
                val stored = if (retry) null else attachments.read(cacheKey)
                val matching = stored?.takeIf { it.urlHash.startsWith("$resourceHash:") }
                if (matching != null && now() < matching.revalidateAt &&
                    (cacheKey.endsWith(":thumbnail") || matching.urlHash == "$resourceHash:$urlHash")
                ) {
                    if (!attachments.isCurrent(cacheKey, token)) throw ImageHttpFailure(403)
                    return@withContext matching.bytes
                }
                if (retry) attachments.remove(cacheKey, token)
                // Legacy entries require a full response once to learn whether stale previews are permitted.
                val conditional = matching?.takeIf { it.freshness.stalePreviewMillis != null }
                val image = try {
                    loadPublicImage(http, url, noStore = true, conditional = conditional) { response, bytes, redirects ->
                        val receivedAt = now()
                        val validated = response.status == HttpStatusCode.NotModified
                        val freshness = attachmentFreshness(response, matching?.freshness?.takeIf { validated })
                        val redirectDeadlines = redirects.map { redirect ->
                            attachmentFreshness(redirect)?.let {
                                val elapsed = (response.responseTime.timestamp - redirect.responseTime.timestamp).coerceAtLeast(0)
                                attachmentFreshUntil(redirect, it, receivedAt - elapsed, defaultLifetime = 0L)
                            }
                        }
                        if (freshness == null || redirectDeadlines.any { it == null || it <= receivedAt }) {
                            attachments.remove(cacheKey, token)
                            return@loadPublicImage
                        }
                        val responseDeadline = attachmentFreshUntil(response, freshness, receivedAt, defaultLifetime = 300_000L)
                        val expiresAt = minOf(responseDeadline, redirectDeadlines.filterNotNull().minOrNull() ?: responseDeadline)
                        val etag = if (redirects.isEmpty()) {
                            response.headers[HttpHeaders.ETag]?.takeIf { it.length <= 1024 }
                                ?: matching?.etag?.takeIf { validated }
                        } else null
                        attachments.write(cacheKey, token, AttachmentImageDiskCache.Entry(
                            "$resourceHash:$urlHash", expiresAt, minOf(expiresAt, receivedAt + 300_000L),
                            etag, if (redirects.isEmpty()) freshness else freshness.copy(stalePreviewMillis = 0L), bytes,
                        ))
                    }
                } catch (failure: ImageHttpFailure) {
                    if (failure.status in setOf(401, 403, 404, 410)) attachments.remove(cacheKey, token)
                    throw failure
                }
                if (!attachments.isCurrent(cacheKey, token)) throw ImageHttpFailure(403)
                image
            } finally {
                attachments.endLoad(cacheKey, token)
            }
        }
    }

    private fun attachmentFreshness(response: HttpResponse, previous: Freshness? = null): Freshness? {
        val headers = response.headers
        val policy = CacheControl.parse(Headers.Builder().apply {
            for (name in listOf(HttpHeaders.CacheControl, HttpHeaders.Pragma)) {
                headers.getAll(name).orEmpty().forEach { add(name, it) }
            }
        }.build())
        if (policy.noStore || policy.noCache || headers[HttpHeaders.Vary] != null) return null
        val maxAge = when {
            policy.maxAgeSeconds >= 0 -> policy.maxAgeSeconds * 1000L
            headers[HttpHeaders.CacheControl] == null -> previous?.maxAgeMillis
            else -> null
        }
        val expiresHeader = headers[HttpHeaders.Expires]
        val expiresAt = if (expiresHeader == null) previous?.httpExpiresAt else httpDate(expiresHeader)
        if (expiresHeader != null && expiresAt == null && maxAge == null) return null
        val freshness = maxAge ?: 0L.takeIf { policy.mustRevalidate && expiresAt == null }
        val staleMillis = when {
            policy.mustRevalidate -> 0L
            headers[HttpHeaders.CacheControl] == null && previous != null -> previous.stalePreviewMillis
            else -> stalePreviewMillis(headers.getAll(HttpHeaders.CacheControl).orEmpty())
        }
        return Freshness(freshness, expiresAt, staleMillis)
    }

    private fun stalePreviewMillis(headers: List<String>): Long =
        headers.flatMap { it.split(',') }.map(String::trim)
            .filter { it.substringBefore('=').trim().equals("stale-while-revalidate", ignoreCase = true) }
            .minOfOrNull { directive ->
                directive.substringAfter('=', "").trim().trim('"').toLongOrNull()
                    ?.coerceIn(0L, Int.MAX_VALUE.toLong())?.times(1000L) ?: 0L
            } ?: Long.MAX_VALUE

    private fun httpDate(value: String): Long? =
        runCatching { value.fromHttpToGmtDate().timestamp }.getOrNull()

    private fun attachmentFreshUntil(
        response: HttpResponse, freshness: Freshness, receivedAt: Long, defaultLifetime: Long,
    ): Long {
        val headers = response.headers
        val serverDate = headers[HttpHeaders.Date]?.let(::httpDate) ?: receivedAt
        val lifetime = freshness.maxAgeMillis ?: freshness.httpExpiresAt?.minus(serverDate) ?: defaultLifetime
        val age = maxOf(
            (receivedAt - serverDate).coerceAtLeast(0),
            (headers[HttpHeaders.Age]?.toLongOrNull() ?: 0L).coerceIn(0, Int.MAX_VALUE.toLong()) * 1000L,
        )
        return receivedAt + lifetime - age
    }

    suspend fun imageIfCached(request: ImageRequest, limit: Int): ByteArray? =
        try {
            loadPublicImage(cacheHttp, request.url, limit, cacheOnly = true, restrictOrigin = request.proxied)
        } catch (_: ImageCacheMiss) {
            null
        }

    suspend fun download(request: ImageRequest, limit: Int, bypassCache: Boolean = false): ByteArray =
        loadPublicImage(http, request.url, limit, restrictOrigin = request.proxied,
            revalidate = !bypassCache, bypassCache = bypassCache)

    suspend fun imageFromCacheFirst(request: ImageRequest, limit: Int = 20 * 1024 * 1024): ByteArray =
        imageIfCached(request, limit) ?: download(request, limit)

    override fun close() {
        attachments.close()
        http.close()
        cacheHttp.close()
        disk.close()
        client.connectionPool.evictAll()
        client.dispatcher.executorService.shutdown()
        cacheReader.dispatcher.executorService.shutdown()
    }
}
