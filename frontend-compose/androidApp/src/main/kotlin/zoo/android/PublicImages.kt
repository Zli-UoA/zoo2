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
    withHostSlot: (suspend (String, suspend () -> Unit) -> Unit)? = null,
    redirectRoute: (suspend (String) -> ImageRequest)? = null,
    push: Boolean = false,
    onResponse: ((AttachmentImageDiskCache.Entry) -> Unit)? = null,
): ByteArray {
    suspend fun fetch(start: String, maxRequests: Int, restrict: Boolean): ByteArray {
        var current = start
        var classifyRedirects = false
        repeat(maxRequests) { hop ->
            val destination = Url(current)
            require(destination.protocol == URLProtocol.HTTPS)
            require(!restrict || sameImageOrigin(Url(start), destination)) { "Image proxy redirected outside its origin" }
            val urlHash = imageResponseHash(current, signed = noStore)
            val validator = conditional?.takeIf {
                it.responseUrlHash == urlHash || it.responseUrlHash == AttachmentImageDiskCache.digest(current)
            }
            var bytes: ByteArray? = null
            var redirected: String? = null
            val receive: suspend () -> Unit = {
                http.prepareGet(current) {
                    validator?.etag?.let { header(HttpHeaders.IfNoneMatch, it) }
                    if (validator?.etag == null) validator?.lastModified?.let { header(HttpHeaders.IfModifiedSince, it) }
                    if (noStore) header(HttpHeaders.CacheControl, "no-store, no-cache")
                    else if (conditional != null) header(HttpHeaders.CacheControl, "no-cache")
                }.execute { response ->
                    if (response.status.value in setOf(301, 302, 303, 307, 308)) {
                        val location = response.headers[HttpHeaders.Location] ?: error("Missing image redirect")
                        val next = current.toHttpUrl().resolve(location) ?: error("Invalid image redirect")
                        require(next.isHttps) { "Image redirect requires HTTPS" }
                        redirected = next.toString()
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
            }
            if (withHostSlot == null) receive() else withHostSlot(destination.host.lowercase(), receive)
            bytes?.let { return it }
            val next = checkNotNull(redirected)
            if (hop + 1 < maxRequests) {
                classifyRedirects = classifyRedirects || destination.isDedicatedImageOrigin()
                val route = if (!restrict && classifyRedirects) redirectRoute?.invoke(next) else null
                val nextRequest = route ?: ImageRequest(next)
                if (nextRequest.proxied) {
                    return downloadProxyImage(push) { fetch(nextRequest.url, maxRequests - hop - 1, restrict = true) }
                }
                current = nextRequest.url
            }
        }
        error("Too many image redirects")
    }
    return fetch(url, 6, restrictOrigin)
}
