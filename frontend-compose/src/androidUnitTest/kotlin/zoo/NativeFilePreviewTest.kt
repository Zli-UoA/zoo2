package zoo

import androidx.test.core.app.ApplicationProvider
import java.io.File
import java.io.IOException
import java.net.InetAddress
import java.net.UnknownHostException
import java.net.InetSocketAddress
import java.net.Proxy
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
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

    @Test fun avifUsesTheNativeDecoderOnlyOnSupportedAndroidVersions() {
        assertFalse(nativeImagePreviewSupported("image/avif", 26))
        assertFalse(nativeImagePreviewSupported("Image/AVIF; q=1", 30))
        assertTrue(nativeImagePreviewSupported("image/avif", 31))
        assertTrue(nativeImagePreviewSupported("image/png", 26))
        assertTrue(nativeImagePreviewSupported("image/jpeg", 30))
    }

    @Test fun privateDnsIsAnOrdinaryAsyncFailureWithoutAnUncaughtWorkerException() = runBlocking {
        val uncaught = AtomicReference<Throwable?>()
        val executor = Executors.newCachedThreadPool { task -> Thread(task).apply {
            uncaughtExceptionHandler = Thread.UncaughtExceptionHandler { _, error -> uncaught.set(error) }
        } }
        val client = previewFileClient("api.example.test").newBuilder()
            .dns(PreviewDns("api.example.test", object : Dns { override fun lookup(hostname: String) = listOf(InetAddress.getByName("127.0.0.1")) }))
            .dispatcher(Dispatcher(executor)).build()
        val file = temporaryFile()
        try {
            assertFailsWith<UnknownHostException> { downloadPreviewFile(client, "https://peer.example.test/file", file) }
            assertFalse(file.exists())
        } finally {
            executor.shutdown()
            assertTrue(executor.awaitTermination(5, TimeUnit.SECONDS))
            file.delete()
        }
        assertNull(uncaught.get(), "Rejected DNS must not terminate the Android process")
    }

    @Test fun privateIpLiteralsAreRejectedBeforeAConnectionIsAttempted() {
        var connections = 0
        val client = previewFileClient("api.example.test").newBuilder().eventListener(object : EventListener() {
            override fun connectStart(call: Call, inetSocketAddress: InetSocketAddress, proxy: Proxy) { connections++ }
        }).build()
        for (host in listOf("127.0.0.1", "192.168.1.2", "10.0.0.2", "169.254.1.1", "100.64.0.1", "[::1]", "[fc00::1]", "[fe80::1]", "[::ffff:127.0.0.1]")) {
            assertFailsWith<UnknownHostException>(host) {
                client.newCall(Request.Builder().url("https://$host/file").build()).execute().close()
            }
        }
        assertEquals(0, connections)
    }

    @Test fun publicLiteralsAndTheConfiguredApiHostRemainAllowed() {
        for (host in listOf("8.8.8.8", "2606:4700:4700::1111")) validatePreviewLiteralHost(host, "api.example.test")
        validatePreviewLiteralHost("127.0.0.1", "127.0.0.1")
        val address = InetAddress.getByName("127.0.0.1")
        assertEquals(listOf(address), PreviewDns("api.example.test", object : Dns { override fun lookup(hostname: String) = listOf(address) }).lookup("api.example.test"))
    }

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
