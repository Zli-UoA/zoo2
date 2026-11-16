package zoo

import android.graphics.Color
import android.graphics.Paint
import android.graphics.pdf.PdfDocument
import androidx.test.core.app.ApplicationProvider
import java.io.File
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.test.*
import kotlinx.coroutines.*
import okhttp3.*
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class NativeFilePreviewTest {
    private fun temporaryFile() = File.createTempFile("native-preview-test-", ".tmp",
        ApplicationProvider.getApplicationContext<android.content.Context>().cacheDir)

    @Test fun webViewIsDetachedBeforeItsNativeResourcesAreDestroyed() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val parent = android.widget.FrameLayout(context)
        var destroyed = false
        val view = object : android.webkit.WebView(context) {
            override fun destroy() {
                assertNull(this.parent)
                destroyed = true
                super.destroy()
            }
        }
        parent.addView(view)
        releasePreviewWebView(view)
        assertTrue(destroyed)
        assertEquals(0, parent.childCount)
    }

    @Test fun rendersPdfPagesAndReusesTheDownloadedFile() {
        val file = temporaryFile()
        try {
            val pdf = PdfDocument()
            try {
                repeat(2) { index ->
                    val page = pdf.startPage(PdfDocument.PageInfo.Builder(300, 400, index + 1).create())
                    page.canvas.drawColor(if (index == 0) Color.RED else Color.BLUE)
                    page.canvas.drawRect(30f, 30f, 70f, 70f, Paint().apply { color = Color.BLACK })
                    pdf.finishPage(page)
                }
                file.outputStream().use { pdf.writeTo(it) }
            } finally { pdf.close() }
            val first = renderNativePdfPage(file, 0)
            val second = renderNativePdfPage(file, 1)
            try {
                assertEquals(2, first.pages)
                assertEquals(2, second.pages)
                assertTrue(first.bitmap.width <= 1600 && first.bitmap.height <= 2400)
                assertEquals(Color.RED, first.bitmap.getPixel(10, 10))
                assertEquals(Color.BLUE, second.bitmap.getPixel(10, 10))
                assertTrue(file.length() > 0, "Changing pages must reuse the source")
            } finally { first.bitmap.recycle(); second.bitmap.recycle() }
        } finally { file.delete() }
    }

    @Test fun rejectsOversizedPreviewAndRemovesItsTemporaryFile() = runBlocking {
        val file = temporaryFile()
        val client = OkHttpClient.Builder().addInterceptor { chain ->
            Response.Builder().request(chain.request()).protocol(Protocol.HTTP_1_1).code(200).message("OK")
                .body(object : ResponseBody() {
                    override fun contentType() = null
                    override fun contentLength() = 65L * 1024 * 1024
                    override fun source() = okio.Buffer()
                }).build()
        }.build()
        try {
            assertFailsWith<IOException> { downloadPreviewFile(client, "https://example.test/file", file) }
            assertFalse(file.exists())
        } finally { file.delete(); client.dispatcher.executorService.shutdown() }
    }

    @Test fun closingDuringDownloadCancelsTheCallAndDeletesThePartialFile() = runBlocking {
        val file = temporaryFile()
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        var captured: Call? = null
        val client = OkHttpClient.Builder().addInterceptor { chain ->
            captured = chain.call()
            started.countDown()
            check(release.await(10, TimeUnit.SECONDS))
            throw IOException("Viewer closed")
        }.build()
        try {
            val download = launch(start = CoroutineStart.UNDISPATCHED) {
                downloadPreviewFile(client, "https://example.test/file", file)
            }
            assertTrue(withContext(Dispatchers.IO) { started.await(10, TimeUnit.SECONDS) })
            download.cancelAndJoin()
            assertTrue(captured!!.isCanceled())
            assertFalse(file.exists())
        } finally { release.countDown(); file.delete(); client.dispatcher.executorService.shutdown() }
    }
}
