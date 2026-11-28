package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.request.prepareGet
import io.ktor.client.request.header
import io.ktor.http.*
import okhttp3.HttpUrl.Companion.toHttpUrl

private val signatureParameters = setOf(
    "x-amz-algorithm", "x-amz-credential", "x-amz-date", "x-amz-expires",
    "x-amz-signedheaders", "x-amz-signature", "x-amz-security-token",
)

private fun imageResponseHash(url: String, signed: Boolean): String {
    val parsed = url.toHttpUrl()
    val resource = parsed.newBuilder().fragment(null)
    if (signed) {
        parsed.queryParameterNames.filter { it.lowercase() in signatureParameters }
            .forEach(resource::removeAllQueryParameters)
    }
    return AttachmentImageDiskCache.digest(resource.build().toString())
}

/** The API client disables redirects; public images may legitimately use a CDN redirect. */
internal class ImageHttpFailure(val status: Int) : IllegalStateException("Image request failed: $status")

internal suspend fun loadPublicImage(
    http: HttpClient, url: String, limit: Int = 20 * 1024 * 1024,
    restrictOrigin: Boolean = false, noStore: Boolean = false,
    conditional: AttachmentImageDiskCache.Entry? = null,
    onResponse: ((AttachmentImageDiskCache.Entry) -> Unit)? = null,
): ByteArray {
    var current = url
    repeat(6) {
        val destination = Url(current)
        require(destination.protocol == URLProtocol.HTTPS)
        require(!restrictOrigin || sameImageOrigin(Url(url), destination)) { "Image proxy redirected outside its origin" }
        val urlHash = imageResponseHash(current, signed = noStore)
        val validator = conditional?.takeIf {
            it.responseUrlHash == urlHash || it.responseUrlHash == AttachmentImageDiskCache.digest(current)
        }
        var bytes: ByteArray? = null
        http.prepareGet(current) {
            validator?.etag?.let { header(HttpHeaders.IfNoneMatch, it) }
            if (validator?.etag == null) validator?.lastModified?.let { header(HttpHeaders.IfModifiedSince, it) }
            if (noStore) header(HttpHeaders.CacheControl, "no-store, no-cache")
            else if (conditional != null) header(HttpHeaders.CacheControl, "no-cache")
        }.execute { response ->
            if (response.status.value in setOf(301, 302, 303, 307, 308)) {
                val location = response.headers[HttpHeaders.Location] ?: error("Missing image redirect")
                current = current.toHttpUrl().resolve(location)?.toString() ?: error("Invalid image redirect")
            } else {
                val unchanged = response.status == HttpStatusCode.NotModified
                val image = if (unchanged && validator != null &&
                    (validator.etag != null || validator.lastModified != null)
                ) {
                    validator.bytes
                } else {
                    if (response.status.value !in 200..299) throw ImageHttpFailure(response.status.value)
                    readImageResponse(response, limit)
                }
                bytes = image
                val entry = AttachmentImageDiskCache.Entry(
                    urlHash,
                    response.headers[HttpHeaders.ETag]?.takeIf { it.length <= 1024 }
                        ?: validator?.etag?.takeIf { unchanged },
                    response.headers[HttpHeaders.LastModified]?.takeIf { it.length <= 128 }
                        ?: validator?.lastModified?.takeIf { unchanged },
                    image,
                )
                if (!unchanged || entry != validator) onResponse?.invoke(entry)
            }
        }
        bytes?.let { return it }
    }
    error("Too many image redirects")
}
