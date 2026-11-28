package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.sp
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import zoo.ChannelLinkTarget

class ChannelLinkTextTest {
    private val app = "https://zoo.test/channels/c?message=m"
    private val labelled: (String, Boolean) -> EntryChannelLink? = { url, _ ->
        if (url.startsWith("https://zoo.test/")) EntryChannelLink("はるさん: #開発", messageIcon = true) {} else null
    }

    @Test
    fun onlyBareLinksToThisAppAreShortened() {
        val text = formatEntry("見て $app と [資料]($app) と https://example.test/a", channelLink = labelled)
        // アイコンの位置は、読みのない文字にする。
        assertEquals("見て \u2060 はるさん: #開発 と 資料 と https://example.test/a", text.text)
        val links = text.getLinkAnnotations(0, text.length).map { it.item }
        assertEquals(2, links.count { it is LinkAnnotation.Clickable })
        assertEquals("https://example.test/a", links.filterIsInstance<LinkAnnotation.Url>().single().url)
    }

    @Test
    fun labelsThatContainUrlsAreNotRewritten() {
        val asked = mutableListOf<Boolean>()
        val link: (String, Boolean) -> EntryChannelLink? = { url, bare -> asked += bare; labelled(url, bare) }
        val text = formatEntry("[$app]($app) と [見る $app]($app)", channelLink = link)
        // ラベルが URL でも書き換えず、ラベルの中の URL はリンクとして扱わない。
        assertEquals("$app と 見る $app", text.text)
        assertEquals(listOf(false, false), asked)
        assertEquals(2, text.getLinkAnnotations(0, text.length).size)
    }

    @Test
    fun unresolvedAndHighlightedLinksKeepTheUrl() {
        val unresolved = formatEntry(app) { _, _ -> null }
        assertEquals(app, unresolved.text)
        val waiting = formatEntry(app, channelLink = { _, _ -> EntryChannelLink(null, false) {} })
        assertEquals(app, waiting.text)
        assertTrue(waiting.getLinkAnnotations(0, waiting.length).single().item is LinkAnnotation.Clickable)
        val searching = formatEntry(app, highlight = EntryHighlight(listOf("zoo"), SpanStyle(background = Color.Yellow)),
            channelLink = labelled)
        assertEquals(app, searching.text)
    }

