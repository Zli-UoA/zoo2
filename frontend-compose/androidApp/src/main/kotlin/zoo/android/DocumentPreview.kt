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
 * 描いたページは表示幅に合わせた大きさ（画素数に上限あり）で、最大 [MAX_CACHED_PAGES] 枚・[MAX_CACHED_BYTES] まで残す。
 */
internal class PdfPages(file: File) : AutoCloseable {
    private val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    private val renderer = try { PdfRenderer(descriptor) } catch (failure: Exception) { descriptor.close(); throw failure }
    private val lock = Mutex()
    private val closed = java.util.concurrent.atomic.AtomicBoolean(false)
    private var released = false
    private var cachedBytes = 0L
    private val cache = LinkedHashMap<Pair<Int, Int>, Bitmap>(8, 0.75f, true)
    val count: Int = renderer.pageCount

    /** ページの縦横比（高さ ÷ 幅）。 */
    suspend fun ratio(index: Int): Float = withRenderer {
        renderer.openPage(index).use { it.height.toFloat() / it.width.coerceAtLeast(1) }
    }

    suspend fun render(index: Int, width: Int): Bitmap {
        locked { cache[index to width] }?.let { return it }
        return withRenderer {
            renderer.openPage(index).use { page ->
                val (target, height) = pageSize(page.width, page.height, width)
                Bitmap.createBitmap(target, height, Bitmap.Config.ARGB_8888).also { bitmap ->
                    bitmap.eraseColor(android.graphics.Color.WHITE)
                    page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                }
            }
        }.also { bitmap -> locked { if (!closed.get()) remember(index to width, bitmap) } }
    }

    /** キャッシュは枚数とバイト数の両方で抑える。古く使ったものから捨てる。 */
    private fun remember(key: Pair<Int, Int>, bitmap: Bitmap) {
        cache.put(key, bitmap)?.let { cachedBytes -= it.allocationByteCount }
        cachedBytes += bitmap.allocationByteCount
        val entries = cache.entries.iterator()
        while ((cache.size > MAX_CACHED_PAGES || cachedBytes > MAX_CACHED_BYTES) && cache.size > 1 && entries.hasNext()) {
            val eldest = entries.next()
            if (eldest.key == key) continue
            cachedBytes -= eldest.value.allocationByteCount
            entries.remove()
        }
    }

    /**
     * レンダラーを使う処理を 1 つずつ行う。閉じたあとは新しく始めない。描画中に閉じられたら、
     * そのページを閉じ終えてから（ロックを離したところで）レンダラーを解放する。
     */
    private suspend fun <T> withRenderer(block: () -> T): T = locked {
        check(!closed.get()) { "closed" }
        withContext(Dispatchers.IO + kotlinx.coroutines.NonCancellable) { block() }
    }

    private suspend inline fun <T> locked(block: () -> T): T = try { lock.withLock { block() } } finally { releaseIfClosed() }

    private fun releaseIfClosed() {
        if (!closed.get() || !lock.tryLock()) return
        try {
            if (released) return
            released = true
            cache.clear()
            cachedBytes = 0
            renderer.close()
            descriptor.close()
        } finally { lock.unlock() }
    }

    /** 画面から外すときに呼ぶ（UI スレッドから呼んでも、描画中のページを待たない）。 */
    override fun close() {
        closed.set(true)
        releaseIfClosed()
    }

    companion object {
        const val MAX_CACHED_PAGES = 6
        const val MAX_PAGE_WIDTH = 2048
        /** 1 ページの画素数の上限（ARGB_8888 で 16 MiB）。縦長のページは幅を縮めて収める。 */
        const val MAX_PAGE_PIXELS = 4L * 1024 * 1024
        const val MAX_CACHED_BYTES = 48L * 1024 * 1024

        /** 表示幅に合わせた描画の大きさ。幅は [MAX_PAGE_WIDTH]、画素数は [MAX_PAGE_PIXELS] までにする。 */
        fun pageSize(pageWidth: Int, pageHeight: Int, width: Int): Pair<Int, Int> {
            val ratio = pageHeight.coerceAtLeast(1).toDouble() / pageWidth.coerceAtLeast(1)
            var target = width.coerceIn(1, MAX_PAGE_WIDTH)
            val byPixels = kotlin.math.sqrt(MAX_PAGE_PIXELS / ratio).toInt().coerceAtLeast(1)
            target = minOf(target, byPixels)
            val height = (target * ratio).toInt().coerceIn(1, (MAX_PAGE_PIXELS / target).toInt().coerceAtLeast(1))
            return target to height
        }
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
