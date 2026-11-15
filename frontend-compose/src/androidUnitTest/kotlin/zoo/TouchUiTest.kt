package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import java.io.File
import kotlin.test.*
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w1200dp-h1000dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@OptIn(ExperimentalTestApi::class)
class TouchUiTest {
    @get:Rule val ui = createComposeRule()

    @Test
    fun bottomNavigationRetainsSelectionBadgeAndTargetsAtPhoneWidths() {
        var path by mutableStateOf("/home")
        var width by mutableStateOf(320.dp)
        var dark by mutableStateOf(false)
        ui.setContent {
            CompositionLocalProvider(LocalTouchUi provides true) {
                MaterialTheme(colorScheme = if (dark) darkColorScheme() else lightColorScheme()) {
                    Box(Modifier.width(width).testTag("navigation")) {
                        MobileNavigation(path, 120, { it }, "未読通知120件") { path = it.path }
                    }
                }
            }
        }
        ui.onNodeWithContentDescription("home.title").assertIsSelected()
        ui.onNodeWithContentDescription("notifications.title")
            .assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).performClick()
        ui.onNodeWithContentDescription("notifications.title").assertIsSelected()
        ui.onNodeWithText("99+").assertExists()
        screenshot("navigation", "navigation-320-light")
        ui.runOnIdle { width = 600.dp; dark = true }
        ui.onNodeWithContentDescription("channels.title").performClick().assertIsSelected()
        ui.onNodeWithContentDescription("settings.zooTitle").assertIsDisplayed()
        ui.onNodeWithContentDescription("bubbleChat.title").assertIsDisplayed()
        screenshot("navigation", "navigation-600-dark")
    }

    @Test
    fun smallControlsHaveNonOverlappingTouchTargetsAndDisabledActionsStayDisabled() {
        var clicks = 0
        ui.setContent {
            CompositionLocalProvider(LocalTouchUi provides true) {
                MaterialTheme {
                    Row {
                        IconButton("search", "検索", size = 24.dp) { clicks++ }
                        IconButton("x", "閉じる", enabled = false, size = 24.dp) { clicks++ }
                    }
                }
            }
        }
        val search = ui.onNodeWithContentDescription("検索")
        val close = ui.onNodeWithContentDescription("閉じる")
        search.assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).performClick()
        close.assertIsNotEnabled()
        assertTrue(search.fetchSemanticsNode().boundsInRoot.right <= close.fetchSemanticsNode().boundsInRoot.left)
        ui.runOnIdle { assertEquals(1, clicks) }
    }

    @Test
    fun editorAndErrorRecoveryRemainUsableWithLargeJapaneseText() {
        var value by mutableStateOf("")
        var retries = 0
        ui.setContent {
            CompositionLocalProvider(
                LocalTouchUi provides true,
                LocalDensity provides Density(LocalDensity.current.density, fontScale = 1.8f),
            ) {
                MaterialTheme {
                    Column(Modifier.width(320.dp).testTag("editor")) {
                        Field(value, "投稿内容", Modifier.fillMaxWidth(), singleLine = false) { value = it }
                        LoadErrorState("チャンネルを読み込めませんでした", "再試行") { retries++ }
                    }
                }
            }
        }
        ui.onNodeWithContentDescription("投稿内容").performTextInput("日本語の下書き\n返信を入力")
        ui.onNodeWithText("再試行").performClick()
        ui.runOnIdle {
            assertEquals("日本語の下書き\n返信を入力", value)
            assertEquals(1, retries)
        }
        screenshot("editor", "editor-320-font180")
    }

    private fun screenshot(tag: String, name: String) {
        val directory = File(System.getProperty("zoo.uiScreenshotDir", "build/reports/ui")).apply { mkdirs() }
        val bitmap = ui.onNodeWithTag(tag).captureToImage().asAndroidBitmap()
        File(directory, "$name.png").outputStream().use {
            check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it))
        }
    }
}
