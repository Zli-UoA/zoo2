package zoo.android

import io.ktor.client.statement.*
import io.ktor.http.*
import io.ktor.utils.io.*
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

/** Enforce the limit while streaming, including chunked responses with no Content-Length. */
internal suspend fun readImageResponse(response: HttpResponse, limit: Int = 20 * 1024 * 1024): ByteArray {
    check(response.status.value in 200..299) { "Image request failed: ${response.status.value}" }
    require(response.contentLength()?.let { it <= limit } != false) { "Image is too large" }
    val channel = response.bodyAsChannel()
    val output = ByteArrayOutputStream()
    val buffer = ByteArray(8192)
    while (true) {
        currentCoroutineContext().ensureActive()
        val count = channel.readAvailable(buffer)
        if (count == -1) break
        require(count <= limit - output.size()) { "Image is too large" }
        output.write(buffer, 0, count)
    }
    return output.toByteArray()
}
