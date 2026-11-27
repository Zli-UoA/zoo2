package zoo.android

import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.layout.*
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import java.io.File
import java.io.IOException
import java.net.InetAddress
import java.net.UnknownHostException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.*
import okhttp3.HttpUrl.Companion.toHttpUrl
import zoo.ApiException
import zoo.AttachmentImageLinks
import zoo.Session
import zoo.android.ui.*

internal suspend fun timelineMediaUrl(session: Session, media: TimelineMedia): String =
    media.url ?: session.attachmentLinks.get(requireNotNull(media.attachmentId))

internal fun cachedTimelineImage(session: Session, media: TimelineMedia): ImageBitmap? {
    val thumbnail = media.thumbnailUrl.takeIf { media.useThumbnail }
    thumbnail?.let { session.cachedImageBitmap(it) }?.let { return it }
    val publicUrl = media.previewUrl ?: media.url
    if (publicUrl != null) return session.cachedImageBitmap(publicUrl)
    val id = media.attachmentId ?: return null
    if (session.attachmentLinks.isDenied(id)) return null
    val links = session.attachmentLinks.cachedLinks(id) ?: return null
    val signedThumbnail = links.thumbnail.takeIf { media.useThumbnail }
    signedThumbnail?.let { session.cachedImageBitmap(it, signedAttachment = true, cacheKey = "$id:thumbnail") }
        ?.let { return it }
    return session.cachedImageBitmap(links.original, signedAttachment = true, cacheKey = "$id:original")
}

internal suspend fun timelineImage(
    session: Session,
    media: TimelineMedia,
    retry: Boolean = false,
    onCached: ((ImageBitmap) -> Unit)? = null,
): ImageBitmap? {
    suspend fun load(url: String): ImageBitmap? = try {
        session.imageBitmap(url, retry = retry)
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        null
    }

    val publicUrl = media.previewUrl ?: media.url
    if (publicUrl != null) {
        if (media.useThumbnail) media.thumbnailUrl?.let { load(it) }?.let { return it }
        return load(publicUrl)
    }
    val id = media.attachmentId ?: return null
    val platform = session.platform
    val previouslyDenied = session.attachmentLinks.isDenied(id)
    val cached = if (onCached != null && !retry && !previouslyDenied) {
        suspend fun decode(key: String): ImageBitmap? = try {
            platform.freshAttachmentImage(key)?.let { platform.imageFrames(it).first() }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            null
        }
        val thumbnail = if (media.useThumbnail) decode("$id:thumbnail") else null
        (thumbnail ?: decode("$id:original"))?.also(onCached)
    } else null
    fun canConfirm(): Boolean = session.env.online && session.env.visible
    if (cached != null) snapshotFlow { canConfirm() }.first { it }
    var retryDelay = 5_000L
    var links: AttachmentImageLinks? = null
    while (links == null) {
        try {
            links = session.attachmentLinks.getLinks(id, force = retry || previouslyDenied || cached != null)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            if (cached == null || !failure.isTemporaryImageFailure()) return null
            withTimeoutOrNull(retryDelay) { snapshotFlow { canConfirm() }.first { !it } }
            snapshotFlow { canConfirm() }.first { it }
            retryDelay = (retryDelay * 2).coerceAtMost(60_000L)
        }
    }

    suspend fun loadAttachment(url: String, variant: String): ImageBitmap =
        session.imageBitmap(url, signedAttachment = true, retry = retry, cacheKey = "$id:$variant")

    val thumbnail = links.thumbnail.takeIf { media.useThumbnail }
    var thumbnailFailure: Exception? = null
    if (thumbnail != null) {
        try {
            return loadAttachment(thumbnail, "thumbnail")
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            thumbnailFailure = failure
        }
    }
    return try {
        loadAttachment(links.original, "original")
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (failure: Exception) {
        cached.takeIf {
            failure.isTemporaryImageFailure() ||
                (failure !is ImageHttpFailure && thumbnailFailure?.isTemporaryImageFailure() == true)
        }
    }
}

private fun Exception.isTemporaryImageFailure(): Boolean = when (this) {
    is ApiException -> status == 429 || status in 500..599
    is ImageHttpFailure -> status == 429 || status in 500..599
    is IOException -> true
    else -> false
}

@Composable
internal fun TimelineVideoPreview(
    session: Session, media: TimelineMedia, colors: AndroidColors, dismiss: () -> Unit, openOriginal: () -> Unit,
) {
    val context = LocalContext.current
    var video by remember(media) { mutableStateOf<File?>(null) }
    var failed by remember(session, media) { mutableStateOf(false) }
    val cacheDir = context.cacheDir
    DisposableEffect(media) { onDispose { video?.delete() } }
    LaunchedEffect(session, media) {
        try {
            video = downloadTimelineVideo(timelineMediaUrl(session, media), cacheDir)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            failed = true
        }
    }
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(media.name, color = colors.foreground) },
        containerColor = colors.background,
        text = {
            Box(Modifier.fillMaxWidth().heightIn(min = 80.dp, max = 340.dp), contentAlignment = Alignment.Center) {
                val file = video
                when {
                    failed -> Text("プレビューを表示できません", color = colors.foreground)
                    file != null -> AndroidView(
                        factory = { ctx ->
                            VideoView(ctx).apply {
                                val controls = MediaController(ctx)
                                controls.setAnchorView(this)
                                setMediaController(controls)
                                setOnErrorListener { _, _, _ -> failed = true; true }
                                setOnPreparedListener { start() }
                                setVideoPath(file.absolutePath)
                            }
                        },
                        modifier = Modifier.fillMaxWidth().height(220.dp),
                        onRelease = { it.stopPlayback() },
                    )
                    else -> CircularProgressIndicator(color = colors.accent)
                }
            }
        },
        confirmButton = { DialogAction("閉じる", UiIcon.Close, click = dismiss) },
        dismissButton = { DialogAction("元のファイル", UiIcon.External, click = openOriginal) },
    )
}

