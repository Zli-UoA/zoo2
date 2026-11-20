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
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.semantics.SemanticsProperties
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
            override fun relativeTimestamp(timestamp: String) = "5分前"
            override fun notificationTime(timestamp: String) = "11/15 09:00"
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
    fun kitchenFallbackConvertsTheDpSlotToSpOnceAtLargeFontScale() {
        val fixture = Fixture()
        try {
            ui.setContent {
                CompositionLocalProvider(
                    LocalApp provides fixture.app,
                    androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(2f, 1.8f),
                ) {
                    MaterialTheme {
                        ReactionEmoji("kitchen:1f431_1f600", size = 24.dp, loadCatalog = false)
                    }
                }
            }
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            ui.onNodeWithText("🐱", useUnmergedTree = true).performSemanticsAction(
                androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult
            ) { it(layouts) }
            assertEquals(1, layouts.size)
            val layout = layouts.single()
            // Android 14+ uses nonlinear font scaling; verify the rendered dp
            // size instead of assuming that sp = dp / fontScale.
            val glyphDp = with(layout.layoutInput.density) { layout.layoutInput.style.fontSize.toDp() }
            assertEquals(24f, glyphDp.value, 0.1f)
            assertEquals(layout.layoutInput.style.fontSize, layout.layoutInput.style.lineHeight)
        } finally { fixture.close() }
    }

    @Test
    fun emojiPickerRecentsRenderKitchenImagesWithPhoneSizedTargets() {
        val fixture = Fixture()
        var width by mutableStateOf(256.dp)
        var dark by mutableStateOf(false)
        var chosen = ""
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    CompositionLocalProvider(androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(androidx.compose.ui.platform.LocalDensity.current.density, fontScale = 1.8f)) {
                        MaterialTheme(colorScheme = zooColorScheme(dark)) {
                            Box(Modifier.width(width)) {
                                EmojiPicker(fixture.app, {}, initialRecents = listOf("kitchen:1f431_1f600", "😀"), embedded = true) { chosen = it }
                            }
                        }
                    }
                }
            }
            ui.onNodeWithContentDescription("🐱 + 😀")
                .assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).performClick()
            ui.runOnIdle { assertEquals("kitchen:1f431_1f600", chosen); width = 416.dp; dark = true }
            ui.onNodeWithContentDescription("😀").assertIsDisplayed().performClick()
            ui.runOnIdle { assertEquals("😀", chosen) }
        } finally { fixture.close() }
    }

    @Test
    fun mediaPreviewClosesAndReopensWithWebViewDetachedBeforeDestruction() {
        val fixture = Fixture()
        var open by mutableStateOf(true)
        val destroyed = mutableListOf<android.webkit.WebView>()
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        if (open) MediaPreviewPanel("preview", { open = false }) {
                            androidx.compose.ui.viewinterop.AndroidView(
                                factory = { context ->
                                    object : android.webkit.WebView(context) {
                                        override fun destroy() {
                                            assertNull(parent, "WebView must be detached before destroy")
                                            destroyed.add(this)
                                            super.destroy()
                                        }
                                    }
                                },
                                modifier = Modifier.fillMaxWidth().weight(1f),
                                onRelease = ::releasePreviewWebView,
                            )
                        }
                    }
                }
            }
            repeat(3) { index ->
                ui.onNodeWithContentDescription("common.close").assertIsDisplayed().performClick()
                ui.waitForIdle()
                assertEquals(index + 1, destroyed.size)
                assertTrue(fixture.app.overlays.isEmpty(), "Native preview must not blur the main scene")
                if (index < 2) ui.runOnIdle { open = true }
            }
        } finally { fixture.close() }
    }

    @Test
    fun messageMenuUsesInsetHostCoordinatesAndFlipsAboveBottomTrigger() {
        val fixture = Fixture()
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        Column {
                            Spacer(Modifier.height(64.dp))
                            NativeWindow(fixture.app, Modifier.size(320.dp, 500.dp)) {
                                Box(Modifier.fillMaxSize().padding(8.dp), contentAlignment = androidx.compose.ui.Alignment.BottomEnd) {
                                    Menu("message actions", List(6) { index ->
                                        MenuAction("copy", "action $index") {}
                                    })
                                }
                            }
                        }
                    }
                }
            }
            val trigger = ui.onNodeWithContentDescription("message actions")
            val triggerBounds = trigger.fetchSemanticsNode().boundsInRoot
            trigger.performClick()
            val menu = ui.onNode(SemanticsMatcher.expectValue(
                androidx.compose.ui.semantics.SemanticsProperties.PaneTitle, "message actions",
            ))
            menu.assertIsDisplayed()
            val bounds = menu.fetchSemanticsNode().boundsInRoot
            assertTrue(bounds.height <= triggerBounds.height * 248f / 48f + 1f,
                "Menu height should stay within 240dp plus its padding")
            assertTrue(bounds.bottom <= triggerBounds.top, "Menu should flip above its trigger")
            assertTrue(kotlin.math.abs(bounds.right - triggerBounds.right) <= 1f,
                "Menu should align with the trigger inside the inset host")
            ui.onNodeWithText("action 5").performScrollTo().assertIsDisplayed()
            ui.onNodeWithText("action 5").performClick()
            menu.assertDoesNotExist()
        } finally { fixture.close() }
    }

    @Test
    fun aDownConsumedByTheScrollingParentDoesNotNavigateOnRelease() {
        val fixture = Fixture(Environment(path = "/home", touch = true))
        var stoppingScroll = true
        var prepared = 0
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
                            NativeLink("/home/posts/target", Modifier.size(160.dp).testTag("target"), beforeNavigate = { prepared++ }) {
                                Text("target")
                            }
                        }
                    }
                }
            }
            ui.onNodeWithTag("target").performTouchInput { click() }
            ui.runOnIdle {
                assertEquals("/home", fixture.app.env.path)
                assertEquals(0, prepared)
                stoppingScroll = false
            }
            ui.onNodeWithTag("target").performTouchInput {
                down(center)
                moveTo(center.copy(y = center.y + 60f))
                moveTo(center)
                up()
            }
            ui.runOnIdle { assertEquals("/home", fixture.app.env.path); assertEquals(0, prepared) }
            ui.onNodeWithTag("target").performTouchInput { click() }
            ui.runOnIdle { assertEquals("/home/posts/target", fixture.app.env.path); assertEquals(1, prepared) }
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
    fun homeAndChannelAuthorsShareTheAvatarGutterAndTopAlignment() {
        val fixture = Fixture()
        val user = obj("id" to "alice", "display_name" to "Alice", "user_name" to "alice")
        val post = obj("id" to "post", "user_id" to "alice", "display_name" to "Alice",
            "user_name" to "alice", "content" to "Home body", "created_at" to "2026-11-15T00:00:00Z")
        val message = obj("id" to "message", "user_id" to "alice", "content" to "Channel body",
            "created_at" to "2026-11-15T00:00:00Z")
        var width by mutableStateOf(320.dp)
        var notificationReference by mutableStateOf(true)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        Column(Modifier.width(width)) {
                            if (notificationReference) {
                                NotificationCard(fixture.app, listOf(obj(
                                    "id" to "notification", "actor_id" to "alice", "is_read" to true,
                                    "notification_type" to "mention", "created_at" to "2026-11-15T00:00:00Z",
                                    "display_data" to obj("actor" to user, "message" to message,
                                        "channel" to obj("id" to "general", "channel_name" to "general")),
                                )), listOf(user), visible = false, readEnabled = false, onLoading = {})
                            } else {
                                HomePostCard(fixture.app, post, false, false, {}, {}, {}, {}, {}, {})
                                MessageList(fixture.app, obj("id" to "general"),
                                    listOf(message, obj("id" to "message-2", "user_id" to "alice",
                                        "content" to "Grouped body", "created_at" to "2026-11-15T00:00:01Z")),
                                    emptyList(), listOf(user), parent = null, wide = false,
                                    modifier = Modifier.height(350.dp), readState = false)
                            }
                        }
                    }
                }
            }
            for (nextWidth in listOf(320.dp, 440.dp)) {
                ui.runOnIdle { width = nextWidth; notificationReference = true }
                val keyline = ui.onNodeWithText("Alice").fetchSemanticsNode().boundsInRoot.left
                assertTrue(keyline > 0f)
                ui.runOnIdle { notificationReference = false }
                val names = ui.onAllNodesWithText("Alice").fetchSemanticsNodes()
                assertEquals(2, names.size)
                names.forEach { assertEquals(keyline, it.boundsInRoot.left, 1f) }
                val avatars = ui.onAllNodesWithTag("post-avatar", useUnmergedTree = true).fetchSemanticsNodes()
                assertEquals(2, avatars.size)
                assertEquals(with(ui.density) { 12.dp.toPx() }, avatars[0].boundsInRoot.left, 1f)
                assertEquals(avatars[0].boundsInRoot.left, avatars[1].boundsInRoot.left, 1f)
                avatars.zip(names).forEach { (avatar, name) ->
                    assertEquals(with(ui.density) { 40.dp.toPx() }, avatar.boundsInRoot.width, 1f)
                    assertEquals(avatar.boundsInRoot.top, name.boundsInRoot.top, 1f)
                }
                val grouped = ui.onNodeWithText("Grouped body").fetchSemanticsNode().boundsInRoot
                assertEquals(keyline, grouped.left, 1f)
                ui.onAllNodesWithTag("post-avatar", useUnmergedTree = true)[0].onParent()
                    .assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp)
            }
        } finally { fixture.close() }
    }

    @Test
    fun channelComposerSwipesToFullScreenAndPreservesSelectedJapaneseText() {
        val fixture = Fixture(Environment(touch = true, online = false))
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme {
                        NativeWindow(fixture.app, Modifier.width(320.dp).height(900.dp)) {
                            Column(Modifier.fillMaxSize()) {
                                Spacer(Modifier.weight(1f))
                                Composer(fixture.app, "general", null, emptyList(), enabled = true,
                                    initialValue = "日本語の下書き", showTyping = false, onTyping = {})
                            }
                        }
                    }
                }
            }
            val input = ui.onNodeWithContentDescription("messages.compose")
            val compact = input.fetchSemanticsNode().boundsInRoot.height
            input.performTextInputSelection(TextRange(0, 3))
            input.assert(SemanticsMatcher.expectValue(SemanticsProperties.TextSelectionRange, TextRange(0, 3)))
            ui.onNodeWithTag("composer-drag-handle").performTouchInput { swipeUp(endY = -150f) }
            ui.onNodeWithContentDescription("composer.collapse").assertIsDisplayed()
            assertTrue(input.fetchSemanticsNode().boundsInRoot.height > compact * 2)
            input.assert(SemanticsMatcher.expectValue(SemanticsProperties.TextSelectionRange, TextRange(0, 3)))
            ui.onNodeWithContentDescription("composer.formatting").performClick()
            ui.onNodeWithContentDescription("composer.bold").performClick()
            input.assertTextEquals("<b>日本語</b>の下書き")
            ui.onNodeWithContentDescription("composer.collapse").performClick()
            input.assert(SemanticsMatcher.expectValue(SemanticsProperties.TextSelectionRange, TextRange(3, 6)))
            input.performTextInput("置換")
            input.assertTextEquals("<b>置換</b>の下書き")
            ui.onNodeWithContentDescription("messages.send").assertIsNotEnabled()
            ui.onNodeWithContentDescription("attachments.add").assertIsNotEnabled()
            // Back returns to the compact draft rather than navigating out of the channel.
            ui.onNodeWithTag("composer-drag-handle").performTouchInput { swipeUp(endY = -150f) }
            ui.runOnIdle { fixture.app.update(fixture.app.env.copy(visualViewportHeight = 220f)) }
            ui.onNodeWithContentDescription("messages.send").performScrollTo().assertIsDisplayed()
            ui.runOnIdle { assertTrue(fixture.app.escape()) }
            input.assertTextEquals("<b>置換</b>の下書き")
            ui.onNodeWithContentDescription("composer.collapse").assertDoesNotExist()
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
    @Test
    fun reactionGlyphsAndMissingCustomEmojiNamesAreNotReplacedByAnEllipsis() {
        val fixture = Fixture()
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme {
                        Column {
                            ReactionEmoji("👩🏽‍💻", size = 16.dp)
                            ActivityPubEmoji(obj("name" to ":missing_custom_emoji:", "image_id" to "invalid"), size = 18.dp)
                        }
                    }
                }
            }
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            ui.onNodeWithText("👩🏽‍💻").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) { it(layouts) }
            assertFalse(layouts.single().isLineEllipsized(0))
            ui.onNodeWithText(":missing_custom_emoji:").assertWidthIsAtLeast(40.dp)
        } finally { fixture.close() }
    }


    @Test
    fun homeBodyTimestampAndParentLinksHandOffTheirOwnPost() {
        val fixture = Fixture(Environment(path = "/home", touch = true))
        val parent = obj("id" to "parent", "display_name" to "Parent", "content" to "Parent body")
        val post = obj("id" to "post", "user_id" to "alice", "display_name" to "Alice",
            "content" to "Known body", "created_at" to "2026-11-15T00:00:00Z",
            "parent_id" to "parent", "parent" to parent)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides fixture.app) {
                    MaterialTheme { HomePostCard(fixture.app, post, false, false, {}, {}, {}, {}, {}, {}) }
                }
            }
            for (label in listOf("Known body", "5分前", "home.replyTo")) {
                ui.onNodeWithText(label).performTouchInput { click() }
                ui.runOnIdle {
                    val id = if (label == "home.replyTo") "parent" else "post"
                    assertEquals("/home/posts/$id", fixture.app.env.path)
                    assertEquals(if (id == "parent") "Parent body" else "Known body",
                        fixture.app.homeTimelines.takePost(id)?.text("content"))
                    assertNull(fixture.app.homeTimelines.takePost(if (id == "parent") "post" else "parent"))
                    fixture.app.update(fixture.app.env.copy(path = "/home"))
                }
            }
        } finally { fixture.close() }
    }
}
