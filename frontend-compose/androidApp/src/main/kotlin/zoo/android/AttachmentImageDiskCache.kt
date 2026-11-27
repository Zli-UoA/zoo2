package zoo.android

import java.io.File
import java.io.IOException
import java.security.MessageDigest

/** Stores attachment bytes within one account. Callers check access before every lookup. */
internal class AttachmentImageDiskCache(
    private val directory: File,
    private val maxBytes: Long = 128L * 1024 * 1024,
    private val maxEntries: Int = 512,
) {
    private var closed = false

    private fun file(id: String): File {
        val name = MessageDigest.getInstance("SHA-256").digest(id.toByteArray())
            .joinToString("") { "%02x".format(it) }
        return directory.resolve(name)
    }

    @Synchronized
    fun read(id: String): ByteArray? {
        if (closed) return null
        val file = file(id)
        return try {
            if (!file.isFile || file.length() > maxBytes) return null
            file.readBytes().also { file.setLastModified(System.currentTimeMillis()) }
        } catch (_: IOException) {
            null
        }
    }

    @Synchronized
    fun write(id: String, bytes: ByteArray) {
        if (closed || bytes.size > maxBytes) return
        if (!directory.isDirectory && !directory.mkdirs()) return
        val target = file(id)
        val files = directory.listFiles()?.filter { it != target } ?: return
        var total = files.sumOf { it.length() } + bytes.size
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
            temporary.writeBytes(bytes)
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

    @Synchronized
    fun close() {
        closed = true
    }
}
