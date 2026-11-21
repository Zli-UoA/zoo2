package zoo

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onNodeWithTag
import java.io.File
import kotlin.math.roundToInt

internal fun captureAndroidUi(ui: ComposeContentTestRule, activity: () -> ComponentActivity, tag: String, name: String) {
    val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/ui").apply { mkdirs() }
    val bounds = ui.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot
    // PixelCopy used by captureToImage waits for a compositor frame on Robolectric API 35.
    // Draw the real content View with the native graphics backend instead.
    ui.runOnIdle {
        val content = activity().findViewById<android.view.View>(android.R.id.content)
        val full = android.graphics.Bitmap.createBitmap(content.width, content.height, android.graphics.Bitmap.Config.ARGB_8888)
        try {
            content.draw(android.graphics.Canvas(full))
            val bitmap = android.graphics.Bitmap.createBitmap(
                full, bounds.left.roundToInt(), bounds.top.roundToInt(),
                bounds.width.roundToInt(), bounds.height.roundToInt(),
            )
            try {
                val pixels = IntArray(bitmap.width * bitmap.height)
                bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
                check(pixels.any { it != pixels.first() }) { "UI capture is blank: $name" }
                File(directory, "$name.png").outputStream().use {
                    check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it))
                }
            } finally { if (bitmap !== full) bitmap.recycle() }
        } finally { full.recycle() }
    }
}
