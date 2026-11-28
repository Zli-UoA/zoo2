package zoo.android

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import zoo.Session
import zoo.android.ui.*

/** 添付の PDF をアプリ内で開く（iOS の `PDFPreviewView`）。平文の添付は署名付き URL から、暗号化した添付は復号した内容から開く。 */
internal sealed interface DocumentSource {
    val name: String
    data class Signed(val attachmentId: String, override val name: String) : DocumentSource
    class Decrypted(val bytes: ByteArray, override val name: String) : DocumentSource
}

/** PDF として開く添付か（名前で判定する。中身は開くときに `%PDF-` で確かめる）。 */
internal fun isPdfAttachment(name: String): Boolean = name.substringAfterLast('.', "").equals("pdf", ignoreCase = true)

internal fun isPdfHeader(header: ByteArray): Boolean = header.size >= 5 && header.decodeToString(0, 5) == "%PDF-"

/**
 * 端末の [PdfRenderer] で 1 ページずつ描く。PdfRenderer は同時に 1 ページしか開けないので、描画を順に行う。
 * 描いたページは表示幅に合わせた大きさで、最大 [MAX_CACHED_PAGES] 枚だけ残す。
 */
internal class PdfPages(file: File) : AutoCloseable {
    private val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    private val renderer = try { PdfRenderer(descriptor) } catch (failure: Exception) { descriptor.close(); throw failure }
    private val lock = Mutex()
    private val cache = object : LinkedHashMap<Pair<Int, Int>, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Pair<Int, Int>, Bitmap>) = size > MAX_CACHED_PAGES
    }
    val count: Int get() = renderer.pageCount

    /** ページの縦横比（高さ ÷ 幅）。 */
    suspend fun ratio(index: Int): Float = lock.withLock {
        withContext(Dispatchers.IO) { renderer.openPage(index).use { it.height.toFloat() / it.width.coerceAtLeast(1) } }
    }

    suspend fun render(index: Int, width: Int): Bitmap = lock.withLock {
        cache[index to width]?.let { return@withLock it }
        withContext(Dispatchers.IO) {
            renderer.openPage(index).use { page ->
                val target = width.coerceIn(1, MAX_PAGE_WIDTH)
                val height = (target.toFloat() * page.height / page.width.coerceAtLeast(1)).toInt().coerceIn(1, MAX_PAGE_WIDTH * 4)
                Bitmap.createBitmap(target, height, Bitmap.Config.ARGB_8888).also { bitmap ->
                    bitmap.eraseColor(android.graphics.Color.WHITE)
                    page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                }
            }
        }.also { cache[index to width] = it }
    }

    override fun close() {
        cache.clear()
        renderer.close()
        descriptor.close()
    }

    private companion object {
        const val MAX_CACHED_PAGES = 6
        const val MAX_PAGE_WIDTH = 2048
    }
}

@Composable
internal fun DocumentPreview(
    session: Session, platform: AndroidPlatform, source: DocumentSource, dark: Boolean, dismiss: () -> Unit,
) {
    val context = LocalContext.current
    var file by remember(source) { mutableStateOf<File?>(null) }
    var pages by remember(source) { mutableStateOf<PdfPages?>(null) }
    var failed by remember(source) { mutableStateOf(false) }
    var attempt by remember(source) { mutableIntStateOf(0) }
    DisposableEffect(source) { onDispose { pages?.close(); file?.delete() } }
    LaunchedEffect(source, attempt) {
        failed = false
        pages?.close(); pages = null
        file?.delete(); file = null
        try {
            val downloaded = when (source) {
                is DocumentSource.Signed -> downloadPreviewFile(session.attachmentLinks.get(source.attachmentId), context.cacheDir,
                    ".pdf", MaxDocumentBytes, ::isPdfHeader)
                is DocumentSource.Decrypted -> withContext(Dispatchers.IO) {
                    require(isPdfHeader(source.bytes.copyOf(minOf(source.bytes.size, 8))))
                    File.createTempFile("zoo-preview-", ".pdf", context.cacheDir).apply { writeBytes(source.bytes) }
                }
            }
            file = downloaded
            pages = withContext(Dispatchers.IO) { PdfPages(downloaded) }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            failed = true
        }
    }
    val save = {
        when (source) {
            is DocumentSource.Signed -> session.attachmentLinks.download(zoo.obj("id" to source.attachmentId, "file_name" to source.name))
            is DocumentSource.Decrypted -> session.run {
                try { platform.saveToDownloads(source.bytes, source.name, "application/pdf") }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { session.toast = session.t("attachments.downloadError") }
            }
        }
    }
    val document = pages
    DocumentViewer(
        source.name, dark,
        state = when {
            failed -> DocumentViewerState.Failed
            document == null -> DocumentViewerState.Loading
            else -> DocumentViewerState.Ready(document.count)
        },
        ratio = { index -> document?.let { runCatching { it.ratio(index) }.getOrNull() } },
        page = { index, width -> document?.let { runCatching { it.render(index, width).asImageBitmap() }.getOrNull() } },
        save = save, retry = { attempt++ }, dismiss = dismiss,
    )
}

private const val MaxDocumentBytes = 64L * 1024 * 1024
