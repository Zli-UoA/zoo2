package zoo.android

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.IOException
import java.security.MessageDigest

/** Stores attachment bytes within one account. Callers check access before every lookup. */
internal class AttachmentImageDiskCache(
    private val directory: File,
    private val maxBytes: Long = 128L * 1024 * 1024,
    private val maxEntries: Int = 512,
) {
    data class Freshness(val maxAgeMillis: Long?, val httpExpiresAt: Long?)

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
    fun read(id: String, freshAt: Long? = null): Entry? {
        if (closed || id in blockedKeys) return null
        val file = file(id)
        return try {
            if (!file.isFile || file.length() > maxBytes) return null
            DataInputStream(file.inputStream().buffered()).use { input ->
                val url = input.readUTF()
                val expiresAt = input.readLong()
                if (freshAt != null && expiresAt <= freshAt) return null
                val etag = input.readUTF().ifEmpty { null }
                val maxAge = input.readLong().takeUnless { it == Long.MIN_VALUE }
                val httpExpiry = input.readLong().takeUnless { it == Long.MIN_VALUE }
                Entry(url, expiresAt, etag, Freshness(maxAge, httpExpiry), input.readBytes())
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
            output.writeUTF(entry.urlHash)
            output.writeLong(entry.expiresAt)
            output.writeUTF(entry.etag.orEmpty())
            output.writeLong(entry.freshness.maxAgeMillis ?: Long.MIN_VALUE)
            output.writeLong(entry.freshness.httpExpiresAt ?: Long.MIN_VALUE)
        }
        val sizeOnDisk = metadata.size().toLong() + entry.bytes.size
        if (closed || sizeOnDisk > maxBytes) return
        if (!directory.isDirectory && !directory.mkdirs()) return
        val target = file(id)
        val files = directory.listFiles()?.filter { it != target } ?: return
        var total = files.sumOf { it.length() } + sizeOnDisk
        var entries = files.size + 1
        for (candidate in files.sortedBy { it.lastModified() }) {
            if (total <= maxBytes && entries <= maxEntries) break
            val size = candidate.length()
            if (candidate.delete()) {
                total -= size
                entries--
            }
        }
        if (total > maxBytes || entries > maxEntries) return
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
    fun remove(id: String, token: Any) {
        if (isCurrent(id, token)) file(id).delete()
    }

    companion object {
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
