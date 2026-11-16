package zoo

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlinx.coroutines.*
import kotlin.test.*
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w480dp-h1100dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ReviewRegressionUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private class Fixture(initial: Environment = Environment(touch = true)) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            private var nextId = 0
            override fun uuid() = "review-${nextId++}"
            override fun now() = "2026-11-15T00:00:00Z"
            override fun navigate(path: String) { app.update(app.env.copy(path = path)) }
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val app: Session = Session(
            Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform,
                HttpClient(MockEngine { error("Unexpected API request") }), scope, synchronize = false),
            scope, initial,
        )
        fun close() { app.close(); scope.cancel() }
    }

    @Test
    fun aDownConsumedByTheScrollingParentDoesNotNavigateOnRelease() {
        val fixture = Fixture(Environment(path = "/home", touch = true))
        var stoppingScroll = true
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme {
                        Box(Modifier.size(320.dp).pointerInput(Unit) {
                            awaitEachGesture {
                                val down = awaitFirstDown(false, PointerEventPass.Main)
                                // LazyColumn consumes this down when a finger stops its fling.
                                if (stoppingScroll) down.consume()
                                waitForUpOrCancellation()
                            }
                        }) {
                            NativeLink("/home/posts/target", Modifier.size(160.dp).testTag("target")) {
                                Text("target")
                            }
                        }
                    }
                }
            }
            ui.onNodeWithTag("target").performTouchInput { click() }
            ui.runOnIdle {
                assertEquals("/home", fixture.app.env.path)
                stoppingScroll = false
            }
            ui.onNodeWithTag("target").performTouchInput {
                down(center)
                moveTo(center.copy(y = center.y + 60f))
                moveTo(center)
                up()
            }
            ui.runOnIdle { assertEquals("/home", fixture.app.env.path) }
            ui.onNodeWithTag("target").performTouchInput { click() }
            ui.runOnIdle { assertEquals("/home/posts/target", fixture.app.env.path) }
        } finally { fixture.close() }
    }

    @Test
    fun tallHomeMediaDoesNotOverlapTheNextRowAtPhoneWidth() {
        val fixture = Fixture()
        val tall = obj("url" to "/api/home/images/abcd-1234", "width" to 100,
            "height" to 10000, "is_sensitive" to true)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme {
                        Column(Modifier.width(240.dp).testTag("media-container")) {
                            HomeMedia(listOf(tall))
                            Text("next row", Modifier.testTag("next"))
                        }
                    }
                }
            }
            val media = ui.onNodeWithContentDescription("attachments.showSensitive").fetchSemanticsNode().boundsInRoot
            val next = ui.onNodeWithTag("next").fetchSemanticsNode().boundsInRoot
            val container = ui.onNodeWithTag("media-container").fetchSemanticsNode().boundsInRoot
            assertTrue(media.left >= container.left && media.right <= container.right)
            assertTrue(media.bottom <= next.top, "Tall images must remain inside their measured row")
            ui.onNodeWithTag("next").assertIsDisplayed()
        } finally { fixture.close() }
    }

    @Test
    fun homeQuotesRenderFromTheDtoWithoutAnotherPostRequest() {
        val fixture = Fixture()
        val quoted = obj("id" to "quoted", "remote" to false, "user_name" to "bob",
            "content" to "quoted body", "created_at" to "2026-11-15T00:00:00Z")
        val post = kotlinx.serialization.json.JsonObject(obj("content" to "comment\n\n<https://zoo.test/note>") +
            ("quote" to kotlinx.serialization.json.JsonObject(obj("uri" to "https://zoo.test/note") + ("post" to quoted))))
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme { Box(Modifier.width(300.dp)) { HomePostContent(fixture.app, post, previewLinks = false) } }
                }
            }
            ui.onNodeWithText("quoted body").assertIsDisplayed()
            ui.onNodeWithText("comment").assertIsDisplayed()
            ui.onNodeWithText("<https://zoo.test/note>").assertDoesNotExist()
        } finally { fixture.close() }
    }

    @Test
    fun failedRefreshKeepsCachedChannelsAndDirectMessagesVisible() {
        val fixture = Fixture()
        val general = obj("id" to "general", "channel_name" to "General")
        val dm = obj("id" to "dm", "channel_name" to "Support", "is_dm" to true)
        var rows by mutableStateOf(listOf(general, dm))
        val members = listOf("general", "dm").map { obj("channel_id" to it, "user_id" to "alice") } +
            obj("channel_id" to "dm", "user_id" to "bob")
        val users = listOf(obj("id" to "bob", "user_name" to "bob", "display_name" to "Support"))
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        Sidebar(fixture.app, rows, users, members, false,
                            Modifier.width(400.dp).height(1000.dp), error = true, narrow = true)
                    }
                }
            }
            ui.onNodeWithText("channels.loadError").assertIsDisplayed()
            ui.onNodeWithText("General").assertIsDisplayed()
            ui.onNodeWithText("Support").assertIsDisplayed()
            ui.onNodeWithText("offline.retry").performClick()
            ui.onNodeWithText("General").assertIsDisplayed()
            ui.runOnIdle { rows = emptyList() }
            ui.onNodeWithText("channels.loadError").assertIsDisplayed()
            ui.onNodeWithText("channels.empty").assertDoesNotExist()
        } finally { fixture.close() }
    }

    @Test
    fun reselectingTheCurrentDmReturnsFromTheSidebarToChat() {
        val fixture = Fixture(Environment(path = "/channels/dm", touch = true))
        fixture.app.pane = "sidebar"
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        DmItem(fixture.app, obj("id" to "dm", "channel_name" to "Support"),
                            "Support", active = true, unread = false, narrow = true)
                    }
                }
            }
            ui.onNodeWithContentDescription("Support").performClick()
            ui.runOnIdle {
                assertEquals("/channels/dm", fixture.app.env.path)
                assertEquals("chat", fixture.app.pane)
            }
        } finally { fixture.close() }
    }

    @Test
    fun homeComposerContinuesSendingAfterItIsRemovedFromComposition() {
        val fixture = Fixture(Environment(path = "/home", touch = true))
        val state = HomeComposerState().apply { content = "画面を離れても送信する本文"; opened = true }
        val response = CompletableDeferred<Unit>()
        var shown by mutableStateOf(true)
        var posts = 0
        try {
            ui.setContent {
                CompositionLocalProvider(
                    LocalTouchUi provides true, LocalApp provides fixture.app, LocalViewportWidth provides 360f,
                ) {
                    MaterialTheme {
                        NativeWindow(fixture.app, Modifier.width(360.dp).height(900.dp)) {
                            if (shown) HomeComposer(fixture.app, false, state) {
                                posts++
                                response.await()
                            }
                        }
                    }
                }
            }
            ui.onNodeWithContentDescription("home.post").performClick()
            ui.runOnIdle { assertEquals(1, posts); assertTrue(state.sending); shown = false }
            ui.waitForIdle()
            ui.runOnIdle {
                assertTrue(state.sending, "Removing the editor must not cancel its submission")
                response.complete(Unit)
            }
            ui.waitUntil(5000) { !state.sending }
            ui.runOnIdle {
                assertEquals("", state.content)
                assertFalse(state.failed)
                assertEquals(1, posts)
                shown = true
            }
            ui.waitForIdle()
            ui.runOnIdle { assertEquals("", state.content); assertEquals(1, posts) }
        } finally { fixture.close() }
    }
}
