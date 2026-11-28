package zoo.android.update

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Keep the IPC platform type nullable until the response has been validated. */
internal suspend fun <T> installerSession(
    connect: suspend () -> T,
    install: suspend (T) -> String?,
    disconnect: suspend () -> Unit,
): String {
    try {
        val result = install(connect())
        if (result.isNullOrBlank()) throw UpdateStateException("android.update.noResultFromShizuku")
        return result
    } finally {
        withContext(NonCancellable) { runCatching { disconnect() } }
    }
}
