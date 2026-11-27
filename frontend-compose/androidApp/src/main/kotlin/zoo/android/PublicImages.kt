package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.request.prepareGet
import io.ktor.http.*
import okhttp3.CacheControl
import okhttp3.Headers
import okhttp3.HttpUrl.Companion.toHttpUrl

/** The API client disables redirects; public images may legitimately use a CDN redirect. */
internal suspend fun loadPublicImage(
    http: HttpClient,
    url: String,
    limit: Int = 20 * 1024 * 1024,
    onCacheable: ((ByteArray) -> Unit)? = null,
): ByteArray {
    var current = url
    repeat(6) {
        require(Url(current).protocol == URLProtocol.HTTPS)
        var bytes: ByteArray? = null
        http.prepareGet(current).execute { response ->
            if (response.status.value in setOf(301, 302, 303, 307, 308)) {
                val location = response.headers[HttpHeaders.Location] ?: error("Missing image redirect")
                current = current.toHttpUrl().resolve(location)?.toString() ?: error("Invalid image redirect")
            } else {
                bytes = readImageResponse(response, limit).also { image ->
                    if (onCacheable != null) {
                        val policy = CacheControl.parse(Headers.Builder().apply {
                            for (name in listOf(HttpHeaders.CacheControl, HttpHeaders.Pragma)) {
                                response.headers.getAll(name).orEmpty().forEach { add(name, it) }
                            }
                        }.build())
                        if (!policy.noStore && !policy.noCache && policy.maxAgeSeconds != 0) onCacheable(image)
                    }
                }
            }
        }
        bytes?.let { return it }
    }
    error("Too many image redirects")
}
