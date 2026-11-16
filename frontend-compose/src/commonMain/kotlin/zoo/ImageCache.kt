package zoo

import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Session-local weighted LRU; concurrent consumers share loads and failures remain retryable. */
internal class AssetCache<T>(
    scope: CoroutineScope,
    private val maxBytes: Int = 32 * 1024 * 1024,
    private val maxEntries: Int = 256,
    private val weight: (T) -> Int,
) {
    private val job = SupervisorJob(scope.coroutineContext[Job])
    private val scope = CoroutineScope(scope.coroutineContext + job)
    private val mutex = Mutex()
    private val images = linkedMapOf<String, T>()
    private val pending = mutableMapOf<String, Deferred<T>>()
    private var bytes = 0
    private var closed = false

    suspend fun get(url: String, load: suspend () -> T): T {
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
                            if (!closed && weight(image) <= maxBytes && maxEntries > 0) {
                                images[url] = image
                                bytes += weight(image)
                                while (bytes > maxBytes || images.size > maxEntries) {
                                    bytes -= weight(images.remove(images.keys.first())!!)
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
        closed = true
        job.cancel()
        images.clear()
        pending.clear()
        bytes = 0
    }
}

/** Encoded images and decoded bitmaps have separate memory budgets. */
internal class ImageCache(scope: CoroutineScope, maxBytes: Int = 32 * 1024 * 1024, maxEntries: Int = 256) {
    private val cache = AssetCache<ByteArray>(scope, maxBytes, maxEntries) { it.size }
    suspend fun get(url: String, load: suspend () -> ByteArray): ByteArray = cache.get(url, load)
    fun close() = cache.close()
}
