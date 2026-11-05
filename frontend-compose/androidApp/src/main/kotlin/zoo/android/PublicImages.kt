package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.statement.readRawBytes
import io.ktor.http.*
import java.net.URI

/** The API client disables redirects; public images may legitimately use a CDN redirect. */
internal suspend fun loadPublicImage(http: HttpClient, url: String): ByteArray {
    var current = url
    repeat(6) {
        require(Url(current).protocol == URLProtocol.HTTPS)
        val response = http.get(current)
        if (response.status.value in setOf(301, 302, 303, 307, 308)) {
            val location = response.headers[HttpHeaders.Location] ?: error("Missing image redirect")
            response.readRawBytes()
            current = URI(current).resolve(location).toString()
        } else {
            check(response.status.value in 200..299) { "Image HTTP ${response.status.value}" }
            return response.readRawBytes()
        }
    }
    error("Too many image redirects")
}
