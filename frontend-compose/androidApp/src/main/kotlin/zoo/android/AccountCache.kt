package zoo.android

import android.util.AtomicFile
import java.io.File
import java.security.MessageDigest

/** Serialize readers, writers and logout, including work already dispatched to the IO pool. */
internal class AccountCache(private val directory: File) {
    private var closed = false

    private fun file(key: String): AtomicFile {
        val name = MessageDigest.getInstance("SHA-256").digest(key.toByteArray())
            .joinToString("") { "%02x".format(it) }
        return AtomicFile(File(directory, name))
    }

    @Synchronized
    fun read(key: String): String? {
        if (closed) return null
        val file = file(key)
        // openRead also recovers an interrupted atomic write.
        return try {
            file.openRead().bufferedReader(Charsets.UTF_8).use { it.readText() }
        } catch (_: java.io.FileNotFoundException) {
            null
        }
    }

    @Synchronized
    fun write(key: String, value: String) {
        if (closed) return
        check(directory.isDirectory || directory.mkdirs()) { "Cannot create account cache" }
        val file = file(key)
        val stream = file.startWrite()
        try {
            stream.write(value.toByteArray(Charsets.UTF_8))
            file.finishWrite(stream)
        } catch (failure: Throwable) {
            file.failWrite(stream)
            throw failure
        }
    }

    @Synchronized
    fun clear() {
        closed = true
        directory.deleteRecursively()
    }
}
