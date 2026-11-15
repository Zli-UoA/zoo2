package zoo

import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Session-local LRU of encoded images; failures remain retryable. */
internal class ImageCache(
    scope: CoroutineScope,
    private val maxBytes: Int = 32 * 1024 * 1024,
    private val maxEntries: Int = 256,
) {
    private val job = SupervisorJob(scope.coroutineContext[Job])
    private val scope = CoroutineScope(scope.coroutineContext + job)
    private val mutex = Mutex()
    private val images = linkedMapOf<String, ByteArray>()
    private val pending = mutableMapOf<String, Deferred<ByteArray>>()
    private var bytes = 0

    suspend fun get(url: String, load: suspend () -> ByteArray): ByteArray {
        val request = mutex.withLock {
            images.remove(url)?.let {
                images[url] = it
                return it
            }
            pending.getOrPut(url) {
                scope.async(start = CoroutineStart.LAZY) {
                    try {
                        val image = load()
                        mutex.withLock {
                            if (image.size <= maxBytes && maxEntries > 0) {
                                images[url] = image
                                bytes += image.size
                                while (bytes > maxBytes || images.size > maxEntries) {
                                    bytes -= images.remove(images.keys.first())!!.size
                                }
                            }
                        }
                        image
                    } finally {
                        mutex.withLock { pending.remove(url) }
                    }
                }
            }
        }
        return request.await()
    }

    fun close() {
        job.cancel()
        images.clear()
        bytes = 0
    }
}
