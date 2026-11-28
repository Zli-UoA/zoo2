package zoo.android

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.IOException
import java.security.MessageDigest

/** Account-local image bodies ordered by last use. */
internal class AttachmentImageDiskCache(
    private val directory: File,
    private var maxBytes: Long,
    private val legacyBodyDirectory: File? = null,
    private val legacyHttpDirectory: File? = null,
) {
    data class Entry(
        val responseUrlHash: String?,
        val etag: String?,
        val lastModified: String?,
        val bytes: ByteArray,
    )

    private var closed = false
    private val blockedKeys = mutableSetOf<String>()
    private val activeLoads = mutableMapOf<String, MutableSet<Any>>()
    private val filesByUse = linkedMapOf<File, Long>()
    private var indexed = false
    private var prepared = false
    private var totalBytes = 0L

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
        prepare()
        delete(file(key))
    }

    @Synchronized
    fun allow(key: String) {
        blockedKeys.remove(key)
    }

    private fun file(id: String): File {
        return directory.resolve(digest(id))
    }

    @Synchronized
    fun read(id: String, limit: Int = 20 * 1024 * 1024): Entry? {
        if (closed || id in blockedKeys) return null
        if (!prepare()) return null
        val file = file(id)
        return try {
            if (!file.isFile || file.length() > maxBytes) return null
            DataInputStream(file.inputStream().buffered()).use { input ->
                val header = input.readUTF()
                val validator = if (header == BodyFormat) {
                    Triple(input.readUTF().ifEmpty { null }, input.readUTF().ifEmpty { null },
                        input.readUTF().ifEmpty { null })
                } else {
                    // Read existing attachment bodies without applying their old freshness metadata.
                    input.readLong()
                    if (header.startsWith("http:")) input.readLong()
                    val etag = input.readUTF().ifEmpty { null }
                    input.readLong()
                    input.readLong()
                    Triple(header.substringAfterLast(':').takeIf { it.length == 64 }, etag, null)
                }
                val bytes = input.readNBytesCompat(limit)
                Entry(validator.first, validator.second, validator.third, bytes)
            }.also {
                file.setLastModified(System.currentTimeMillis())
                filesByUse.remove(file)?.let { size -> filesByUse[file] = size }
            }
        } catch (_: IOException) {
            null
        }
    }

    private fun DataInputStream.readNBytesCompat(limit: Int): ByteArray {
        val size = available()
        if (size > limit) throw IOException("Cached image exceeds size limit")
        return ByteArray(size).also { readFully(it) }
    }

    @Synchronized
    fun write(id: String, token: Any, entry: Entry) {
        if (!isCurrent(id, token)) return
        if (!prepare()) return
        val metadata = java.io.ByteArrayOutputStream()
        DataOutputStream(metadata).use { output ->
            output.writeUTF(BodyFormat)
            output.writeUTF(entry.responseUrlHash.orEmpty())
            output.writeUTF(entry.etag.orEmpty())
            output.writeUTF(entry.lastModified.orEmpty())
        }
        val sizeOnDisk = metadata.size().toLong() + entry.bytes.size
        if (closed || sizeOnDisk > maxBytes) return
        if (!directory.isDirectory && !directory.mkdirs()) return
        val target = file(id)
        if (!indexFiles()) return
        val temporary = directory.resolve("${target.name}.tmp")
        try {
            temporary.outputStream().buffered().use { output ->
                metadata.writeTo(output)
                output.write(entry.bytes)
            }
            if (!temporary.renameTo(target)) return
            totalBytes -= filesByUse.remove(target) ?: 0L
            filesByUse[target] = sizeOnDisk
            totalBytes += sizeOnDisk
            trimToBudget()
        } catch (_: IOException) {
            // The response can still be displayed when the disk cache is unavailable.
        } finally {
            temporary.delete()
        }
    }

    @Synchronized
    fun resize(bytes: Long): Boolean {
        if (closed) return false
        maxBytes = bytes
        return prepare() && trimToBudget()
    }

    private fun prepare(): Boolean {
        if (prepared) return true
        val legacy = legacyBodyDirectory
        if (legacy != null && legacy.isDirectory && !directory.exists()) {
            directory.parentFile?.mkdirs()
            if (!legacy.renameTo(directory)) return false
        }
        legacyHttpDirectory?.deleteRecursively()
        prepared = true
        trimToBudget()
        return true
    }

    private fun indexFiles(): Boolean {
        if (indexed) return true
        if (!directory.exists()) return true
        val files = directory.listFiles() ?: return false
        files.filter { it.isFile }
            .map { it to it.lastModified() }
            .sortedBy { it.second }
            .forEach { (file, _) ->
                val size = file.length()
                filesByUse[file] = size
                totalBytes += size
            }
        indexed = true
        return true
    }

    private fun trimToBudget(): Boolean {
        if (!indexFiles()) return false
        val entries = filesByUse.entries.iterator()
        while ((totalBytes > maxBytes || maxBytes == 0L) && entries.hasNext()) {
            val (file, size) = entries.next()
            if (file.delete() || !file.exists()) {
                totalBytes -= size
                entries.remove()
            }
        }
        return if (maxBytes == 0L) filesByUse.isEmpty() else totalBytes <= maxBytes
    }

    @Synchronized
    fun discard(id: String, bytes: ByteArray) {
        if (read(id)?.bytes?.contentEquals(bytes) == true) delete(file(id))
    }

    private fun delete(file: File) {
        if (file.delete() || !file.exists()) totalBytes -= filesByUse.remove(file) ?: 0L
    }

    companion object {
        private const val BodyFormat = "image-body-v1"

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
