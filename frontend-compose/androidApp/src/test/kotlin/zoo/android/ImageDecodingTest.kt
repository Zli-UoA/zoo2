package zoo.android

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.test.runTest
import zoo.*
import java.io.ByteArrayOutputStream
import kotlin.test.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 28], manifest = Config.NONE)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ImageDecodingTest {
    @Test
    fun staticPreviewIconAndAvatarUseTheBoundedPlatformDecoder() = runTest {
        val original = Bitmap.createBitmap(4096, 32, Bitmap.Config.ARGB_8888)
        val stream = ByteArrayOutputStream()
        assertTrue(original.compress(Bitmap.CompressFormat.PNG, 100, stream))
        original.recycle()
        val bytes = stream.toByteArray()
        val requested = mutableListOf<String>()
        var decodes = 0
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "static-image"
            override fun now() = "2026-11-15T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
            override suspend fun image(url: String): ByteArray {
                requested += url
                return if (url.endsWith("/broken")) byteArrayOf(1, 2, 3) else bytes
            }
            override suspend fun refreshImage(url: String): ByteArray = bytes
            override fun imageFrames(bytes: ByteArray) = flow {
                decodes++
                emit(decodeOrientedBitmap(bytes).asImageBitmap())
            }
        }
        val app = Session(
            Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform,
                HttpClient(MockEngine { error("Unexpected API request") }), backgroundScope, synchronize = false),
            backgroundScope, Environment(),
        )
        try {
            for (kind in listOf("preview", "icon", "avatar")) {
                val image = app.imageBitmap("https://media.test/$kind")
                assertEquals(2048, image.width)
                assertEquals(16, image.height)
                image.asAndroidBitmap().recycle()
            }
            assertEquals(3, decodes)
            assertEquals(listOf("https://media.test/preview", "https://media.test/icon", "https://media.test/avatar"), requested)
            val broken = "https://media.test/broken"
            assertFailsWith<IllegalArgumentException> { app.imageBitmap(broken) }
            assertFailsWith<IllegalArgumentException> { app.imageBitmap(broken) }
            assertEquals(1, requested.count { it == broken })
            assertEquals(2048, app.imageBitmap(broken, retry = true).width)
            assertEquals(2048, app.cachedImageBitmap(broken)?.width)
        } finally { app.close() }
    }

    @Test
    fun largePreviewsAreSampledBeforeDecoding() {
        assertEquals(8, imageSampleSize(2048, 2048, maxEdge = 256))
        assertEquals(16, imageSampleSize(2049, 100, maxEdge = 256))
        assertEquals(1, imageSampleSize(2048, 2048))
        assertEquals(2, imageSampleSize(2049, 100))
        assertEquals(4, imageSampleSize(8000, 6000))
        assertEquals(4, imageSampleSize(100, 8192))
        assertEquals(1 shl 20, imageSampleSize(Int.MAX_VALUE, 1))
        assertFailsWith<IllegalArgumentException> { imageSampleSize(-1, 100) }
        assertFailsWith<IllegalArgumentException> { imageSampleSize(100, 0) }
    }

    @Test
    fun oversizedBitmapIsActuallyDecodedAtThePreviewSize() {
        val original = Bitmap.createBitmap(4096, 32, Bitmap.Config.ARGB_8888)
        val stream = ByteArrayOutputStream()
        assertTrue(original.compress(Bitmap.CompressFormat.PNG, 100, stream))
        original.recycle()
        val decoded = decodeOrientedBitmap(stream.toByteArray())
        assertEquals(2048, decoded.width)
        assertEquals(16, decoded.height)
        decoded.recycle()
    }

    @Test
    fun invalidBytesFailWithoutAllocatingAnImage() {
        assertFailsWith<IllegalArgumentException> { decodeOrientedBitmap(byteArrayOf(1, 2, 3)) }
    }

    private fun jpeg(orientation: Int): ByteArray {
        val bitmap = Bitmap.createBitmap(40, 20, Bitmap.Config.ARGB_8888)
        for (y in 0 until 20) for (x in 0 until 40) {
            bitmap.setPixel(x, y, if (x < 20) Color.RED else Color.BLUE)
        }
        val stream = ByteArrayOutputStream()
        assertTrue(bitmap.compress(Bitmap.CompressFormat.JPEG, 100, stream))
        bitmap.recycle()
        val jpeg = stream.toByteArray()
        val exif = byteArrayOf(69,120,105,102,0,0,73,73,42,0,8,0,0,0,1,0,18,1,3,0,1,0,0,0,orientation.toByte(),0,0,0,0,0,0,0)
        val length = exif.size + 2
        return byteArrayOf(0xff.toByte(),0xd8.toByte(),0xff.toByte(),0xe1.toByte(),(length shr 8).toByte(),length.toByte()) + exif + jpeg.copyOfRange(2, jpeg.size)
    }

    @Test
    fun rotatedAndMirroredJpegsMatchTheServerPreview() {
        for (orientation in 1..8) {
            val image = decodeOrientedBitmap(jpeg(orientation))
            val swapped = orientation >= 5
            assertEquals(if (swapped) 20 else 40, image.width, "EXIF $orientation")
            assertEquals(if (swapped) 40 else 20, image.height, "EXIF $orientation")
            val first = image.getPixel(if (swapped) 10 else 5, if (swapped) 5 else 10)
            val last = image.getPixel(if (swapped) 10 else 35, if (swapped) 35 else 10)
            val redFirst = orientation in listOf(1, 4, 5, 6)
            assertEquals(redFirst, Color.red(first) > Color.blue(first), "EXIF $orientation first half")
            assertEquals(!redFirst, Color.red(last) > Color.blue(last), "EXIF $orientation last half")
            image.recycle()
        }
    }

    @Test
    fun pngWithoutExifPreservesItsDimensions() {
        val original = Bitmap.createBitmap(3, 2, Bitmap.Config.ARGB_8888)
        val stream = ByteArrayOutputStream()
        assertTrue(original.compress(Bitmap.CompressFormat.PNG, 100, stream))
        val decoded = decodeOrientedBitmap(stream.toByteArray())
        assertEquals(3, decoded.width)
        assertEquals(2, decoded.height)
        decoded.recycle()
        original.recycle()
    }
}
