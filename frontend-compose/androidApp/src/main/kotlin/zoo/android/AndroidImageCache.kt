package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import okhttp3.Dispatcher
import okhttp3.OkHttpClient

internal const val FOREGROUND_IMAGE_PARALLELISM = 6

/** Account-local image storage shared by public and signed downloads. */
internal class AndroidImageCache(
    directory: File,
    maxBytes: Long = 512L * 1024 * 1024,
    timeoutMillis: Long = 120_000,
    legacyDirectory: File? = null,
    configure: OkHttpClient.Builder.() -> Unit = {},
) : java.io.Closeable {
    private var closed = false
    private val bodyDirectory = imageBodyDirectory(directory)
    private val disk = acquire(bodyDirectory, maxBytes, legacyDirectory)
    private class HostSlot {
        val permits = Semaphore(FOREGROUND_IMAGE_PARALLELISM)
        var users = 0
    }

    private val hostSlots = mutableMapOf<String, HostSlot>()

    private suspend fun withHostSlot(host: String, transfer: suspend () -> Unit) {
        val slot = synchronized(hostSlots) {
            hostSlots.getOrPut(host, ::HostSlot).also { it.users++ }
        }
        try {
            slot.permits.withPermit { transfer() }
        } finally {
            synchronized(hostSlots) {
                if (--slot.users == 0) hostSlots.remove(host)
            }
        }
    }

    private val client = OkHttpClient.Builder()
        .dispatcher(Dispatcher().apply { maxRequestsPerHost = FOREGROUND_IMAGE_PARALLELISM })
        .apply(configure)
        .followRedirects(false)
        .followSslRedirects(false)
        .build()
    private val http = HttpClient(OkHttp) {
        followRedirects = false
        engine {
            preconfigured = client
            config { dispatcher(client.dispatcher) }
        }
        install(HttpTimeout) {
            requestTimeoutMillis = timeoutMillis
            socketTimeoutMillis = minOf(timeoutMillis, 90_000)
        }
    }

    suspend fun cachedAttachmentPreview(cacheKey: String): ByteArray? = withContext(Dispatchers.IO) {
        disk.read(cacheKey)?.bytes
    }

    suspend fun discardUndecodableImage(source: String, attachmentKey: String?, bytes: ByteArray) =
        withContext(Dispatchers.IO) { disk.discard(attachmentKey ?: "public:$source", bytes) }

    suspend fun resizeAttachmentCache(bytes: Long) = withContext(Dispatchers.IO) { disk.resize(bytes) }

    suspend fun removeAttachmentImages(id: String) = withContext(Dispatchers.IO) {
        disk.revoke("$id:thumbnail")
        disk.revoke("$id:original")
    }

    suspend fun allowAttachmentImages(id: String) = withContext(Dispatchers.IO) {
        disk.allow("$id:thumbnail")
        disk.allow("$id:original")
    }

    suspend fun signedImage(url: String, cacheKey: String? = null, retry: Boolean = false): ByteArray =
        load(ImageRequest(url), cacheKey ?: "signed:$url", 20 * 1024 * 1024, retry, signed = true)

    suspend fun cachedPublicImage(source: String, limit: Int): ByteArray? = withContext(Dispatchers.IO) {
        disk.read("public:$source", limit)?.bytes
    }

    suspend fun download(
        request: ImageRequest, limit: Int, bypassCache: Boolean = false, source: String = request.url,
        redirectRoute: (suspend (String) -> ImageRequest)? = null, push: Boolean = false,
    ): ByteArray = load(request, "public:$source", limit, bypassCache, redirectRoute = redirectRoute, push = push)

    private suspend fun load(
        request: ImageRequest, key: String, limit: Int, refresh: Boolean, signed: Boolean = false,
        redirectRoute: (suspend (String) -> ImageRequest)? = null, push: Boolean = false,
    ): ByteArray = withContext(Dispatchers.IO) {
        val token = disk.beginLoad(key) ?: throw ImageHttpFailure(403)
        try {
            val stored = disk.read(key, limit)
            val bytes = if (stored != null && !refresh) stored.bytes else
                loadPublicImage(http, request.url, limit, restrictOrigin = request.proxied,
                    noStore = signed, conditional = stored, withHostSlot = ::withHostSlot,
                    redirectRoute = redirectRoute, push = push) { entry -> disk.write(key, token, entry) }
            if (!disk.isCurrent(key, token)) throw ImageHttpFailure(403)
            bytes
        } finally {
            disk.endLoad(key, token)
        }
    }

    @Synchronized
    override fun close() {
        if (closed) return
        closed = true
        release(bodyDirectory, disk)
        http.close()
        client.connectionPool.evictAll()
        client.dispatcher.executorService.shutdown()
    }

    companion object {
        private class SharedStore(val disk: AttachmentImageDiskCache, var users: Int = 1)
        private fun imageBodyDirectory(directory: File) = directory.resolveSibling("${directory.name}-attachment-bodies-v3")
        private val stores = mutableMapOf<File, SharedStore>()

        @Synchronized
        private fun acquire(directory: File, maxBytes: Long, legacyDirectory: File?): AttachmentImageDiskCache {
            stores[directory]?.let {
                it.users++
                return it.disk
            }
            return AttachmentImageDiskCache(directory, maxBytes, legacyDirectory?.let(::imageBodyDirectory), legacyDirectory)
                .also { stores[directory] = SharedStore(it) }
        }

        @Synchronized
        private fun release(directory: File, disk: AttachmentImageDiskCache) {
            val store = stores[directory] ?: return
            if (store.disk === disk && --store.users == 0) {
                stores.remove(directory)
                disk.close()
            }
        }

        @Synchronized
        fun clearAccount(directory: File) {
            val bodyDirectory = imageBodyDirectory(directory)
            stores.remove(bodyDirectory)?.disk?.close()
            bodyDirectory.deleteRecursively()
        }
    }
}
