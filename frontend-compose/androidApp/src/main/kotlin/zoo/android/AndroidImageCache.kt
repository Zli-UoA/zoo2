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
            requestTimeoutMillis = 120_000
            socketTimeoutMillis = 90_000
        }
    }

    suspend fun image(url: String): ByteArray = loadPublicImage(http, url)

    override fun close() {
        http.close()
        disk.close()
        client.connectionPool.evictAll()
        client.dispatcher.executorService.shutdown()
    }
}
