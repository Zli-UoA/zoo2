@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun
import kotlin.js.JsAny

@JsFun("(bridge, id, bounds, receive) => bridge.fileTarget(id, bounds, receive)")
private external fun registerBrowserFiles(
    bridge: JsAny,
    id: String,
    bounds: String,
    receive: (String) -> Boolean,
)

@JsFun("(bridge, id) => bridge.forgetFileTarget(id)")
private external fun forgetBrowserFiles(bridge: JsAny, id: String)

/** The production app and Storybook use the same browser file ownership and Kotlin callbacks. */
internal class BrowserFileBridge(private val bridge: JsAny) {
    fun target(id: String, bounds: String, receive: (String) -> Boolean) =
        registerBrowserFiles(bridge, id, bounds, receive)

    fun forget(id: String) = forgetBrowserFiles(bridge, id)

    fun action(name: String) = browserAction(bridge, name)

    suspend fun invoke(name: String, payload: String): String =
        browserInvoke(bridge, name, payload).awaitBrowser().toString()
}
