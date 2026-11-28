package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color as AndroidColor
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowDialog
import zoo.GlobalSearchType
import zoo.SearchPeriod

/** iOS にあって Android になかった表示（入力中、添付ごとのセンシティブ、参加中のスレッド、検索の期間）。 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class IosParitySamplesTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private val messages = listOf(
        TimelineItem("m1", "みずき", "リリースノートの下書きを置きました。", "11:02", dateDivider = "今日"),
        TimelineItem("m2", "なぎ", "スクリーンショットも追加します。", "11:05"),
    )

    @Test
    fun conversationShowsWhoIsTypingAndSensitiveAttachments() {
        val actions = mutableListOf<AndroidUiAction>()
        var dark by mutableStateOf(false)
        var input by mutableStateOf(InputState("スクショを添付します", files = listOf("f1" to "screen.png", "f2" to "draft.pdf"),
            sensitiveFiles = setOf("f1"), typing = "みずき、なぎが入力中…"))
        ui.setContent {
            AndroidScreens(AndroidScreen.Conversation("dev", "開発", "", messages, input), dark, true, dispatch = { action ->
                actions += action
                if (action is AndroidUiAction.ToggleFileSensitive) {
                    val current = input.sensitiveFiles.orEmpty()
                    input = input.copy(sensitiveFiles = if (action.id in current) current - action.id else current + action.id)
                }
            }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithTag("typing-indicator").assertTextEquals("みずき、なぎが入力中…")
        ui.onNodeWithContentDescription("screen.png: センシティブ").assertHeightIsAtLeast(48.dp).assertIsOn()
        capture("conversation-typing-sensitive-light")
        ui.runOnIdle { dark = true }
        capture("conversation-typing-sensitive-dark")
        ui.onNodeWithContentDescription("draft.pdf: センシティブ").assertIsOff().performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.ToggleFileSensitive("f2"), actions.last()) }
        ui.onNodeWithContentDescription("draft.pdf: センシティブ").assertIsOn()
    }

    @Test
    fun encryptedConversationsOfferNoPerFileToggle() {
        ui.setContent {
            AndroidScreens(AndroidScreen.Conversation("dev", "開発", "", messages,
                InputState("", files = listOf("f1" to "screen.png"), sensitiveFiles = null)), false, true, dispatch = {},
                language = "ja") { _, _ -> }
        }
        ui.onAllNodesWithTag("file-sensitive").assertCountEquals(0)
        ui.onNodeWithTag("typing-indicator").assertDoesNotExist()
    }

    @Test
    fun joinedThreadsSitUnderTheirChannel() {
        val actions = mutableListOf<AndroidUiAction>()
        var dark by mutableStateOf(false)
        val screen = AndroidScreen.Channels(listOf(
            ChannelSection("ピン留め", listOf(ChannelItem("ops", "運営"))),
            ChannelSection("参加中", listOf(
                ChannelItem("dev", "開発", unread = true, threads = listOf(
                    ChannelThreadItem("t1", "リリース手順の見直し", 12),
                    ChannelThreadItem("t2", "無題のスレッド", 3),
                )),
                ChannelItem("random", "雑談", threads = listOf(ChannelThreadItem("t3", "週末の予定", 5))),
            )),
            ChannelSection("ダイレクトメッセージ", listOf(ChannelItem("dm", "みずき", isUserName = true)), dm = true),
        ), moreThreads = true)
        ui.setContent { AndroidScreens(screen, dark, true, dispatch = { actions += it }, language = "ja") { _, _ -> } }
        capture("channels-threads-light")
        ui.runOnIdle { dark = true }
        capture("channels-threads-dark")
        ui.onNodeWithText("リリース手順の見直し").assertHeightIsAtLeast(48.dp).performClick()
        ui.onNodeWithText("さらに表示").performClick()
        ui.runOnIdle {
            assertEquals(listOf(AndroidUiAction.OpenChannelThread("dev", "t1"), AndroidUiAction.LoadMoreThreads), actions.takeLast(2))
        }
    }

    @Test
    fun aFailedThreadLoadOffersRetry() {
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(AndroidScreen.Channels(listOf(ChannelSection("参加中", listOf(ChannelItem("dev", "開発")))),
                moreThreads = true, threadsFailed = true), false, true, dispatch = { actions += it }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithTag("threads-error").assertExists()
        ui.onNodeWithText("さらに表示").assertDoesNotExist()
        ui.onNodeWithText("再試行").performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.RetryThreads, actions.last()) }
    }

    @Test
    @Config(qualifiers = "ja-rJP-w640dp-h360dp-xhdpi")
    fun theCustomPeriodCanBeAppliedOnALowScreen() {
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(AndroidScreen.Search(SearchScreenState("会議", "会議", GlobalSearchType.All, loaded = true,
                filters = SearchFilterState(period = SearchPeriod.Custom, after = "1999-01-05", endDay = "2026-11-20", today = "2026-11-28"))),
                false, true, dispatch = { actions += it }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithTag("search-filter-period").performScrollTo().performClick()
        ui.onNodeWithTag("search-period-apply").performScrollTo().performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.SelectSearchPeriod("1999-01-05", "2026-11-21"), actions.last()) }
    }

    @Test
    fun searchPeriodIsPickedInASheet() {
        val actions = mutableListOf<AndroidUiAction>()
        var dark by mutableStateOf(false)
        var filters by mutableStateOf(SearchFilterState(today = "2026-11-28"))
        ui.setContent {
            AndroidScreens(AndroidScreen.Search(SearchScreenState("会議", "会議", GlobalSearchType.All, loaded = true, filters = filters)),
                dark, true, dispatch = { action ->
                    actions += action
                    if (action is AndroidUiAction.SelectSearchPeriod) filters = filters.copy(
                        period = if (action.after == "2026-11-22" && action.before == null) SearchPeriod.Last7 else SearchPeriod.Custom,
                        after = action.after, endDay = action.before)
                }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithTag("search-filter-period").assertIsNotSelected().performScrollTo().performClick()
        ui.onNodeWithTag("search-period-last7").performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.SelectSearchPeriod("2026-11-22", null), actions.last()) }
        ui.onNodeWithTag("search-filter-period").assertIsSelected()
        ui.onNodeWithTag("search-filter-reset").assertExists()
        capture("search-period-chip-light")
        ui.onNodeWithTag("search-filter-period").performScrollTo().performClick()
        ui.onNodeWithTag("search-period-custom").performClick()
        capture("search-period-sheet-light", withSheet = true)
        ui.runOnIdle { dark = true }
        capture("search-period-sheet-dark", withSheet = true)
    }

    @Test
    fun recentCustomEmojiComeFirstInTheCustomTab() {
        val image = Bitmap.createBitmap(48, 48, Bitmap.Config.ARGB_8888).apply { eraseColor(AndroidColor.rgb(255, 128, 0)) }
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(false), unicode = emptyList(),
                custom = listOf(
                    EmojiChoice(":party:", "", "party", "fixture", category = "お祝い"),
                    EmojiChoice(":zoo:", "", "zoo", "fixture", category = "Zoo"),
                ),
                recent = listOf(":zoo:"),
                loadKitchen = { null }, loadImage = { image.asImageBitmap() }, dismiss = {}, select = {},
            )
        }
        ui.onNodeWithText("カスタム").performClick()
        ui.onNodeWithText("最近使った絵文字").assertIsDisplayed()
        ui.onAllNodesWithContentDescription("zoo").assertCountEquals(2)
        capture("emoji-picker-recent-custom-light", withSheet = true)
    }

    private fun capture(name: String, withSheet: Boolean = false) {
        ui.runOnIdle {
            val view = ui.activity.findViewById<View>(android.R.id.content)
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                val canvas = Canvas(bitmap)
                view.draw(canvas)
                if (withSheet) {
                    val dialog = checkNotNull(ShadowDialog.getLatestDialog()?.window).decorView
                    canvas.translate((view.width - dialog.width) / 2f, (view.height - dialog.height).toFloat())
                    dialog.draw(canvas)
                }
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { bitmap.recycle() }
        }
    }
}
