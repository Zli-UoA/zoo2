package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class ImageCacheTest {
    @Test fun sharesLoadsAndKeepsThemAliveWhenOneConsumerLeaves() = runTest {
        val cache = ImageCache(backgroundScope)
        val loaded = CompletableDeferred<ByteArray>()
        var requests = 0
        suspend fun load(): ByteArray { requests++; return loaded.await() }
        val first = async { cache.get("image", ::load) }
        val second = async { cache.get("image", ::load) }
        runCurrent()
        assertEquals(1, requests)
        first.cancelAndJoin()
        loaded.complete(byteArrayOf(1, 2))
        assertContentEquals(byteArrayOf(1, 2), second.await())
        assertContentEquals(byteArrayOf(1, 2), cache.get("image") { error("Already cached") })
        cache.close()
    }

    @Test fun evictsLeastRecentlyUsedAndDoesNotKeepOversizedImages() = runTest {
        val cache = ImageCache(backgroundScope, maxBytes = 4, maxEntries = 2)
        cache.get("a") { byteArrayOf(1, 1) }
        cache.get("b") { byteArrayOf(2, 2) }
        cache.get("a") { error("Cached") }
        cache.get("c") { byteArrayOf(3, 3) }
        cache.get("huge") { ByteArray(5) }
        assertContentEquals(byteArrayOf(1, 1), cache.get("a") { error("Retained") })
        var reloaded = false
        cache.get("b") { reloaded = true; byteArrayOf(2, 2) }
        assertTrue(reloaded)
        cache.get("huge") { reloaded = false; ByteArray(5) }
        assertFalse(reloaded)
        cache.close()
    }

    @Test fun failedRequestsCanBeRetried() = runTest {
        val cache = ImageCache(backgroundScope)
        assertFailsWith<IllegalStateException> { cache.get("a") { error("Offline") } }
        assertContentEquals(byteArrayOf(1), cache.get("a") { byteArrayOf(1) })
        cache.close()
    }
}
