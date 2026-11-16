package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.activity.ComponentActivity
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import java.io.File
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.math.roundToInt
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
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private val labels = mapOf(
        "nav.main" to "メインナビゲーション", "home.title" to "Home",
        "channels.title" to "チャンネル", "notifications.title" to "通知",
        "settings.zooTitle" to "Zoo設定", "bubbleChat.title" to "Bubble Chat",
    )

    @Test
    fun bottomNavigationRetainsSelectionBadgeAndTargetsAtPhoneWidths() {
        var path by mutableStateOf("/home")
        var width by mutableStateOf(320.dp)
        var dark by mutableStateOf(false)
        ui.setContent {
            CompositionLocalProvider(LocalTouchUi provides true) {
                MaterialTheme(colorScheme = zooColorScheme(dark)) {
                    Box(Modifier.width(width).testTag("navigation")) {
                        MobileNavigation(path, 120, { labels.getValue(it) }, "未読通知120件") { path = it.path }
                    }
                }
            }
        }
        ui.onNodeWithContentDescription("Home").assertIsSelected()
        ui.onNodeWithContentDescription("通知")
            .assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).performClick()
        ui.onNodeWithContentDescription("通知").assertIsSelected()
        // NavigationBarItem hides decorative icon descendants; expose the full count on the tab.
        ui.onNodeWithContentDescription("通知").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "未読通知120件"),
        )
        screenshot("navigation", "navigation-320-light")
        ui.runOnIdle { width = 600.dp; dark = true }
        ui.onNodeWithContentDescription("チャンネル").performClick().assertIsSelected()
        ui.onNodeWithContentDescription("Zoo設定").assertIsDisplayed()
        ui.onNodeWithContentDescription("Bubble Chat").assertIsDisplayed()
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

    @Test
    fun designPrimitivesKeepTargetsAndSettingsUsableAcrossWidthsAndFontScales() {
        var width by mutableStateOf(320.dp)
        var dark by mutableStateOf(false)
        var scale by mutableStateOf(1f)
        var value by mutableStateOf("")
        ui.setContent {
            CompositionLocalProvider(
                LocalTouchUi provides true,
                LocalDensity provides Density(LocalDensity.current.density, fontScale = scale),
            ) {
                ZooTheme(dark) {
                    Column(Modifier.width(width).testTag("design-primitives")) {
                        ScreenHeader("settings", "表示と通知")
                        Column(
                            Modifier.fillMaxWidth().padding(AndroidDesign.screenInset),
                            verticalArrangement = Arrangement.spacedBy(AndroidDesign.spaceLarge),
                        ) {
                            Field(value, "日本語の表示名", Modifier.fillMaxWidth()) { value = it }
                            Field("", "返信", Modifier.fillMaxWidth(), chrome = false,
                                singleLine = false, minimumHeight = 36.dp) {}
                            Row(horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceMedium)) {
                                Button("保存", size = "xs", icon = "check") {}
                                IconButton("x", "キャンセル", size = 24.dp) {}
                            }
                            SettingRow("長い日本語の設定名", "通知の表示方法を変更します") {
                                Button("変更する", variant = "outline") {}
                            }
                        }
                    }
                }
            }
        }
        for ((nextWidth, nextScale, nextDark) in listOf(
            Triple(320.dp, 1f, false), Triple(320.dp, 1.8f, true), Triple(600.dp, 1f, true),
        )) {
            ui.runOnIdle { width = nextWidth; scale = nextScale; dark = nextDark }
            ui.onNodeWithContentDescription("保存").assertHeightIsAtLeast(48.dp)
            ui.onNodeWithContentDescription("キャンセル").assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp)
            ui.onNodeWithContentDescription("変更する").assertIsDisplayed().assertHeightIsAtLeast(48.dp)
            ui.onNodeWithContentDescription("日本語の表示名").assertHeightIsAtLeast(48.dp)
            ui.onNodeWithContentDescription("返信").assertHeightIsAtLeast(48.dp)
            val container = ui.onNodeWithTag("design-primitives").fetchSemanticsNode().boundsInRoot
            val control = ui.onNodeWithContentDescription("変更する").fetchSemanticsNode().boundsInRoot
            assertTrue(control.left >= container.left && control.right <= container.right)
            screenshot("design-primitives", "design-${nextWidth.value}-${nextScale}-${nextDark}")
        }
        ui.onNodeWithContentDescription("日本語の表示名").performTextInput("日本語の名前")
        ui.runOnIdle { assertEquals("日本語の名前", value) }
    }

    private fun screenshot(tag: String, name: String) {
        val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/ui").apply { mkdirs() }
        val bounds = ui.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot
        // PixelCopy used by captureToImage waits for a compositor frame on Robolectric API 35.
        // Draw the real content View with the native graphics backend instead.
        ui.runOnIdle {
            val content = ui.activity.findViewById<android.view.View>(android.R.id.content)
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

    @Test
    fun offlineComposerAllowsDraftingButNeverSendsOrUploads() {
        var requests = 0
        var sends = 0
        var editAllowed by mutableStateOf(true)
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val http = HttpClient(MockEngine { requests++; error("Offline composer made a request") })
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "offline-composer"
            override fun now() = "2026-11-15T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = mapOf(
                    "messages.compose" to "メッセージを送信",
                    "messages.send" to "送信",
                    "attachments.add" to "ファイルを添付",
                    "composer.editorTitle" to "メッセージの編集",
                    "composer.collapse" to "入力欄を小さく戻す",
                    "composer.formatting" to "書式設定",
                    "composer.expand" to "入力欄を全画面に広げる",
                    "composer.dragHandle" to "入力欄を上下にスワイプ"
                )[key] ?: key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
            override suspend fun invoke(name: String, payload: String) = ""
        }
        val app = Session(
            Repository(Config("https://zoo.test", "/api/shapes", "alice", false), platform, http, scope, synchronize = false),
            scope, Environment(online = false, touch = true),
        )
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides app) {
                    ZooTheme(dark = true) {
                        Box(Modifier.width(320.dp).height(800.dp).testTag("channel-editor")) {
                            NativeWindow(app) {
                                Column(Modifier.fillMaxSize()) {
                                    Spacer(Modifier.weight(1f))
                                    Composer(app, "general", null, emptyList(), enabled = editAllowed, showTyping = false,
                                        sendMessage = { _, _, _ -> sends++; true })
                                }
                            }
                        }
                    }
                }
            }
            ui.onNodeWithContentDescription("メッセージを送信").performTextInput("通信が戻るまで下書き")
            ui.onNodeWithContentDescription("送信").assertIsNotEnabled()
            ui.onNodeWithContentDescription("ファイルを添付").assertIsNotEnabled()
            screenshot("channel-editor", "channel-editor-compact")
            ui.onNodeWithTag("composer-drag-handle").performTouchInput { swipeUp(endY = -150f) }
            ui.onNodeWithContentDescription("書式設定").performClick()
            screenshot("channel-editor", "channel-editor-expanded-formatting")
            ui.runOnIdle { editAllowed = false }
            ui.onNodeWithContentDescription("メッセージを送信").assertIsNotEnabled()
            ui.runOnIdle { assertEquals(0, requests); assertEquals(0, sends) }
        } finally { app.close(); scope.cancel() }
    }
}
