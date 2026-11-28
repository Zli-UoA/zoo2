package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color as AndroidColor
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
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
    fun customTabPrefetchesEveryDistinctImageRegardlessOfSearch() {
        val images = List(200) { "image-$it" }
        val custom = (images + images.first()).mapIndexed { index, image ->
            EmojiChoice(":emoji-$index:", "", "emoji-$index", image)
        }
        val batches = mutableListOf<List<String>>()
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(false), unicode = emptyList(), custom = custom,
                loadKitchen = { null }, loadImage = { null }, dismiss = {},
                prefetchCustomImages = { batches.add(it) }, select = {},
            )
        }
        ui.runOnIdle { assertTrue(batches.isEmpty()) }
        ui.onNodeWithText("カスタム").performClick()
        ui.onNodeWithTag("emoji-search").performTextInput("emoji-199")
        ui.runOnIdle { assertEquals(listOf(images), batches) }
    }

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

    @Test
    fun recentEmojiAlsoRemainsInItsCategoryAndSearchShowsItOnce() {
        val cat = EmojiChoice("🐱", "🐱", "cat", keywords = "猫", category = "動物と自然")
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(false), unicode = listOf(cat), recent = listOf(cat.value),
                custom = emptyList(), loadKitchen = { null }, loadImage = { null }, dismiss = {}, select = {},
            )
        }
        ui.onNodeWithText("最近使った絵文字").assertIsDisplayed()
        ui.onNodeWithText("動物と自然").assertIsDisplayed()
        ui.onAllNodesWithContentDescription("cat").assertCountEquals(2)
        ui.onNodeWithTag("emoji-search").performTextInput("猫")
        ui.onAllNodesWithContentDescription("cat").assertCountEquals(1)
        ui.onNodeWithText("最近使った絵文字").assertDoesNotExist()
    }

    @Test
    fun unicodeSearchKeepsCategoryOrderAndRanksNameMatchesFirst() {
        val faces = "スマイリーと感情"
        val animals = "動物と自然"
        val choices = listOf(
            EmojiChoice("😀", "😀", "grinning", category = faces),
            EmojiChoice("🐦‍🔥", "🐦‍🔥", "phoenix", keywords = "red fire bird rebirth rising rare", category = animals),
            EmojiChoice("🦍", "🦍", "gorilla", category = animals),
            EmojiChoice("🦏", "🦏", "rhinoceros", category = animals),
        )
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(false), unicode = choices, custom = emptyList(),
                loadKitchen = { null }, loadImage = { null }, dismiss = {}, select = {},
            )
        }
        ui.onNodeWithTag("emoji-search").performTextInput("r")
        ui.onNodeWithText(faces).assertIsDisplayed()
        ui.onNodeWithText(animals).assertIsDisplayed()
        val names = choices.map { it.name }.toSet()
        val order = ui.onAllNodes(hasClickAction()).fetchSemanticsNodes().mapNotNull { node ->
            if (SemanticsProperties.ContentDescription in node.config) {
                node.config[SemanticsProperties.ContentDescription].firstOrNull()
            } else null
        }.filter { it in names }
        assertEquals(listOf("grinning", "rhinoceros", "gorilla", "phoenix"), order)
    }

    @Test
    fun thePickerHandleMovesBeforeRelease() {
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(false), unicode = listOf(EmojiChoice("🌱", "🌱", "seedling")),
                custom = emptyList(), loadKitchen = { null }, loadImage = { null }, dismiss = {}, select = {},
            )
        }
        val handle = ui.onNodeWithTag("emoji-sheet-handle", useUnmergedTree = true)
        val initialTop = handle.fetchSemanticsNode().boundsInRoot.top
        val distance = with(ui.density) { 120.dp.toPx() }
        handle.performTouchInput { down(center); moveBy(Offset(0f, -distance)) }
        assertTrue(handle.fetchSemanticsNode().boundsInRoot.top < initialTop - distance / 2)
        handle.performTouchInput { up() }
    }

    private fun capture(name: String) {
        ui.recordControlBounds(name)
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
