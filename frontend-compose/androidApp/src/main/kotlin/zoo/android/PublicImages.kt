package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.request.prepareGet
import io.ktor.client.request.header
import io.ktor.http.*
import io.ktor.client.statement.HttpResponse
import okhttp3.HttpUrl.Companion.toHttpUrl

/** The API client disables redirects; public images may legitimately use a CDN redirect. */
internal class ImageHttpFailure(val status: Int) : IllegalStateException("Image request failed: $status")

internal class ImageCacheMiss : IllegalStateException("Image is not in the disk cache")

internal suspend fun loadPublicImage(
    http: HttpClient, url: String, limit: Int = 20 * 1024 * 1024,
    cacheOnly: Boolean = false, restrictOrigin: Boolean = false, noStore: Boolean = false, revalidate: Boolean = false, bypassCache: Boolean = false,
    conditional: AttachmentImageDiskCache.Entry? = null,
    onSignedResponse: ((HttpResponse, ByteArray, Boolean) -> Unit)? = null,
): ByteArray {
    var current = url
    repeat(6) { hop ->
        val destination = Url(current)
        require(destination.protocol == URLProtocol.HTTPS)
        require(!restrictOrigin || sameImageOrigin(Url(url), destination)) { "Image proxy redirected outside its origin" }
        var bytes: ByteArray? = null
        http.prepareGet(current) {
            if (hop == 0) conditional?.etag?.let { header(HttpHeaders.IfNoneMatch, it) }
            when {
                noStore -> header(HttpHeaders.CacheControl, "no-store, no-cache")
                bypassCache -> header(HttpHeaders.CacheControl, "no-cache")
                revalidate -> header(HttpHeaders.CacheControl, "max-age=0")
                cacheOnly -> header(HttpHeaders.CacheControl, "only-if-cached, max-stale=86400")
            }
        }.execute { response ->
            if (response.status.value in setOf(301, 302, 303, 307, 308)) {
                val location = response.headers[HttpHeaders.Location] ?: error("Missing image redirect")
                current = current.toHttpUrl().resolve(location)?.toString() ?: error("Invalid image redirect")
            } else {
                val image = if (hop == 0 && response.status == HttpStatusCode.NotModified && conditional?.etag != null) {
                    conditional.bytes
                } else {
                    if (response.status.value !in 200..299) {
                        if (cacheOnly) throw ImageCacheMiss()
                        throw ImageHttpFailure(response.status.value)
                    }
                    readImageResponse(response, limit)
                }
                bytes = image
                onSignedResponse?.invoke(response, image, hop == 0)
            }
        }
        bytes?.let { return it }
    }
    error("Too many image redirects")
}
