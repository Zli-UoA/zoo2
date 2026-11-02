@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun
import kotlin.js.JsAny

@JsFun(
    """(url, auth, refresh, closed) => {
    const socket = globalThis.zooCatalog?.createWebSocket?.(url) ?? new WebSocket(url); let disposed = false;
    socket.onopen = () => { if (!disposed) socket.send(auth); };
    socket.onmessage = event => { if (!disposed && event.data === 'refresh') refresh(); };
    socket.onclose = () => { if (!disposed) closed(); };
    return () => { if(disposed) return; disposed = true; socket.onopen = null; socket.onmessage = null; socket.onclose = null; socket.close(); };
}"""
)
private external fun homeSocket(
    url: String,
    auth: String,
    refresh: () -> Unit,
    closed: () -> Unit,
): JsAny

@JsFun("(dispose) => dispose()") private external fun disposeHomeSocket(dispose: JsAny)

internal actual fun connectHomeSocket(
    url: String,
    auth: String,
    refresh: () -> Unit,
    closed: () -> Unit,
): () -> Unit {
    val dispose = homeSocket(url, auth, refresh, closed)
    return { disposeHomeSocket(dispose) }
}
