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
        check(!result.isNullOrBlank()) { "Shizuku のインストーラーから結果を取得できませんでした。Shizuku を再起動して再試行してください" }
        return result
    } finally {
        withContext(NonCancellable) { runCatching { disconnect() } }
    }
}
