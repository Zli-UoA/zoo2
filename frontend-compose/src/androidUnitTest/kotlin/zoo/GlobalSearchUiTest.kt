package zoo

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.headersOf
import java.io.File
import kotlin.math.roundToInt
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.JsonArray
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w480dp-h1100dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@OptIn(ExperimentalTestApi::class)
class GlobalSearchUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    // Copied from frontend/src/lib/i18n/locales/ja.json so captures show real Japanese text.
    private val ja = mapOf(
        "globalSearch.title" to "検索",
        "globalSearch.placeholder" to "メッセージとファイルを検索",
        "globalSearch.kinds.messages" to "メッセージ",
        "globalSearch.kinds.files" to "ファイル",
        "globalSearch.prompt" to "検索する語を入力してください。空白で区切ると、すべての語を含むものを探します。",
        "globalSearch.empty" to "見つかりませんでした。",
        "globalSearch.emptyOlder" to "{{month}} までに見つかりませんでした。さらに古い期間を検索できます。",
        "globalSearch.searchedUntil" to "{{month}} まで検索しました",
        "globalSearch.searchOlder" to "さらに古い結果を検索",
        "globalSearch.loading" to "検索中…",
        "globalSearch.loadError" to "検索できませんでした。",
        "globalSearch.retry" to "再試行",
        "globalSearch.encryptedNote" to "暗号化されたチャンネルは検索の対象外です。",
        "globalSearch.inThread" to "スレッドの返信",
        "dms.untitled" to "名前のないDM",
    )

    private val alice = obj("id" to "alice", "user_name" to "alice", "display_name" to "Alice")
    private val bob = obj("id" to "bob", "user_name" to "bob", "display_name" to "Bob（長い表示名のテストユーザー）")
    private val users = listOf(alice, bob)

    private fun message(id: String, user: Row, channel: String, name: String, snippet: String, dm: Boolean = false, parent: String? = null) = obj(
        "id" to id, "channel_id" to channel, "channel_name" to name, "is_dm" to dm, "parent_id" to parent,
        "user_id" to user.text("id"), "author" to user, "emojis" to obj(), "snippet" to snippet,
        "created_at" to "2026-11-15T00:00:00.000",
    )

    private fun page(items: List<Row>, cursor: String?, until: String?) =
        obj("items" to JsonArray(items), "next_cursor" to cursor, "searched_until" to until).toString()

    private inner class Fixture(path: String) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val requests = mutableListOf<String>()
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "search-ui"
            override fun now() = "2026-11-21T00:00:00Z"
            override fun navigate(path: String) { app.update(app.env.copy(path = path)) }
            override fun replace(path: String) { app.update(app.env.copy(path = path)) }
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) =
                args.entries.fold(ja[key] ?: key) { text, (name, value) -> text.replace("{{$name}}", value) }
            override fun notificationTime(timestamp: String) = "11/15 09:00"
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        // Answer on the calling (main) thread: Robolectric's paused main looper otherwise
        // leaves later responses waiting for a resumption that waitUntil never delivers.
        val http = HttpClient(MockEngine.create {
            dispatcher = Dispatchers.Unconfined
            addHandler { request ->
            check(request.url.encodedPath == "/api/search") { "Unexpected API request ${request.url}" }
            requests += request.url.encodedQuery
            val q = request.url.parameters["q"]
            val body = when {
                q == "存在しない" -> page(emptyList(), "older", "2025-12")
                request.url.parameters["type"] == "files" -> page(listOf(obj(
                    "id" to "f1", "message_id" to "m1", "channel_id" to "general", "channel_name" to "general",
                    "is_dm" to false, "parent_id" to null, "user_id" to "alice", "author" to alice,
                    "file_name" to "2026年11月_定例会議_資料_最終版_修正済み.pdf", "content_type" to "application/pdf",
                    "file_size" to 2_400_000, "width" to null, "height" to null, "is_sensitive" to false,
                    "snippet" to "2026年11月_定例会議_資料_最終版_修正済み.pdf", "created_at" to "2026-11-15T00:00:00.000",
                )), null, "2026-11")
                else -> page(listOf(
                    message("m1", alice, "general", "general", "来週の会議の資料を共有します。議題は3つあります。"),
                    message("m2", bob, "dm-1", "dm:alice:bob", "会議の時間を15時に変更できますか？", dm = true, parent = "root"),
                    message("m3", alice, "random", "とても長いチャンネル名のテスト用チャンネル",
                        "…前回の会議で決まったことをまとめます。日本語の長い文章が折り返して3行を超える場合は、" +
                            "省略記号で切り詰められることを確認するための本文です。さらに文章を続けて、4行目に入るようにします。…"),
                ), "cursor-1", "2025-12")
            }
            respond(body, headers = headersOf(HttpHeaders.ContentType, "application/json"))
            }
        })
        val app: Session = Session(
            Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform, http, scope, synchronize = false),
            scope, Environment(touch = true, path = path),
        )
        val controller = GlobalSearchController(app, scope)
        fun close() { app.close(); scope.cancel() }
    }

    @Test
    fun searchScreenShowsResultsPagesOlderAndOpensTheMessage() {
        val fixture = Fixture("/search?q=会議&type=messages")
        var width by mutableStateOf(320.dp)
        var dark by mutableStateOf(false)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    ZooTheme(dark = dark) {
                        Box(Modifier.width(width).height(900.dp).testTag("global-search")
                            .background(MaterialTheme.colorScheme.background)) {
                            NativeWindow(fixture.app) {
                                GlobalSearchScreen(fixture.app, users, emptyList(), fixture.controller)
                            }
                        }
                    }
                }
            }
            ui.waitUntil(5_000) { fixture.controller.loaded }
            ui.waitForIdle()
            assertEquals(1, fixture.requests.size)
            assertTrue("q=%E4%BC%9A%E8%AD%B0" in fixture.requests.single(), fixture.requests.single())
            assertTrue("type=messages" in fixture.requests.single())
            ui.onAllNodesWithTag("global-search-result").assertCountEquals(3)
            ui.onNodeWithText("2025-12 まで検索しました").assertIsDisplayed()
            // The author line, and the DM destination label resolved through dmTitle.
            ui.onNode(hasTestTag("global-search-author") and hasText("Bob（長い表示名のテストユーザー）"), useUnmergedTree = true)
                .assertExists()
            ui.onAllNodesWithText("Bob（長い表示名のテストユーザー）", useUnmergedTree = true).assertCountEquals(2)
            ui.onNodeWithText("スレッドの返信", useUnmergedTree = true).assertExists()
            for ((nextWidth, nextDark) in listOf(320.dp to false, 440.dp to false, 320.dp to true, 440.dp to true)) {
                ui.runOnIdle { width = nextWidth; dark = nextDark }
                ui.waitForIdle()
                // The whole card and the older-results button keep 48dp touch targets.
                ui.onAllNodesWithTag("global-search-result")[0].assertHeightIsAtLeast(48.dp)
                ui.onNodeWithText("さらに古い結果を検索").assertHeightIsAtLeast(32.dp)
                screenshot("global-search", "global-search-messages-${nextWidth.value.toInt()}-${if (nextDark) "dark" else "light"}")
            }
            ui.runOnIdle { width = 320.dp; dark = false }

            ui.onNodeWithText("さらに古い結果を検索").performScrollTo().performClick()
            ui.waitUntil(5_000) { fixture.requests.size == 2 && !fixture.controller.loadingOlder }
            assertTrue("cursor=cursor-1" in fixture.requests[1], fixture.requests[1])

            ui.onAllNodesWithTag("global-search-result")[0].performClick()
            ui.runOnIdle { assertEquals("/channels/general?message=m1", fixture.app.env.path) }
        } finally { fixture.close() }
    }

    @Test
    fun filesTabAndEmptyOlderStateAreCaptured() {
        val fixture = Fixture("/search?q=資料&type=files")
        var width by mutableStateOf(320.dp)
        var dark by mutableStateOf(false)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    ZooTheme(dark = dark) {
                        Box(Modifier.width(width).height(640.dp).testTag("global-search")
                            .background(MaterialTheme.colorScheme.background)) {
                            NativeWindow(fixture.app) {
                                GlobalSearchScreen(fixture.app, users, emptyList(), fixture.controller)
                            }
                        }
                    }
                }
            }
            ui.waitUntil(5_000) { fixture.controller.loaded }
            ui.waitForIdle()
            ui.onNodeWithText("ファイル").assertIsSelected()
            ui.onAllNodesWithTag("global-search-result").assertCountEquals(1)
            ui.onNodeWithText("2.3 MB", useUnmergedTree = true).assertExists()
            screenshot("global-search", "global-search-files-320-light")
            ui.onAllNodesWithTag("global-search-result")[0].performClick()
            ui.runOnIdle { assertEquals("/channels/general?message=m1", fixture.app.env.path) }

            ui.runOnIdle { fixture.app.update(fixture.app.env.copy(path = "/search?q=存在しない&type=files")) }
            ui.runOnIdle { fixture.controller.search("存在しない", GlobalSearchType.Messages) }
            try {
                ui.waitUntil(5_000) { fixture.controller.loaded }
            } catch (e: Throwable) {
                val c = fixture.controller
                throw AssertionError("query=${c.query} type=${c.type} loading=${c.loading} error=${c.error} requests=${fixture.requests}", e)
            }
            ui.waitForIdle()
            ui.onNodeWithText("メッセージ").assertIsSelected()
            ui.onNodeWithText("2025-12 までに見つかりませんでした。さらに古い期間を検索できます。").assertIsDisplayed()
            ui.onNodeWithText("さらに古い結果を検索").assertIsDisplayed()
            ui.runOnIdle { assertEquals("/search?q=%E5%AD%98%E5%9C%A8%E3%81%97%E3%81%AA%E3%81%84&type=messages", fixture.app.env.path) }
            ui.runOnIdle { dark = true }
            screenshot("global-search", "global-search-empty-older-320-dark")

            ui.runOnIdle { fixture.controller.search("") }
            ui.onNodeWithText("検索する語を入力してください。空白で区切ると、すべての語を含むものを探します。").assertIsDisplayed()
        } finally { fixture.close() }
    }

    @Test
    fun routeChangesUpdateTheQueryAndKindWithoutRefetchingOwnReplaces() {
        val fixture = Fixture("/search?q=会議&type=messages")
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        Box(Modifier.width(320.dp).height(640.dp)) {
                            NativeWindow(fixture.app) {
                                GlobalSearchScreen(fixture.app, users, emptyList(), fixture.controller)
                            }
                        }
                    }
                }
            }
            ui.waitUntil(5_000) { fixture.controller.loaded }
            ui.runOnIdle { fixture.app.update(fixture.app.env.copy(path = "/search?q=資料&type=files")) }
            ui.waitUntil(5_000) { fixture.controller.type == GlobalSearchType.Files && fixture.controller.loaded }
            ui.waitForIdle()
            assertEquals("資料", fixture.controller.query)
            ui.onNode(hasSetTextAction() and hasText("資料"), useUnmergedTree = true).assertExists()
            ui.onNodeWithText("ファイル").assertIsSelected()
            // The screen's own replace (URL-encoded) does not start another request.
            assertEquals(2, fixture.requests.size, fixture.requests.toString())
            // A route without `q` is an empty query: the previous text and results go.
            ui.runOnIdle { fixture.app.update(fixture.app.env.copy(path = "/search?type=files")) }
            ui.waitUntil(5_000) { fixture.controller.query.isEmpty() }
            ui.waitForIdle()
            ui.onNode(hasSetTextAction() and hasText("資料"), useUnmergedTree = true).assertDoesNotExist()
            ui.onAllNodesWithTag("global-search-result").assertCountEquals(0)
            assertEquals(2, fixture.requests.size, fixture.requests.toString())
        } finally { fixture.close() }
    }

    @Test
    fun searchResultSharesTheNotificationAvatarGutterAndTopAlignment() {
        val fixture = Fixture("/search")
        var width by mutableStateOf(320.dp)
        val notificationMessage = obj("id" to "m1", "user_id" to "alice", "content" to "Notification body",
            "created_at" to "2026-11-15T00:00:00Z")
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        Column(Modifier.width(width)) {
                            NotificationCard(fixture.app, listOf(obj(
                                "id" to "notification", "actor_id" to "alice", "is_read" to true,
                                "notification_type" to "mention", "created_at" to "2026-11-15T00:00:00Z",
                                "display_data" to obj("actor" to alice, "message" to notificationMessage,
                                    "channel" to obj("id" to "general", "channel_name" to "general")),
                            )), users, visible = false, readEnabled = false, onLoading = {})
                            GlobalSearchResult(fixture.app, message("m1", alice, "general", "general", "Search body"),
                                GlobalSearchType.Messages, users, emptyList(), emptyList())
                        }
                    }
                }
            }
            for (nextWidth in listOf(320.dp, 440.dp)) {
                ui.runOnIdle { width = nextWidth }
                val names = ui.onAllNodesWithText("Alice", useUnmergedTree = true).fetchSemanticsNodes()
                assertEquals(2, names.size)
                val (notification, search) = names.sortedBy { it.boundsInRoot.top }
                assertEquals(notification.boundsInRoot.left, search.boundsInRoot.left, 1f)
                val card = ui.onNodeWithTag("global-search-result").fetchSemanticsNode().boundsInRoot
                val avatar = ui.onNodeWithTag("global-search-avatar", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
                assertEquals(with(ui.density) { 12.dp.toPx() }, avatar.left - card.left, 1f)
                assertEquals(with(ui.density) { 12.dp.toPx() }, avatar.top - card.top, 1f)
                assertEquals(with(ui.density) { 40.dp.toPx() }, avatar.width, 1f)
                assertEquals(avatar.top, search.boundsInRoot.top, 1f)
                val body = ui.onNodeWithTag("global-search-body", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
                assertEquals(avatar.right + with(ui.density) { 12.dp.toPx() }, body.left, 1f)
            }
        } finally { fixture.close() }
    }

    private fun screenshot(tag: String, name: String) {
        val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/ui").apply { mkdirs() }
        val bounds = ui.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot
        // Same capture as TouchUiTest: draw the real content View with the native backend.
        ui.runOnIdle {
            val content = ui.activity.findViewById<android.view.View>(android.R.id.content)
            val full = android.graphics.Bitmap.createBitmap(content.width, content.height, android.graphics.Bitmap.Config.ARGB_8888)
            try {
                content.draw(android.graphics.Canvas(full))
                val bitmap = android.graphics.Bitmap.createBitmap(
                    full, bounds.left.roundToInt(), bounds.top.roundToInt(),
                    bounds.width.roundToInt().coerceAtMost(full.width - bounds.left.roundToInt()),
                    bounds.height.roundToInt().coerceAtMost(full.height - bounds.top.roundToInt()),
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
}
