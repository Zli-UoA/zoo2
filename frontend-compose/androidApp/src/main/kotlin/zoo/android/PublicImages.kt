package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.request.prepareGet
import io.ktor.client.request.header
import io.ktor.http.*
import okhttp3.HttpUrl.Companion.toHttpUrl

/** The API client disables redirects; public images may legitimately use a CDN redirect. */
internal class ImageCacheMiss : IllegalStateException("Image is not in the disk cache")

internal suspend fun loadPublicImage(
    http: HttpClient, url: String, limit: Int = 20 * 1024 * 1024,
    cacheOnly: Boolean = false, restrictOrigin: Boolean = false,
): ByteArray {
    var current = url
    repeat(6) {
        val destination = Url(current)
        require(destination.protocol == URLProtocol.HTTPS)
        require(!restrictOrigin || sameImageOrigin(Url(url), destination)) { "Image proxy redirected outside its origin" }
        var bytes: ByteArray? = null
        http.prepareGet(current) {
            if (cacheOnly) header(HttpHeaders.CacheControl, "only-if-cached, max-stale=86400")
        }.execute { response ->
            if (cacheOnly && response.status.value == 504) throw ImageCacheMiss()
            if (response.status.value in setOf(301, 302, 303, 307, 308)) {
                val location = response.headers[HttpHeaders.Location] ?: error("Missing image redirect")
                current = current.toHttpUrl().resolve(location)?.toString() ?: error("Invalid image redirect")
            } else {
                bytes = readImageResponse(response, limit)
            }
        }
        bytes?.let { return it }
    }
    error("Too many image redirects")
}
