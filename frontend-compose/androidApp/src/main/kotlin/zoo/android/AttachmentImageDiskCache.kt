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
    data class Entry(val urlHash: String, val expiresAt: Long, val etag: String?, val bytes: ByteArray)

    private var closed = false

    private fun file(id: String): File {
        return directory.resolve(digest(id))
    }

    @Synchronized
    fun read(id: String): Entry? {
        if (closed) return null
        val file = file(id)
        return try {
            if (!file.isFile || file.length() > maxBytes) return null
            DataInputStream(file.inputStream().buffered()).use { input ->
                val url = input.readUTF()
                val expiresAt = input.readLong()
                val etag = input.readUTF().ifEmpty { null }
                Entry(url, expiresAt, etag, input.readBytes())
            }.also { file.setLastModified(System.currentTimeMillis()) }
        } catch (_: IOException) {
            null
        }
    }

    @Synchronized
    fun write(id: String, entry: Entry) {
        val metadata = java.io.ByteArrayOutputStream()
        DataOutputStream(metadata).use { output ->
            output.writeUTF(entry.urlHash)
            output.writeLong(entry.expiresAt)
            output.writeUTF(entry.etag.orEmpty())
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
    fun remove(id: String) {
        if (!closed) file(id).delete()
    }

    companion object {
        fun digest(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray())
            .joinToString("") { "%02x".format(it) }
    }

    @Synchronized
    fun close() {
        closed = true
    }
}
