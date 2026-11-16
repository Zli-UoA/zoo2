package zoo

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.media.MediaPlayer
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Slider
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import java.io.File
import java.io.IOException
import java.net.InetAddress
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.*
import okhttp3.*

@Composable
internal fun NativeImagePreview(app: Session, src: String, name: String, modifier: Modifier) {
    var bitmap by remember(src) { mutableStateOf<ImageBitmap?>(null) }
    var failed by remember(src) { mutableStateOf(false) }
    LaunchedEffect(src) {
        try { app.platform.imageFrames(app.image(src)).collect { bitmap = it } }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { failed = true }
    }
    Box(modifier) {
        val frame = bitmap
        if (frame != null) Image(frame, name, Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
        else Text(app.t(if (failed) "attachments.loadError" else "attachments.loading"))
    }
}

/** Download once per open viewer; PDF pages and players reuse the same private file. */
@Composable
internal fun NativeFilePreview(app: Session, src: String, name: String, type: String, modifier: Modifier) {
    val context = LocalContext.current
    val file = remember(src) { File.createTempFile("media-preview-", ".tmp", context.cacheDir) }
    val client = remember(src) { previewFileClient(Uri.parse(app.config.apiBaseUrl).host) }
    var attempt by remember(src) { mutableStateOf(0) }
    var loaded by remember(src) { mutableStateOf(false) }
    var failed by remember(src) { mutableStateOf(false) }
    LaunchedEffect(src, attempt) {
        failed = false
        try { downloadPreviewFile(client, src, file); loaded = true }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { failed = true }
    }
    DisposableEffect(file) { onDispose { file.delete(); client.connectionPool.evictAll() } }
    Box(modifier) {
        when {
            !loaded -> Column {
                Text(app.t(if (failed) "attachments.loadError" else "attachments.loading"))
                if (failed) Button(app.t("attachments.retry")) { attempt++ }
            }
            type == "application/pdf" -> NativePdfPreview(app, file, name)
            type.startsWith("video/") -> NativeVideoPreview(app, file)
            else -> NativeAudioPreview(app, file)
        }
    }
}

internal fun previewFileClient(trustedHost: String?): OkHttpClient = OkHttpClient.Builder()
    .followRedirects(false).followSslRedirects(false)
    .connectTimeout(15, TimeUnit.SECONDS).readTimeout(30, TimeUnit.SECONDS)
    .callTimeout(60, TimeUnit.SECONDS)
    .dns(object : Dns {
        override fun lookup(hostname: String): List<InetAddress> = Dns.SYSTEM.lookup(hostname).also { addresses ->
            require(hostname == trustedHost || addresses.all(::publicPreviewAddress))
        }
    }).build()

internal suspend fun downloadPreviewFile(client: OkHttpClient, src: String, target: File) = suspendCancellableCoroutine<Unit> { continuation ->
    val uri = Uri.parse(src)
    if (uri.scheme != "https" || uri.host == null || uri.userInfo != null) {
        continuation.resumeWithException(IOException("Invalid preview source"))
        return@suspendCancellableCoroutine
    }
    val call = client.newCall(Request.Builder().url(src).build())
    continuation.invokeOnCancellation { call.cancel(); target.delete() }
    call.enqueue(object : Callback {
        override fun onFailure(call: Call, error: IOException) {
            target.delete()
            if (continuation.isActive) continuation.resumeWithException(error)
        }
        override fun onResponse(call: Call, response: Response) {
            try {
                response.use {
                    val body = it.body ?: throw IOException("Empty preview")
                    if (!it.isSuccessful || body.contentLength() > MAX_NATIVE_PREVIEW_BYTES) throw IOException("Preview unavailable")
                    body.byteStream().use { input -> target.outputStream().use { output ->
                        val buffer = ByteArray(8192)
                        var total = 0L
                        while (true) {
                            if (!continuation.isActive) throw IOException("Preview closed")
                            val size = input.read(buffer)
                            if (size < 0) break
                            total += size
                            if (total > MAX_NATIVE_PREVIEW_BYTES) throw IOException("Preview too large")
                            output.write(buffer, 0, size)
                        }
                    } }
                }
                continuation.resume(Unit) { _, _, _ -> target.delete() }
            } catch (error: Exception) {
                target.delete()
                if (continuation.isActive) continuation.resumeWithException(error)
            }
        }
    })
}
private const val MAX_NATIVE_PREVIEW_BYTES = 64L * 1024 * 1024

internal data class NativePdfPage(val bitmap: Bitmap, val pages: Int)
internal fun renderNativePdfPage(file: File, index: Int): NativePdfPage =
    ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
        PdfRenderer(descriptor).use { renderer ->
            renderer.openPage(index).use { page ->
                val scale = minOf(2f, 1600f / page.width, 2400f / page.height)
                val bitmap = Bitmap.createBitmap((page.width * scale).toInt().coerceAtLeast(1), (page.height * scale).toInt().coerceAtLeast(1), Bitmap.Config.ARGB_8888)
                try {
                    bitmap.eraseColor(android.graphics.Color.WHITE)
                    page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                    NativePdfPage(bitmap, renderer.pageCount)
                } catch (error: Exception) { bitmap.recycle(); throw error }
            }
        }
    }

