package zoo

import android.annotation.SuppressLint
import android.net.Uri
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.graphics.luminance
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import java.io.ByteArrayInputStream
import java.net.InetAddress
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import okhttp3.Call
import okhttp3.Dns
import okhttp3.OkHttpClient
import okhttp3.Request
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

private const val PREVIEW_ORIGIN = "https://zoo-preview.invalid"

/** No general-purpose proxy: only the selected source and bundled assets are accessible. */
@SuppressLint("SetJavaScriptEnabled")
@Composable
internal actual fun MediaPreviewSurface(app: Session, src: String, name: String, contentType: String, modifier: Modifier) {
    val dark = MaterialTheme.colorScheme.background.luminance() < 0.5f
    key(src, name, contentType, app.env.locale, dark) { NativeMediaPreview(app, src, name, contentType, dark, modifier) }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun NativeMediaPreview(app: Session, src: String, name: String, contentType: String, dark: Boolean, modifier: Modifier) {
    val context = LocalContext.current
    val locale = app.env.locale
    val disposed = remember(src) { AtomicBoolean(false) }
    val calls = remember(src) { ConcurrentHashMap.newKeySet<Call>() }
    val trustedHost = Uri.parse(app.config.apiBaseUrl).host
    val client = remember(src) {
        OkHttpClient.Builder().followRedirects(false).followSslRedirects(false)
            .connectTimeout(15, TimeUnit.SECONDS).readTimeout(30, TimeUnit.SECONDS)
            .dns(object : Dns {
                override fun lookup(hostname: String): List<InetAddress> =
                    Dns.SYSTEM.lookup(hostname).also { addresses ->
                        require(hostname == trustedHost || addresses.all(::publicPreviewAddress))
                    }
            }).build()
    }
    val view = remember(src, name, contentType, locale) {
        WebView(context).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
            settings.mediaPlaybackRequiresUserGesture = true
            addJavascriptInterface(object {
                @JavascriptInterface fun download() {
                    if (disposed.get()) return
                    app.run { app.platform.invoke("attachmentDownload", obj("url" to src, "name" to name).toString()) }
                }
                @JavascriptInterface fun save(fileName: String, type: String, base64: String) {
                    if (disposed.get() || base64.length > 24 * 1024 * 1024) return
                    app.run {
                        try { app.platform.invoke("previewSave", obj("name" to fileName, "type" to type, "base64" to base64).toString()) }
                        catch (_: Exception) { app.toast = app.t("attachments.downloadError") }
                    }
                }
            }, "NativePreview")
            webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse {
                    val uri = request.url
                    if (disposed.get() || "${uri.scheme}://${uri.authority}" != PREVIEW_ORIGIN || request.method != "GET") return deniedPreviewResponse()
                    if (uri.path == "/source") {
                        try {
                            val source = Uri.parse(src)
                            require(source.scheme == "https" && source.host != null && source.userInfo == null)
                            val builder = Request.Builder().url(src)
                            request.requestHeaders.entries.firstOrNull { it.key.equals("Range", ignoreCase = true) }?.let { builder.header("Range", it.value) }
                            val call = client.newCall(builder.build())
                            calls.add(call)
                            val response = call.execute()
                            val body = response.body
                            if (disposed.get() || !response.isSuccessful || body == null) { response.close(); calls.remove(call); return deniedPreviewResponse() }
                            // The stream is consumed by WebView and closed with the viewer; no disk cache.
                            val stream = object : java.io.FilterInputStream(body.byteStream()) {
                                override fun close() { try { super.close() } finally { response.close(); calls.remove(call) } }
                            }
                            return WebResourceResponse(contentType.substringBefore(';'), null, response.code,
                                response.message.ifEmpty { "OK" }, response.headers.toMultimap().mapValues { it.value.joinToString(", ") }, stream)
                        } catch (_: Exception) { return deniedPreviewResponse() }
                    }
                    val path = uri.path.orEmpty().removePrefix("/")
                    if (path.contains("..") || path.contains('\\') || path.isEmpty()) return deniedPreviewResponse()
                    return try {
                        val mime = when {
                            path.endsWith(".js") || path.endsWith(".mjs") -> "text/javascript"
                            path.endsWith(".css") -> "text/css"
                            path.endsWith(".html") -> "text/html"
                            path.endsWith(".wasm") -> "application/wasm"
                            path.endsWith(".woff2") -> "font/woff2"
                            else -> "application/octet-stream"
                        }
                        WebResourceResponse(mime, "UTF-8", context.assets.open("media-preview/$path"))
                    } catch (_: Exception) { deniedPreviewResponse() }
                }
                override fun onPageFinished(view: WebView, url: String) {
                    if (url == "$PREVIEW_ORIGIN/media-preview.html" && !disposed.get()) {
                        view.evaluateJavascript("window.openMediaPreview(${Json.encodeToString(name)},${Json.encodeToString(contentType)},${Json.encodeToString(locale)},$dark)", null)
                    }
                }
            }
            loadUrl("$PREVIEW_ORIGIN/media-preview.html")
        }
    }
    DisposableEffect(view) {
        onDispose {
            disposed.set(true)
            calls.forEach { it.cancel() }; calls.clear()
            view.stopLoading(); view.removeJavascriptInterface("NativePreview"); view.destroy()
            client.dispatcher.executorService.shutdown(); client.connectionPool.evictAll()
        }
    }
    AndroidView(factory = { view }, modifier = modifier)
}

internal fun publicPreviewAddress(address: InetAddress): Boolean {
    val bytes = address.address
    return !address.isAnyLocalAddress && !address.isLoopbackAddress && !address.isLinkLocalAddress &&
        !address.isSiteLocalAddress && !address.isMulticastAddress &&
        !(bytes.size == 16 && (bytes[0].toInt() and 0xfe) == 0xfc) &&
        !(bytes.size == 4 && (bytes[0].toInt() and 0xff) == 100 && (bytes[1].toInt() and 0xc0) == 64)
}
private fun deniedPreviewResponse() = WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden", emptyMap(), ByteArrayInputStream(byteArrayOf()))
