package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
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

/** iOS にあって Android になかった画面（PDF の表示、公開プロフィールの編集、タブレット向けの配置）。 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class IosParityLayoutSamplesTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private val messages = listOf(
        TimelineItem("m1", "みずき", "リリースノートの下書きを置きました。", "11:02", dateDivider = "今日",
            attachments = listOf("a1" to "release-notes.pdf")),
        TimelineItem("m2", "なぎ", "確認します。", "11:05"),
    )

    private val channels = AndroidScreen.Channels(listOf(
        ChannelSection("ピン留め", listOf(ChannelItem("ops", "運営"))),
        ChannelSection("参加中", listOf(
            ChannelItem("dev", "開発", threads = listOf(ChannelThreadItem("t1", "リリース手順の見直し", 12))),
            ChannelItem("random", "雑談", unread = true),
        )),
        ChannelSection("ダイレクトメッセージ", listOf(ChannelItem("dm", "みずき", isUserName = true)), dm = true),
    ), selected = "dev")

    /** 文字と罫線を描いた、PDF のページの代わり。 */
    private fun fakePage(index: Int, width: Int): Bitmap {
        val height = (width * 1.414f).toInt()
        return Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).apply {
            eraseColor(android.graphics.Color.WHITE)
            val canvas = Canvas(this)
            val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0xff202020.toInt(); textSize = width / 14f }
            canvas.drawText("リリースノート p.${index + 1}", width * 0.08f, height * 0.1f, paint)
            paint.color = 0xffbdbdbd.toInt()
            for (line in 0 until 14) canvas.drawRect(width * 0.08f, height * (0.16f + line * 0.05f),
                width * (if (line % 4 == 3) 0.6f else 0.92f), height * (0.16f + line * 0.05f) + width / 60f, paint)
        }
    }

    @Test
    fun pdfOpensFullScreenWithPagesAndSave() {
        var saved = 0
        var dark by mutableStateOf(false)
        ui.setContent {
            DocumentViewer("release-notes.pdf", dark, DocumentViewerState.Ready(3), ratio = { 1.414f },
                page = { index, width -> fakePage(index, width).asImageBitmap() }, save = { saved++ }, retry = {}, dismiss = {})
        }
        ui.onNodeWithContentDescription("release-notes.pdf 1 / 3").assertExists()
        ui.onNodeWithTag("document-page-number").assertTextEquals("1 / 3")
        capture("document-pdf-light", dialog = true)
        ui.runOnIdle { dark = true }
        capture("document-pdf-dark", dialog = true)
        ui.onNodeWithTag("document-save").performClick()
        ui.runOnIdle { assertEquals(1, saved) }
    }

    @Test
    fun pdfFailureOffersRetry() {
        var retried = false
        ui.setContent {
            DocumentViewer("broken.pdf", false, DocumentViewerState.Failed, ratio = { null }, page = { _, _ -> null },
                save = {}, retry = { retried = true }, dismiss = {})
        }
        ui.onNodeWithText("再試行").performClick()
        ui.runOnIdle { assertTrue(retried) }
    }

    @Test
    fun publicProfileEditorUsesTheProfileLabels() {
        val actions = mutableListOf<AndroidUiAction>()
        var dark by mutableStateOf(false)
        ui.setContent {
            AndroidScreens(AndroidScreen.EditChannel("プロフィールを編集", "", "Zoo の開発をしています。", "保存", true,
                nameLabel = "APub表示名", descriptionLabel = "自己紹介", nameHint = "はる",
                description2 = "連合先で使う表示名を設定します。\n100文字以内。空欄にすると通常の表示名を使用します。",
                notice = "保存しました"), dark, true, dispatch = { actions += it }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithText("はる").assertExists()
        ui.onNodeWithTag("editor-notice").assertTextEquals("保存しました")
        capture("profile-edit-light")
        ui.runOnIdle { dark = true }
        capture("profile-edit-dark")
        ui.onNodeWithText("保存").performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.SubmitChannel, actions.last()) }
    }

    @Test
    fun settingsListTheProfileEntries() {
        ui.setContent {
            AndroidScreens(AndroidScreen.Settings(listOf(
                SettingsGroup("プロフィール", listOf(
                    SettingsEntry.Destination("public-profile", "プロフィールを編集"),
                    SettingsEntry.Destination("account-profile", "名前・アイコンを編集"),
                    SettingsEntry.Destination("public-page", "公開ページを見る"),
                )),
                SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", "system"))),
            )), false, true, dispatch = {}, language = "ja") { _, _ -> }
        }
        ui.onNodeWithText("名前・アイコンを編集").assertExists()
        capture("settings-profile-light")
    }

    @Test
    @Config(qualifiers = "ja-rJP-w1024dp-h768dp-xhdpi")
    fun tabletShowsTheRailAndTheListBesideTheConversation() {
        val actions = mutableListOf<AndroidUiAction>()
        var dark by mutableStateOf(false)
        ui.setContent {
            AndroidScreens(AndroidScreen.Conversation("dev", "開発", "", messages, InputState()), dark, true,
                dispatch = { actions += it }, language = "ja", sidebar = channels) { _, _ -> }
        }
        ui.onNodeWithTag("navigation-rail").assertExists()
        ui.onNodeWithTag("channel-list-pane").assertExists()
        ui.onAllNodesWithText("開発").assertCountEquals(2) // 横の一覧と、会話のヘッダー
        ui.onAllNodesWithText("チャンネル").onFirst().assertExists()
        capture("tablet-conversation-light")
        ui.runOnIdle { dark = true }
        capture("tablet-conversation-dark")
        ui.onNodeWithText("雑談").performClick()
        ui.onNodeWithText("ホーム").performClick()
        ui.runOnIdle {
            assertEquals(listOf(AndroidUiAction.OpenChannel("random"), AndroidUiAction.Navigate(TopPage.Home)), actions.takeLast(2))
        }
    }

    @Test
    @Config(qualifiers = "ja-rJP-w1024dp-h768dp-xhdpi")
    fun tabletChannelListAsksToPickAChannel() {
        ui.setContent {
            AndroidScreens(channels.copy(selected = null), false, true, dispatch = {}, language = "ja", sidebar = channels.copy(selected = null)) { _, _ -> }
        }
        ui.onNodeWithText("チャンネルを選んでください").assertExists()
        capture("tablet-channels-light")
    }

    @Test
    @Config(qualifiers = "ja-rJP-w700dp-h900dp-xhdpi")
    fun mediumWidthUsesTheRailWithoutTheList() {
        ui.setContent {
            AndroidScreens(AndroidScreen.Conversation("dev", "開発", "", messages, InputState()), false, true,
                dispatch = {}, language = "ja", sidebar = channels) { _, _ -> }
        }
        ui.onNodeWithTag("navigation-rail").assertExists()
        ui.onNodeWithTag("channel-list-pane").assertDoesNotExist()
        capture("medium-conversation-light")
    }

    @Test
    fun phoneKeepsTheBottomNavigation() {
        ui.setContent {
            AndroidScreens(channels, false, true, dispatch = {}, language = "ja", sidebar = null) { _, _ -> }
        }
        ui.onNodeWithTag("navigation-rail").assertDoesNotExist()
        ui.onNodeWithTag("channel-list-pane").assertDoesNotExist()
        ui.onNodeWithText("ホーム").assertExists()
    }

    @Test
    fun decryptedFilesOfferSaveAndShare() {
        var chosen = ""
        var dark by mutableStateOf(false)
        val labels = mapOf("attachments.downloadShort" to "ダウンロード", "attachments.share" to "共有")
        ui.setContent {
            if (dark) androidx.compose.material3.MaterialTheme(androidx.compose.material3.darkColorScheme()) {
                DecryptedFileDialog("議事録-2026-11.zip", { labels.getValue(it) }, { chosen = "save" }, { chosen = "share" }, {})
            } else DecryptedFileDialog("議事録-2026-11.zip", { labels.getValue(it) }, { chosen = "save" }, { chosen = "share" }, {})
        }
        ui.onNodeWithText("議事録-2026-11.zip").assertExists()
        capture("decrypted-file-light", dialog = true)
        ui.runOnIdle { dark = true }
        capture("decrypted-file-dark", dialog = true)
        ui.onNodeWithText("ダウンロード").performClick()
        ui.runOnIdle { assertEquals("save", chosen) }
        ui.onNodeWithText("共有").performClick()
        ui.runOnIdle { assertEquals("share", chosen) }
    }

    @Test
    @Config(qualifiers = "ja-rJP-w800dp-h360dp-xhdpi")
    fun landscapePhoneCanScrollTheRailToEveryTab() {
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(AndroidScreen.Conversation("dev", "開発", "", messages, InputState()), false, true,
                dispatch = { actions += it }, language = "ja", sidebar = null) { _, _ -> }
        }
        ui.onNodeWithTag("navigation-rail").assertExists()
        ui.onNodeWithText("設定").performScrollTo().performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.Navigate(TopPage.Settings), actions.last()) }
        capture("landscape-rail-scrolled-light")
    }

    private fun capture(name: String, dialog: Boolean = false) {
        ui.runOnIdle {
            val view = ui.activity.findViewById<View>(android.R.id.content)
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                val canvas = Canvas(bitmap)
                view.draw(canvas)
                if (dialog) checkNotNull(ShadowDialog.getLatestDialog()?.window).decorView.draw(canvas)
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { bitmap.recycle() }
        }
    }
}
