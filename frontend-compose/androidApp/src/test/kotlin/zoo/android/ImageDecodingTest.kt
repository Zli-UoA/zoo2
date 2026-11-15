package zoo.android

import android.graphics.Bitmap
import android.graphics.Color
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
    fun largePreviewsAreSampledBeforeDecoding() {
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
