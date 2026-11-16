package zoo.android.phone

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.webkit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.net.URI
import java.util.UUID
import zoo.*
import zoo.phone.PhoneMedia

internal fun phoneOrigin(value: String): String {
    val uri = URI(value)
    require(uri.scheme == "https" && uri.host != null && uri.rawUserInfo == null && uri.rawQuery == null && uri.rawFragment == null && uri.path.orEmpty() in listOf("", "/"))
    return "https://${uri.rawAuthority}"
}
internal fun phoneAsset(origin: String, url: String): String? {
    val uri = Uri.parse(url)
    if (uri.scheme != "https" || uri.encodedAuthority != Uri.parse(origin).encodedAuthority || uri.query != null || uri.fragment != null) return null
    return when (uri.encodedPath) { "/__zoo_phone__/index.html" -> "index.html"; "/__zoo_phone__/engine.js" -> "engine.js"; else -> null }
}
class PhoneJavascriptBridge(private val receive: (String) -> Unit) {
    @JavascriptInterface fun postMessage(value: String) { if (value.length <= 131_072) receive(value) }
}

/** Only two packaged assets can load. No remote HTML, navigation, storage or auth token bridge. */
@Suppress("SetJavaScriptEnabled")
internal class PhoneWebView(
    context: Context,
    base: String,
    private val mayCapture: () -> Boolean,
    private val event: (String, Row) -> Unit,
) : PhoneMedia {
    private val origin = phoneOrigin(base)
    private val main = Handler(Looper.getMainLooper())
    private val token = UUID.randomUUID().toString()
    private val loaded = CompletableDeferred<Unit>()
    private val commands = mutableMapOf<Long, CompletableDeferred<Unit>>()
    private var serial = 0L
    private var closed = false
    private val page = "$origin/__zoo_phone__/index.html"
    private val view = WebView(context.applicationContext)
    init {
        check(Looper.myLooper() == Looper.getMainLooper())
        view.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = false
            allowFileAccess = false
            allowContentAccess = false
            cacheMode = WebSettings.LOAD_NO_CACHE
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            mediaPlaybackRequiresUserGesture = false
            javaScriptCanOpenWindowsAutomatically = false
        }
        view.addJavascriptInterface(PhoneJavascriptBridge { value -> main.post { message(value) } }, "ZooNativePhone")
        view.webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) {
                main.post {
                    if (!closed && request.origin.toString().trimEnd('/') == origin && request.resources.toSet() == setOf(PermissionRequest.RESOURCE_AUDIO_CAPTURE) && context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED && mayCapture()) request.grant(arrayOf(PermissionRequest.RESOURCE_AUDIO_CAPTURE))
                    else request.deny()
                }
            }
        }
        view.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse {
                val name = phoneAsset(origin, request.url.toString())
                val bytes = when {
                    name == "index.html" && request.isForMainFrame -> {
                        val ws = origin.replaceFirst("https:", "wss:")
                        "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'self'; connect-src $ws; media-src blob:; base-uri 'none'; form-action 'none'\"></head><body><script src=\"/__zoo_phone__/engine.js\"></script></body></html>".toByteArray()
                    }
                    name == "engine.js" && !request.isForMainFrame -> runCatching { context.assets.open("phone/engine.js").use { it.readBytes() } }.getOrNull()
                    else -> null
                }
                return if (bytes != null) WebResourceResponse(if (name == "engine.js") "application/javascript" else "text/html", "UTF-8", 200, "OK", mapOf("Cache-Control" to "no-store", "X-Content-Type-Options" to "nosniff"), ByteArrayInputStream(bytes))
                else WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden", mapOf("Cache-Control" to "no-store"), ByteArrayInputStream(ByteArray(0)))
            }
            override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: android.net.http.SslError) { handler.cancel(); fail() }
            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) { if (request.isForMainFrame) fail() }
            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean { fail(); return true }
        }
        view.loadUrl(page)
    }
    private fun fail() { if (!closed) { loaded.completeExceptionally(IllegalStateException("Phone engine unavailable")); event("failed", obj()) } }
    private fun message(raw: String) {
        if (closed) return
        val value = runCatching { json.parseToJsonElement(raw).jsonObject }.getOrNull() ?: return
        if (value.text("event") == "loaded" && view.url == page) { loaded.complete(Unit); return }
        if (value.text("token") != token) return
        val payload = value["payload"] as? JsonObject ?: obj()
        if (value.text("event") == "result") {
            val id = payload["id"]?.jsonPrimitive?.longOrNull ?: return
            val result = commands.remove(id) ?: return
            if (payload.flag("ok")) result.complete(Unit) else result.completeExceptionally(IllegalStateException("Phone operation failed"))
        } else event(value.text("event"), payload)
    }
    override suspend fun command(method: String, payload: Row) = withContext(Dispatchers.Main.immediate) {
        check(method in setOf("sip.register", "sip.call", "sip.answer", "sip.end", "sip.dtmf", "dm.start", "dm.event", "mute"))
        withTimeout(15_000) {
            loaded.await(); check(!closed)
            val id = ++serial; val done = CompletableDeferred<Unit>(); commands[id] = done
            try {
                view.evaluateJavascript("window.ZooPhone.execute($id,${JSONObject.quote(token)},${JSONObject.quote(method)},JSON.parse(${JSONObject.quote(payload.toString())}));", null)
                done.await()
            } finally { commands.remove(id) }
        }
    }
    override fun close() {
        if (closed) return
        closed = true; loaded.cancel(); commands.values.forEach { it.cancel() }; commands.clear()
        // evaluateJavascript runs the synchronous media teardown before its completion callback.
        if (view.url == page) view.evaluateJavascript("window.ZooPhone&&window.ZooPhone.execute(0,${JSONObject.quote(token)},'close',{});", { destroy() }) else destroy()
    }
    private fun destroy() { view.stopLoading(); view.removeJavascriptInterface("ZooNativePhone"); view.destroy() }
}