    @Test
    fun linksDrawnAsTextAreNotResolved() {
        val text = formatEntry(app, links = false, channelLink = { _, _ -> error("not asked") })
        assertEquals(app, text.text)
        assertTrue(text.getLinkAnnotations(0, text.length).isEmpty())
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h900dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ChannelLinkCardTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private fun url(id: String) = "https://zoo.test/channels/dev?message=$id"
    private val reply = ChannelLinkView.Message(
        "はるさん: #開発", "はる", "11/23 16:30", "#開発 内のスレッド",
        ChannelLinkBody("明日のリリースの手順をまとめました。確認をお願いします。"), parentId = "root",
    )
    private val views = mapOf<String, ChannelLinkView>(
        "reply" to reply,
        "locked" to reply.copy(channel = "#秘密", body = ChannelLinkBody("このメッセージは暗号化されています", UiIcon.Lock, true),
            parentId = null),
        "gone" to ChannelLinkView.Notice(UiIcon.VisibilityOff, "メッセージを表示できません"),
        "wait" to ChannelLinkView.Loading("リンク先を読み込み中"),
        "channel" to ChannelLinkView.Channel("#お知らせ", "非公開チャンネル", "全体への連絡"),
    )
    private val opened = mutableListOf<Pair<ChannelLinkTarget, String?>>()
    private val provider = ChannelLinkProvider(
        target = { link -> link.substringAfter("?message=", "").takeIf { it.isNotEmpty() }?.let { ChannelLinkTarget("dev", it) } },
        view = { target -> views.getValue(target.messageId.orEmpty()) },
        open = { target, parent -> opened += target to parent },
    )

    private fun entry(id: String, body: String, links: List<String>) =
        TimelineItem(id, "あおい", body, "10:00", channelLinks = links)

    private fun show(entries: List<TimelineItem>, dark: Boolean = false): (Boolean) -> Unit {
        var theme by mutableStateOf(dark)
        ui.setContent {
            CompositionLocalProvider(LocalChannelLinks provides provider) {
                AndroidScreens(AndroidScreen.Conversation("c", "general", "", entries, InputState()), dark = theme,
                    foreground = true, dispatch = {}, language = "ja") { _, _ -> }
            }
        }
        return { next -> ui.runOnIdle { theme = next } }
    }

    @Test
    fun cardsShowTheDestinationAndOpenIt() {
        show(listOf(entry("e1", "こうだと？ ${url("reply")}", listOf(url("reply")))))
        // 本文中の URL は短くなり、カードに投稿者・日時・チャンネル・本文が出る。
        ui.onNodeWithText("はるさん: #開発", substring = true).assertIsDisplayed()
        ui.onNodeWithText(url("reply"), substring = true).assertDoesNotExist()
        ui.onNodeWithText("11/23 16:30").assertIsDisplayed()
        ui.onNodeWithText("#開発 内のスレッド").assertIsDisplayed()
        ui.onNodeWithText("明日のリリースの手順", substring = true).performClick()
        ui.runOnIdle { assertEquals(listOf(ChannelLinkTarget("dev", "reply") to "root"), opened.toList()) }
    }

    @Test
    fun unavailableAndLoadingCardsDoNotNavigate() {
        show(listOf(entry("e1", "${url("gone")} ${url("wait")}", listOf(url("gone"), url("wait")))))
        // 見られない行き先と読み込み中は、URL のまま。
        ui.onNodeWithText(url("gone"), substring = true).assertIsDisplayed()
        ui.onNodeWithText("メッセージを表示できません").assertHasNoClickAction()
        ui.onNodeWithContentDescription("リンク先を読み込み中").assertHasNoClickAction()
    }

    @Test
    fun linksInAppBlocksKeepTheUrl() {
        val asked = mutableListOf<ChannelLinkTarget>()
        val recording = ChannelLinkProvider(
            target = provider.target, view = { asked += it; reply }, open = { _, _ -> },
        )
        ui.setContent {
            CompositionLocalProvider(LocalChannelLinks provides recording) {
                Column {
                    EntryBlocks(
                        listOf(EntryContentPanel(listOf(EntryContentText(url("reply"))))),
                        AndroidColors(dark = false), 14.sp, emptyMap(),
                    )
                }
            }
        }
        ui.onNodeWithText(url("reply")).assertIsDisplayed()
        ui.runOnIdle { assertEquals(emptyList<ChannelLinkTarget>(), asked.toList()) }
    }

    @Test
    fun captureBothThemes() {
        val setDark = show(listOf(
            entry("e1", "こうだと？ ${url("reply")}", listOf(url("reply"))),
            entry("e2", "暗号化と、見られないリンク", listOf(url("locked"), url("gone"))),
            entry("e3", "読み込み中と、チャンネル", listOf(url("wait"), url("channel"))),
        ))
        for (dark in listOf(false, true)) {
            setDark(dark)
            ui.waitForIdle()
            val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/android-ui").apply { mkdirs() }
            val file = File(directory, "channel-link-${if (dark) "dark" else "light"}.png")
            ui.recordControlBounds(file.nameWithoutExtension)
            ui.runOnIdle {
                // `captureToImage` は Robolectric で待ち切れないので、ほかの画面テストと同じく View を描く。
                val view = ui.activity.findViewById<View>(android.R.id.content)
                val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
                view.draw(Canvas(bitmap))
                file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                bitmap.recycle()
            }
            assertTrue(file.length() > 0)
        }
    }
}
