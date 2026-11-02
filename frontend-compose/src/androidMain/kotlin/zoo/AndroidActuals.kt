package zoo

import android.icu.text.Collator
import android.icu.util.ULocale
import androidx.compose.ui.text.PlatformTextStyle
import okhttp3.*

internal actual fun nativeSelectedText(): String = ""

internal actual fun nativeAbsoluteUrl(url: String): String = url

internal actual fun webTextStyle(): PlatformTextStyle =
    PlatformTextStyle(includeFontPadding = false)

actual fun compareLocaleText(
    left: String,
    right: String,
    locale: String,
    numeric: Boolean,
    baseSensitivity: Boolean,
): Int {
    val collator =
        Collator.getInstance(ULocale.forLanguageTag(locale)) as android.icu.text.RuleBasedCollator
    collator.numericCollation = numeric
    if (baseSensitivity) collator.strength = Collator.PRIMARY
    return collator.compare(left, right)
}

internal actual fun connectHomeSocket(
    url: String,
    auth: String,
    refresh: () -> Unit,
    closed: () -> Unit,
): () -> Unit {
    val handler = android.os.Handler(android.os.Looper.getMainLooper())
    val disposed = java.util.concurrent.atomic.AtomicBoolean(false)
    fun dispatch(task: () -> Unit) {
        handler.post { if (!disposed.get()) task() }
    }
    val client = OkHttpClient()
    val socket =
        client.newWebSocket(
            Request.Builder().url(url).build(),
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    webSocket.send(auth)
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    if (text == "refresh") dispatch(refresh)
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    dispatch(closed)
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                    dispatch(closed)
                }
            },
        )
    return {
        disposed.set(true)
        socket.cancel()
        client.dispatcher.executorService.shutdown()
        client.connectionPool.evictAll()
    }
}
