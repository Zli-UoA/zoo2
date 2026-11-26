package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color as AndroidColor
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowDialog

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EmojiPickerSearchSampleTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun customEmojiSearchShowsTheExactMatchFirst() {
        val image = Bitmap.createBitmap(48, 48, Bitmap.Config.ARGB_8888).apply { eraseColor(AndroidColor.rgb(255, 128, 0)) }
        val names = listOf("xkusa", "kusa_hayasu", "kusa", "kusaa")
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(false), unicode = emptyList(),
                custom = names.map { EmojiChoice(":$it:", "", it, "fixture-emoji") },
                loadKitchen = { null }, loadImage = { image.asImageBitmap() }, dismiss = {}, select = {},
            )
        }
        ui.onNodeWithText("カスタム").performClick()
        ui.onNodeWithTag("emoji-search").performTextInput("kusa")
        ui.waitForIdle()
        val order = ui.onAllNodes(hasClickAction()).fetchSemanticsNodes().mapNotNull { node ->
            if (SemanticsProperties.ContentDescription in node.config) node.config[SemanticsProperties.ContentDescription].firstOrNull() else null
        }.filter { it in names }
        assertEquals(listOf("kusa", "kusaa", "kusa_hayasu", "xkusa"), order)
        capture("emoji-picker-search-exact-first")
    }

    private fun capture(name: String) {
        ui.runOnIdle {
            val view = ui.activity.findViewById<View>(android.R.id.content)
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                val canvas = Canvas(bitmap)
                view.draw(canvas)
                val dialog = checkNotNull(ShadowDialog.getLatestDialog()?.window).decorView
                canvas.translate((view.width - dialog.width) / 2f, (view.height - dialog.height).toFloat())
                dialog.draw(canvas)
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { bitmap.recycle() }
        }
    }
}
