package zoo.android

import android.content.Context
import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.statement.bodyAsText
import io.ktor.http.URLProtocol
import io.ktor.http.Url
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import zoo.endpoint
import zoo.proxyImageUrl
import zoo.publicUrl

private val proxyDownloads = Semaphore(2)
private val directImageHosts = setOf(
    "attachments.zliusercontent.com",
    "avater.zliusercontent.com",
    "emoji.zliusercontent.com",
    "emoji.zli.works",
)

internal suspend fun <T> downloadThroughProxy(push: Boolean, load: suspend () -> T): T {
    if (!push) return proxyDownloads.withPermit { load() }
    check(proxyDownloads.tryAcquire()) { "Image proxy is busy" }
    try { return load() } finally { proxyDownloads.release() }
}

internal suspend fun <T : Any> downloadProxyImage(push: Boolean, load: suspend () -> T): T =
    withTimeoutOrNull(if (push) 2_500L else 15_000L) {
        try {
            downloadThroughProxy(push, load)
        } catch (failure: ImageHttpFailure) {
            if (failure.status != 429 && failure.status !in 500..599) throw failure
            delay(350)
            downloadThroughProxy(push, load)
        }
    } ?: error("Image proxy download timed out")

internal data class ImageRequest(val url: String, val proxied: Boolean = false)

internal fun sameImageOrigin(left: Url, right: Url) =
    left.protocol == right.protocol && left.host == right.host && left.port == right.port

internal fun normalizedImageSource(base: String, source: String): Url? {
    if (source.startsWith("//")) return null
    return publicUrl(if (source.startsWith('/')) endpoint(base, source) else source)
}

internal fun Url.isDedicatedImageOrigin(): Boolean =
    protocol == URLProtocol.HTTPS && port == URLProtocol.HTTPS.defaultPort && host in directImageHosts

internal fun imageRequest(base: String, source: String, proxy: String): ImageRequest? {
    val url = normalizedImageSource(base, source) ?: return null
    val workspaceOrigin = publicUrl(base)?.let { sameImageOrigin(it, url) } == true
    if (url.protocol == URLProtocol.HTTPS && (workspaceOrigin || url.isDedicatedImageOrigin())) {
        return ImageRequest(url.toString())
    }
    val proxyEndpoint = if (proxy.startsWith('/') && !proxy.startsWith("//")) endpoint(base, proxy) else proxy
    return proxyImageUrl(proxyEndpoint, url.toString())
        ?.takeIf { publicUrl(it)?.protocol?.name == "https" }
        ?.let { ImageRequest(it, proxied = true) }
}

/** Resolve the deployment's proxy once across concurrent images and retain it for cached reads after restart. */
internal class AndroidMediaProxy(context: Context, private val base: String, private val clock: () -> Long = System::currentTimeMillis) {
    private val prefs = context.getSharedPreferences("media-proxy", Context.MODE_PRIVATE)
    private val lock = Mutex()
    private var retryAfter = 0L

    suspend fun image(
        source: String, http: HttpClient, cache: AndroidImageCache,
        online: Boolean = true, limit: Int = 20 * 1024 * 1024, push: Boolean = false, bypassCache: Boolean = false,
    ): ByteArray {
        val cacheSource = normalizedImageSource(base, source)?.toString() ?: error("Invalid image URL")
        if (!bypassCache) cache.cachedPublicImage(cacheSource, limit)?.let { return it }
        suspend fun resolve(url: String): ImageRequest {
            if (!push) return request(url, http, online)
            val saved = imageRequest(base, url, prefs.getString(base, "").orEmpty())
            return withTimeoutOrNull(750) { request(url, http, online, timeoutMillis = 700) }
                ?: requireNotNull(saved) { "Image proxy lookup timed out" }
        }
        val route = resolve(cacheSource)
        if (route.proxied) return downloadProxyImage(push) { cache.download(route, limit, bypassCache, cacheSource) }
        return cache.download(route, limit, bypassCache, cacheSource, redirectRoute = ::resolve, push = push)
    }

    internal suspend fun request(source: String, http: HttpClient, online: Boolean, timeoutMillis: Long = 5_000): ImageRequest {
        imageRequest(base, source, "")?.let { return it }
        val proxy = lock.withLock {
            val now = clock()
            val saved = prefs.getString(base, "").orEmpty()
            if (!online || now < retryAfter || (saved.isNotEmpty() && now - prefs.getLong("$base:checked", 0L) < 24 * 60 * 60 * 1000L)) {
                return@withLock saved
            }
            val lookup = try {
                withTimeoutOrNull(timeoutMillis) {
                    val response = http.get(endpoint(base, "/media-proxy-endpoint"))
                    val endpoint = if (response.status.value == 404) ""
                    else if (response.status.value == 200 && response.headers["Content-Type"].orEmpty().startsWith("text/plain")) {
                        response.bodyAsText().trim()
                    } else null
                    Result.success(endpoint)
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                Result.failure<String?>(failure)
            }
            val configured = lookup?.getOrNull()
            if (configured == null) {
                retryAfter = now + 30_000
                require(lookup != null || saved.isNotBlank()) { "Image proxy lookup timed out" }
                saved
            } else {
                if (configured.isBlank()) retryAfter = now + 30_000
                prefs.edit().putString(base, configured).putLong("$base:checked", now).apply()
                configured
            }
        }
        return requireNotNull(imageRequest(base, source, proxy)) { "Image media-proxy is unavailable" }
    }
}
