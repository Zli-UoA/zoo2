package zoo.android

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.IOException
import java.security.MessageDigest

/** Stores attachment bytes within one account. Callers check access before every lookup. */
internal class AttachmentImageDiskCache(
    private val directory: File,
    private var maxBytes: Long,
) {
    data class Freshness(val maxAgeMillis: Long?, val httpExpiresAt: Long?, val canPreviewStale: Boolean?)

    data class Entry(
        val urlHash: String,
        val expiresAt: Long,
        val etag: String?,
        val freshness: Freshness,
        val bytes: ByteArray,
    )

    private var closed = false
    private val blockedKeys = mutableSetOf<String>()
    private val activeLoads = mutableMapOf<String, MutableSet<Any>>()

    @Synchronized
    fun beginLoad(key: String): Any? {
        if (closed || key in blockedKeys) return null
        return Any().also { activeLoads.getOrPut(key) { mutableSetOf() }.add(it) }
    }

    @Synchronized
    fun endLoad(key: String, token: Any) {
        val loads = activeLoads[key] ?: return
        if (loads.remove(token) && loads.isEmpty()) activeLoads.remove(key)
    }

    @Synchronized
    fun isCurrent(key: String, token: Any): Boolean = activeLoads[key]?.contains(token) == true

    @Synchronized
    fun revoke(key: String) {
        if (closed) return
        blockedKeys.add(key)
        activeLoads.remove(key)
        file(key).delete()
    }

    @Synchronized
    fun allow(key: String) {
        blockedKeys.remove(key)
    }

    private fun file(id: String): File {
        return directory.resolve(digest(id))
    }

    @Synchronized
    fun read(id: String, previewAt: Long? = null): Entry? {
        if (closed || id in blockedKeys) return null
        val file = file(id)
        return try {
            if (!file.isFile || file.length() > maxBytes) return null
            DataInputStream(file.inputStream().buffered()).use { input ->
                // Older entries lack this prefix and still require fresh HTTP metadata for a preview.
                val storedUrl = input.readUTF()
                val canPreviewStale = when {
                    storedUrl.startsWith(StalePreviewPrefix) -> true
                    storedUrl.startsWith(FreshPreviewPrefix) -> false
                    else -> null
                }
                val url = storedUrl.removePrefix(StalePreviewPrefix).removePrefix(FreshPreviewPrefix)
                val expiresAt = input.readLong()
                if (previewAt != null && expiresAt <= previewAt && canPreviewStale != true) return null
                val etag = input.readUTF().ifEmpty { null }
                val maxAge = input.readLong().takeUnless { it == Long.MIN_VALUE }
                val httpExpiry = input.readLong().takeUnless { it == Long.MIN_VALUE }
                Entry(url, expiresAt, etag, Freshness(maxAge, httpExpiry, canPreviewStale), input.readBytes())
            }.also { file.setLastModified(System.currentTimeMillis()) }
        } catch (_: IOException) {
            null
        }
    }

    @Synchronized
    fun write(id: String, token: Any, entry: Entry) {
        if (!isCurrent(id, token)) return
        val metadata = java.io.ByteArrayOutputStream()
        DataOutputStream(metadata).use { output ->
            val prefix = if (entry.freshness.canPreviewStale == true) StalePreviewPrefix else FreshPreviewPrefix
            output.writeUTF(prefix + entry.urlHash)
            output.writeLong(entry.expiresAt)
            output.writeUTF(entry.etag.orEmpty())
            output.writeLong(entry.freshness.maxAgeMillis ?: Long.MIN_VALUE)
            output.writeLong(entry.freshness.httpExpiresAt ?: Long.MIN_VALUE)
        }
        val sizeOnDisk = metadata.size().toLong() + entry.bytes.size
        if (closed || sizeOnDisk > maxBytes) return
        if (!directory.isDirectory && !directory.mkdirs()) return
        val target = file(id)
        if (!trimToBudget(excluding = target, reservedBytes = sizeOnDisk)) return
        val temporary = directory.resolve("${target.name}.tmp")
        try {
            temporary.outputStream().buffered().use { output ->
                metadata.writeTo(output)
                output.write(entry.bytes)
            }
            temporary.renameTo(target)
        } catch (_: IOException) {
            // The response can still be displayed when the disk cache is unavailable.
        } finally {
            temporary.delete()
        }
    }

    @Synchronized
    fun resize(bytes: Long) {
        if (closed) return
        maxBytes = bytes
        trimToBudget()
    }

    private fun trimToBudget(excluding: File? = null, reservedBytes: Long = 0): Boolean {
        if (!directory.exists()) return true
        val files = directory.listFiles()?.filter { it != excluding } ?: return false
        var total = files.sumOf { it.length() } + reservedBytes
        if (maxBytes > 0 && total <= maxBytes) return true
        for (candidate in files.sortedBy { it.lastModified() }) {
            if (maxBytes > 0 && total <= maxBytes) break
            val size = candidate.length()
            if (candidate.delete()) total -= size
        }
        return total <= maxBytes
    }

    @Synchronized
    fun remove(id: String, token: Any) {
        if (isCurrent(id, token)) file(id).delete()
    }

    companion object {
        private const val StalePreviewPrefix = "preview:"
        private const val FreshPreviewPrefix = "fresh:"
        fun digest(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray())
            .joinToString("") { "%02x".format(it) }
    }

    @Synchronized
    fun close() {
        closed = true
        activeLoads.clear()
        blockedKeys.clear()
    }
}