private const val MaxVideoBytes = 64L * 1024 * 1024
private val previewVideoClient = OkHttpClient.Builder()
    .followRedirects(false)
    .followSslRedirects(false)
    .callTimeout(90, TimeUnit.SECONDS)
    .dns(object : Dns {
        override fun lookup(hostname: String): List<InetAddress> {
            val addresses = Dns.SYSTEM.lookup(hostname)
            if (addresses.isEmpty() || !addresses.all(::isPublicVideoAddress)) {
                throw UnknownHostException("Video host must resolve to public addresses")
            }
            return addresses
        }
    }).build()

private fun isPublicVideoAddress(address: InetAddress): Boolean {
    if (address.isAnyLocalAddress || address.isLoopbackAddress || address.isLinkLocalAddress ||
        address.isSiteLocalAddress || address.isMulticastAddress) return false
    val bytes = address.address
    val first = bytes[0].toInt() and 255
    val second = bytes[1].toInt() and 255
    return if (bytes.size == 4) first != 0 && first < 224 && (first != 100 || second !in 64..127)
    else first in 0x20..0x3f
}

/** Download to an app-private bounded file so the platform player never follows a remote redirect. */
private suspend fun downloadTimelineVideo(rawUrl: String, cacheDir: File): File =
    suspendCancellableCoroutine { continuation ->
        val url = rawUrl.toHttpUrl()
        val active = AtomicReference<Call?>()
        continuation.invokeOnCancellation { active.get()?.cancel() }
        fun start(target: HttpUrl, depth: Int) {
            if (!continuation.isActive) return
            if (!target.isHttps || target.username.isNotEmpty() || target.password.isNotEmpty() ||
                target.host.none { it in 'a'..'z' } || ':' in target.host || depth >= 6) {
                continuation.resumeWithException(IOException("Invalid video destination"))
                return
            }
            val call = previewVideoClient.newCall(Request.Builder().url(target).build())
            active.set(call)
            if (!continuation.isActive) call.cancel()
            call.enqueue(object : Callback {
                override fun onFailure(call: Call, e: IOException) {
                    if (continuation.isActive) continuation.resumeWithException(e)
                }

                override fun onResponse(call: Call, response: Response) {
                    response.use {
                        var partial: File? = null
                        try {
                            if (response.isRedirect) {
                                val next = response.header("Location")?.let(target::resolve) ?: error("Missing video redirect")
                                start(next, depth + 1)
                                return
                            }
                            require(response.isSuccessful)
                            val body = requireNotNull(response.body)
                            require(body.contentLength() <= MaxVideoBytes)
                            val file = File.createTempFile("zoo-preview-", ".video", cacheDir)
                            partial = file
                            var bytes = 0L
                            body.byteStream().use { input ->
                                file.outputStream().buffered().use { output ->
                                    val buffer = ByteArray(32 * 1024)
                                    while (continuation.isActive) {
                                        val count = input.read(buffer)
                                        if (count < 0) break
                                        bytes += count
                                        require(bytes <= MaxVideoBytes)
                                        output.write(buffer, 0, count)
                                    }
                                }
                            }
                            if (continuation.isActive && bytes > 0) {
                                val header = file.inputStream().buffered().use { stream ->
                                    ByteArray(12) { stream.read().toByte() }
                                }
                                require(supportedVideoContainer(header))
                                continuation.resume(file) { _, unused, _ -> unused.delete() }
                                partial = null
                            } else if (continuation.isActive) error("Empty video")
                        } catch (e: Exception) {
                            if (continuation.isActive) continuation.resumeWithException(e)
                        } finally {
                            partial?.delete()
                        }
                    }
                }
            })
        }
        start(url, 0)
    }


internal fun supportedVideoContainer(header: ByteArray): Boolean =
    header.size >= 8 && (
        header.decodeToString(4, 8) == "ftyp" ||
            header.decodeToString(0, 4) == "OggS" ||
            header[0] == 0x1a.toByte() && header[1] == 0x45.toByte() &&
            header[2] == 0xdf.toByte() && header[3] == 0xa3.toByte()
        )
