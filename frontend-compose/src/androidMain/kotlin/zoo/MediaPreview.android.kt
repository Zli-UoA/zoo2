package zoo

import android.annotation.SuppressLint
import android.net.Uri
import android.os.Build
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Surface
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.graphics.luminance
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import java.io.ByteArrayInputStream
import java.net.InetAddress
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.Call
import okhttp3.Request
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

private const val PREVIEW_ORIGIN = "https://zoo-preview.invalid"

/** No general-purpose proxy: only the selected source and bundled assets are accessible. */
@SuppressLint("SetJavaScriptEnabled")
@Composable
internal actual fun MediaPreviewSurface(app: Session, src: String, name: String, contentType: String, modifier: Modifier) {
    val dark = MaterialTheme.colorScheme.background.luminance() < 0.5f
    key(src, name, contentType, app.env.locale, dark) {
        val mime = contentType.substringBefore(';').trim().lowercase()
        when {
            mime.startsWith("image/") && !nativeImagePreviewSupported(mime) -> Box(modifier) {
                Button(app.t("attachments.downloadShort")) {
                    app.run { app.platform.invoke("attachmentDownload", obj("url" to src, "name" to name).toString()) }
                }
            }
            mime.startsWith("image/") -> NativeImagePreview(app, src, name, modifier)
            mime == "application/pdf" || mime.startsWith("video/") || mime.startsWith("audio/") ->
                NativeFilePreview(app, src, name, mime, modifier)
            else -> BundledDocumentPreview(app, src, name, contentType, dark, modifier)
        }
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun BundledDocumentPreview(app: Session, src: String, name: String, contentType: String, dark: Boolean, modifier: Modifier) {
    val context = LocalContext.current
    val locale = app.env.locale
    val disposed = remember(src) { AtomicBoolean(false) }
    val calls = remember(src) { ConcurrentHashMap.newKeySet<Call>() }
    val trustedHost = app.config.apiBaseUrl.toHttpUrlOrNull()?.host
    val client = remember(src) { previewFileClient(trustedHost) }
    val view = remember(src, name, contentType, locale) {
        WebView(context).apply {
            settings.useWideViewPort = true
            settings.loadWithOverviewMode = true
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
                            if (!registerPreviewCall(call, calls, disposed)) return deniedPreviewResponse()
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
            client.connectionPool.evictAll()
        }
    }
    AndroidView(factory = { view }, modifier = modifier, onRelease = {
        disposed.set(true)
        calls.forEach { call -> call.cancel() }; calls.clear()
        releasePreviewWebView(it)
    })
}

internal fun publicPreviewAddress(address: InetAddress): Boolean {
    val bytes = address.address
    return !address.isAnyLocalAddress && !address.isLoopbackAddress && !address.isLinkLocalAddress &&
        !address.isSiteLocalAddress && !address.isMulticastAddress &&
        !(bytes.size == 16 && (bytes[0].toInt() and 0xfe) == 0xfc) &&
        !(bytes.size == 4 && (bytes[0].toInt() and 0xff) == 100 && (bytes[1].toInt() and 0xc0) == 64)
}
private fun deniedPreviewResponse() = WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden", emptyMap(), ByteArrayInputStream(byteArrayOf()))

/** Close the race between the initial disposed check and registering a synchronous Call. */
internal fun registerPreviewCall(call: Call, calls: MutableSet<Call>, disposed: AtomicBoolean): Boolean {
    calls.add(call)
    if (disposed.get()) {
        call.cancel()
        calls.remove(call)
        return false
    }
    return true
}

/** AndroidView releases the native view after Compose removes it from the hierarchy. */
internal fun releasePreviewWebView(view: WebView) {
    (view.parent as? android.view.ViewGroup)?.removeView(view)
    view.stopLoading()
    view.removeJavascriptInterface("NativePreview")
    view.webViewClient = WebViewClient()
    view.removeAllViews()
    view.destroy()
}

@Composable
internal actual fun MediaPreviewPanel(title: String, close: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    val app = LocalApp.current
    // A separate native window keeps the WebView out of NativeWindow's background blur.
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize().safeDrawingPadding(), color = MaterialTheme.colorScheme.background) {
            Column(Modifier.fillMaxSize().padding(8.dp)) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(title, modifier = Modifier.weight(1f), maxLines = 1)
                    IconButton("x", app.t("common.close")) { close() }
                }
                content()
            }
        }
    }
}

internal fun nativeImagePreviewSupported(mime: String, sdk: Int = Build.VERSION.SDK_INT): Boolean =
    mime.substringBefore(';').trim().lowercase() != "image/avif" || sdk >= 31
