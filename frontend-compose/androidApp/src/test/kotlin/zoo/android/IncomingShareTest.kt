package zoo.android

import android.content.ClipData
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.net.Uri
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.test.*

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 28, 35])
class IncomingShareTest {
    private val app: Context get() = RuntimeEnvironment.getApplication()
    private val readable = Uri.parse("content://photos.example/photo/1")
    private val second = Uri.parse("content://photos.example/photo/2")
    private val recipient get() = object : ContextWrapper(app) {
        override fun checkUriPermission(uri: Uri, pid: Int, uid: Int, modeFlags: Int): Int =
            if (uri == readable || uri == second) PackageManager.PERMISSION_GRANTED else PackageManager.PERMISSION_DENIED
    }

    @Test fun acceptsTextAndKeepsClipOrderWithoutRepeatingImages() {
        val text = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_TEXT, "集合は北口です")
        assertEquals(IncomingShare("集合は北口です", emptyList()), recipient.incomingShare(text))
        val clip = ClipData.newRawUri("写真", second).apply {
            addItem(ClipData.Item(readable))
            addItem(ClipData.Item(second))
        }
        val intent = Intent(Intent.ACTION_SEND_MULTIPLE).apply { clipData = clip }
        assertEquals(listOf(second, readable), recipient.incomingShare(intent)?.images)
    }

    @Test fun refusesUnreadableAndPrivateContent() {
        for (uri in listOf(Uri.parse("file:///private.jpg"), Uri.parse("content://photos.example/unreadable"),
            Uri.parse("content://${app.packageName}.files/private"))) {
            assertNull(recipient.incomingShare(Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uri)))
        }
    }

    @Config(sdk = [26, 28])
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    @Test fun olderAndroidDecodesASharedPreviewWithinTheThumbnailBudget() {
        val file = java.io.File.createTempFile("share", ".png", app.cacheDir)
        try {
            val image = Bitmap.createBitmap(512, 128, Bitmap.Config.ARGB_8888)
            file.outputStream().use { assertTrue(image.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            image.recycle()
            val preview = assertNotNull(app.contentResolver.shareThumbnail(Uri.fromFile(file)))
            assertTrue(preview.width in 1..144)
            assertEquals(preview.width / 4, preview.height)
            preview.recycle()
        } finally { file.delete() }
    }
}
