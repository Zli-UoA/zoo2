package zoo.android

import android.content.Context
import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.statement.bodyAsText
import io.ktor.http.Url
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import zoo.endpoint
import zoo.proxyImageUrl
import zoo.publicUrl

internal data class ImageRequest(val url: String, val proxied: Boolean = false)

internal fun sameImageOrigin(left: Url, right: Url) =
    left.protocol == right.protocol && left.host == right.host && left.port == right.port

internal fun imageRequest(base: String, source: String, proxy: String): ImageRequest? {
    if (source.startsWith("//")) return null
    val url = publicUrl(if (source.startsWith('/')) endpoint(base, source) else source) ?: return null
    if (url.protocol.name != "https") return null
    if (publicUrl(base)?.let { sameImageOrigin(it, url) } == true) return ImageRequest(url.toString())
    val proxyEndpoint = if (proxy.startsWith('/') && !proxy.startsWith("//")) endpoint(base, proxy) else proxy
    return proxyImageUrl(proxyEndpoint, url.toString())
        ?.takeIf { publicUrl(it)?.protocol?.name == "https" }
        ?.let { ImageRequest(it, proxied = true) }
}

/** Resolve the deployment's proxy once across concurrent images and retain it for cached reads after restart. */
internal class AndroidMediaProxy(context: Context, private val base: String) {
    private val prefs = context.getSharedPreferences("media-proxy", Context.MODE_PRIVATE)
    private val lock = Mutex()
    private var retryAfter = 0L

    suspend fun image(
        source: String, http: HttpClient, cache: AndroidImageCache,
        online: Boolean = true, limit: Int = 20 * 1024 * 1024,
    ): ByteArray {
        val cachedRoute = imageRequest(base, source, prefs.getString(base, "").orEmpty())
        if (cachedRoute != null) cache.imageIfCached(cachedRoute, limit)?.let { return it }
        val route = request(source, http, online)
        return if (route == cachedRoute) cache.download(route, limit) else cache.imageFromCacheFirst(route, limit)
    }

    private suspend fun request(source: String, http: HttpClient, online: Boolean): ImageRequest {
        imageRequest(base, source, "")?.let { return it }
        val proxy = lock.withLock {
            val now = System.currentTimeMillis()
            val saved = prefs.getString(base, "").orEmpty()
            if (!online || now < retryAfter || now - prefs.getLong("$base:checked", 0L) < 24 * 60 * 60 * 1000L) {
                return@withLock saved
            }
            val configured = try {
                withTimeoutOrNull(5_000) {
                    val response = http.get(endpoint(base, "/media-proxy-endpoint"))
                    if (response.status.value == 404) ""
                    else if (response.status.value == 200 && response.headers["Content-Type"].orEmpty().startsWith("text/plain")) {
                        response.bodyAsText().trim()
                    } else null
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                null
            }
            if (configured == null) {
                retryAfter = now + 30_000
                saved
            } else {
                prefs.edit().putString(base, configured).putLong("$base:checked", now).apply()
                configured
            }
        }
        return requireNotNull(imageRequest(base, source, proxy)) { "Image media-proxy is unavailable" }
    }
}