@Composable
private fun NativePdfPreview(app: Session, file: File, name: String) {
    var index by remember(file) { mutableStateOf(0) }
    var page by remember(file) { mutableStateOf<NativePdfPage?>(null) }
    var failed by remember(file) { mutableStateOf(false) }
    var loading by remember(file) { mutableStateOf(true) }
    LaunchedEffect(file, index) {
        loading = true
        try { page = withContext(Dispatchers.IO) { renderNativePdfPage(file, index) } }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { failed = true }
        finally { loading = false }
    }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Button(app.t("attachments.previousPage"), enabled = index > 0 && !loading && !failed) { index-- }
            Text("${index + 1} / ${page?.pages ?: "—"}")
            Button(app.t("attachments.nextPage"), enabled = !loading && !failed && index + 1 < (page?.pages ?: 0)) { index++ }
        }
        when {
            failed -> Text(app.t("attachments.loadError"))
            loading -> Text(app.t("attachments.loading"))
            else -> page?.let { Image(it.bitmap.asImageBitmap(), name, Modifier.fillMaxWidth().weight(1f), contentScale = ContentScale.Fit) }
        }
    }
}

@Composable
private fun NativeVideoPreview(app: Session, file: File) {
    var failed by remember(file) { mutableStateOf(false) }
    if (failed) Text(app.t("attachments.loadError"))
    else AndroidView(
        update = { if (!app.env.visible) it.pause() },
        factory = { context -> VideoView(context).apply {
            setMediaController(MediaController(context).also { it.setAnchorView(this); tag = it })
            setOnErrorListener { _, _, _ -> failed = true; true }
            setVideoURI(Uri.fromFile(file))
            // Show the first frame, but leave playback to the user's controls.
            setOnPreparedListener { seekTo(1) }
        } },
        modifier = Modifier.fillMaxSize(),
        onRelease = { (it.tag as? MediaController)?.hide(); it.stopPlayback() },
    )
}

@Composable
private fun NativeAudioPreview(app: Session, file: File) {
    val player = remember(file) { MediaPlayer() }
    var ready by remember(file) { mutableStateOf(false) }
    var playing by remember(file) { mutableStateOf(false) }
    var failed by remember(file) { mutableStateOf(false) }
    var position by remember(file) { mutableStateOf(0f) }
    DisposableEffect(player) {
        player.setOnPreparedListener { ready = true }
        player.setOnCompletionListener { playing = false; position = 0f }
        player.setOnErrorListener { _, _, _ -> failed = true; playing = false; true }
        try { player.setDataSource(file.path); player.prepareAsync() }
        catch (_: Exception) { failed = true }
        onDispose { player.release() }
    }
    LaunchedEffect(app.env.visible) {
        if (!app.env.visible && playing) { player.pause(); playing = false }
    }
    LaunchedEffect(playing) {
        while (playing) { position = player.currentPosition.toFloat(); delay(500) }
    }
    Column(Modifier.fillMaxWidth()) {
        if (failed) Text(app.t("attachments.loadError"))
        else {
            IconButton(if (playing) "pause" else "play", app.t(if (playing) "attachments.pause" else "attachments.play"), enabled = ready) {
                if (playing) player.pause() else player.start()
                playing = !playing
            }
            Slider(position, { position = it; if (ready) player.seekTo(it.toInt()) }, enabled = ready,
                valueRange = 0f..(if (ready) player.duration.toFloat().coerceAtLeast(1f) else 1f))
        }
    }
}
