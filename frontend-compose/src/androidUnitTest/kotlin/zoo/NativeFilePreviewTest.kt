package zoo

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
