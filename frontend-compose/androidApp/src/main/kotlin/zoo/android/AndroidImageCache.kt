package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.Cache
import okhttp3.Dispatcher
import okhttp3.OkHttpClient

/** Account-local persistent HTTP cache. Public media never receives API authentication. */
internal class AndroidImageCache(
    directory: File,
    maxBytes: Long = 512L * 1024 * 1024,
    private val timeoutMillis: Long = 120_000,
    configure: OkHttpClient.Builder.() -> Unit = {},
) : java.io.Closeable {
    private val attachments = AttachmentImageDiskCache(directory.resolveSibling("${directory.name}-attachment-bodies"))
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

    // Callers supplying a stable key resolve an authorized link before looking up the image.
    suspend fun signedImage(url: String, cacheKey: String? = null, retry: Boolean = false): ByteArray {
        if (cacheKey == null) return loadPublicImage(http, url, noStore = true)
        return withContext(Dispatchers.IO) {
            if (retry) attachments.remove(cacheKey)
            else attachments.read(cacheKey)?.let { return@withContext it }
            loadPublicImage(http, url, noStore = true) { attachments.write(cacheKey, it) }
        }
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
