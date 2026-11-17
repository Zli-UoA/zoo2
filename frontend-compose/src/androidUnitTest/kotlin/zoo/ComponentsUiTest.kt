package zoo

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.*
import io.ktor.http.content.TextContent
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.Json
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** メッセージ部品の確認（confirm）。確認した内容と、アプリへ送る操作・値が食い違わないこと。 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w480dp-h1100dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ComponentsUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private class Fixture {
        /** invoke に送った本文 */
        val invoked = mutableListOf<String>()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "components-test"
            override fun now() = "2026-11-17T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val app = Session(
            Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform,
                HttpClient(MockEngine { request ->
                    if (request.url.encodedPath.endsWith("/invoke"))
                        invoked += (request.body as? TextContent)?.text.orEmpty()
                    respond("{}", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                }), scope, synchronize = false),
            scope, Environment(touch = true),
        )
        fun close() { app.close(); scope.cancel() }
    }

    private fun doc(json: String) = parseComponents(Json.parseToJsonElement(json))!!

    @Test
    fun declinedConfirmationKeepsTheOldChoiceAndDoesNotSubmitIt() {
        val fixture = Fixture()
        // Label があるメッセージなので送信ボタンが付く。Label の外の radio は選んだときに送る（確認あり）
        val message = doc(
            """{"components":[
              {"type":1,"components":[{"type":21,"custom_id":"plan","options":[
                {"label":"Free","value":"free","default":true},{"label":"Paid","value":"paid"}],
                "confirm":{"title":"Change?","text":"Sure?","confirm":"Yes","deny":"No"}}]},
              {"type":18,"label":"Name","component":{"type":4,"custom_id":"name","required":false}}]}"""
        )
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme {
                        Box {
                            ComponentTree(fixture.app, message, "app", message = "m1", channel = "c1")
                            // 確認は Panel（重ねて出す画面）なので、アプリの外枠と同じく重ね先を置く
                            OverlayHost(fixture.app)
                        }
                    }
                }
            }
            ui.onNodeWithText("Paid").performClick()
            ui.onNodeWithText("No").performClick()
            ui.waitForIdle()
            assertTrue(fixture.invoked.isEmpty(), "declined choice must not be sent")
            // 送信しても、取り消した値は入らない（既定の free のまま）
            ui.onNodeWithText("apps.submit").performClick()
            ui.waitUntil(5_000) { fixture.invoked.isNotEmpty() }
            val body = fixture.invoked.single()
            assertTrue(body.contains("\"kind\":\"submit\""), body)
            assertTrue(body.contains("\"plan\":\"free\""), body)
            assertFalse(body.contains("paid"), body)
        } finally {
            fixture.close()
        }
    }

    @Test
    fun confirmationClosesWhenTheConfirmedComponentMovesAway() {
        val fixture = Fixture()
        fun button(id: String, title: String) =
            """{"type":2,"style":1,"label":"$id","custom_id":"$id",
               "confirm":{"title":"$title","text":"$title","confirm":"Yes","deny":"No"}}"""
        var current by mutableStateOf(
            doc("""{"components":[{"type":1,"components":[${button("action-a", "A?")},${button("action-b", "B?")}]}]}""")
        )
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme {
                        Box {
                            ComponentTree(fixture.app, current, "app", message = "m1", channel = "c1")
                            OverlayHost(fixture.app)
                        }
                    }
                }
            }
            ui.onNodeWithText("action-a").performClick()
            ui.onAllNodesWithText("A?")[0].assertExists()
            // 確認の表示中に、アプリが部品の順序を入れ替える
            current = doc("""{"components":[{"type":1,"components":[${button("action-b", "B?")},${button("action-a", "A?")}]}]}""")
            ui.waitForIdle()
            // 別の操作（B）の文言で確認を出したまま、A を送ることはない
            ui.onAllNodesWithText("B?").assertCountEquals(0)
            ui.onAllNodesWithText("Yes").assertCountEquals(0)
            assertTrue(fixture.invoked.isEmpty())
        } finally {
            fixture.close()
        }
    }
    @Test
    fun confirmationClosesWhenTheDocumentUpdatesWithTheSameComponent() {
        val fixture = Fixture()
        fun message(value: String) =
            doc("""{"components":[{"type":1,"components":[{"type":2,"style":1,"label":"Go","custom_id":"go","value":"$value",
                "confirm":{"title":"Go?","text":"Go?","confirm":"Yes","deny":"No"}}]}]}""")
        var current by mutableStateOf(message("v1"))
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme {
                        Box {
                            ComponentTree(fixture.app, current, "app", message = "m1", channel = "c1")
                            OverlayHost(fixture.app)
                        }
                    }
                }
            }
            ui.onNodeWithText("Go").performClick()
            ui.onNodeWithText("Yes").assertExists()
            // 同じ custom_id・同じ確認の文言のまま、アプリが文書を更新する。開いていた確認で古い値を送らない
            current = message("v2")
            ui.waitForIdle()
            ui.onAllNodesWithText("Yes").assertCountEquals(0)
            assertTrue(fixture.invoked.isEmpty())
        } finally {
            fixture.close()
        }
    }

    @Test
    fun tablePageStaysInRangeWhenThePageSizeChanges() {
        val fixture = Fixture()
        fun table(size: Int) =
            doc("""{"components":[{"type":1009,"page_size":$size,"rows":[[{"text":"row-a"}],[{"text":"row-b"}]]}]}""")
        var current by mutableStateOf(table(1))
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides fixture.app) {
                    MaterialTheme { ComponentTree(fixture.app, current, null) }
                }
            }
            ui.onNodeWithText("›").performClick()
            ui.onNodeWithText("row-b").assertExists()
            current = table(0)
            ui.waitForIdle()
            ui.onNodeWithText("row-a").assertExists()
            ui.onNodeWithText("row-b").assertExists()
        } finally {
            fixture.close()
        }
    }
}
