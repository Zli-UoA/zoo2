package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import java.io.File
import okhttp3.Cache
import okhttp3.OkHttpClient

/** Account-local persistent HTTP cache. Public media never receives API authentication. */
internal class AndroidImageCache(
    directory: File,
    maxBytes: Long = 512L * 1024 * 1024,
    private val timeoutMillis: Long = 120_000,
    configure: OkHttpClient.Builder.() -> Unit = {},
) : java.io.Closeable {
    private val disk = Cache(directory, maxBytes)
    private val client = OkHttpClient.Builder()
        .apply(configure)
        .cache(disk)
        .followRedirects(false)
        .followSslRedirects(false)
        .build()
    private val http = HttpClient(OkHttp) {
        // loadPublicImage validates every redirect, including HTTPS-only destinations.
        followRedirects = false
        engine { preconfigured = client }
        install(HttpTimeout) {
            requestTimeoutMillis = timeoutMillis
            socketTimeoutMillis = minOf(timeoutMillis, 90_000)
        }
    }

    suspend fun image(url: String): ByteArray = loadPublicImage(http, url)

    suspend fun signedImage(url: String): ByteArray = loadPublicImage(http, url, noStore = true)

    suspend fun imageIfCached(request: ImageRequest, limit: Int): ByteArray? =
        try {
            loadPublicImage(http, request.url, limit, cacheOnly = true, restrictOrigin = request.proxied)
        } catch (_: ImageCacheMiss) {
            null
        }

    suspend fun download(request: ImageRequest, limit: Int): ByteArray =
        loadPublicImage(http, request.url, limit, restrictOrigin = request.proxied, revalidate = true)

    suspend fun imageFromCacheFirst(request: ImageRequest, limit: Int = 20 * 1024 * 1024): ByteArray =
        imageIfCached(request, limit) ?: download(request, limit)

    override fun close() {
        http.close()
        disk.close()
        client.connectionPool.evictAll()
        client.dispatcher.executorService.shutdown()
    }
}
