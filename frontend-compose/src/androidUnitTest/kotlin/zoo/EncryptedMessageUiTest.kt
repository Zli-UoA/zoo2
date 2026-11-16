package zoo

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.serialization.json.*
import kotlin.test.*
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w480dp-h1100dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EncryptedMessageUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private class Crypto : MessageEncryption {
        override val changes = MutableStateFlow(0L)
        var calls = 0
        var result = DecryptedBody("confirmed body")
        override suspend fun send(channel: String, id: String, parent: String?, body: String) {}
        override suspend fun edit(message: Row, body: String) {}
        override suspend fun decrypt(message: Row): DecryptedBody { calls++; return result }
    }
    private class Fixture(val crypto: Crypto = Crypto()) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val platform = object : Platform {
            override val messageEncryption = crypto
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) { error("Plaintext must stay in the UI") }
            override fun uuid() = "test-id"
            override fun now() = "2026-11-15T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val http = HttpClient(MockEngine { error("Unexpected request") })
        val app = Session(Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform, http, scope, synchronize = false), scope, Environment(touch = true))
        fun close() { app.close(); scope.cancel(); http.close() }
    }
    private val channel = obj("id" to "c", "encrypted" to true)
    private val base = obj("id" to "m", "channel_id" to "c", "user_id" to "alice", "content" to "", "epoch" to 1, "revision" to 2, "ciphertext" to obj("ciphertext" to "x"))

    @Test
    fun metadataAndReDecryptionDoNotOverwriteTheEditDraft() {
        val f = Fixture()
        var row by mutableStateOf(base)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides f.app) { MaterialTheme {
                    Box(Modifier.width(440.dp)) { Message(f.app, channel, row, emptyList(), emptyList(), emptyList(), false, false, initialEditing = true) }
                } }
            }
            val editor = ui.onNodeWithContentDescription("messages.editLabel")
            editor.assertTextContains("confirmed body").performTextReplacement("unsaved draft")
            ui.runOnIdle { row = JsonObject(row + ("display_data" to obj("reply_count" to 3))) }
            editor.assertTextContains("unsaved draft")
            ui.runOnIdle { assertEquals(1, f.crypto.calls); f.crypto.result = DecryptedBody("updated verified body"); f.crypto.changes.value++ }
            editor.assertTextContains("unsaved draft")
            ui.runOnIdle { assertEquals(2, f.crypto.calls) }
        } finally { f.close() }
    }

    @Test
    fun legacyPlaintextCanStillBeReactedTo() {
        val f = Fixture()
        var toggles = 0
        val plain = JsonObject(base - "ciphertext" + ("content" to JsonPrimitive("before encryption")))
        val reactions = listOf(obj("message_id" to "m", "user_id" to "alice", "emoji" to "👍"))
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides f.app) { MaterialTheme {
                    Box(Modifier.width(440.dp)) { Message(f.app, channel, plain, emptyList(), reactions, emptyList(), false, false,
                        callbacks = MessageCallbacks(reaction = { toggles++ })) }
                } }
            }
            ui.onNodeWithContentDescription("reactions.chipMine").assertIsEnabled().performClick()
            ui.onNodeWithContentDescription("reactions.add").assertIsEnabled()
            ui.runOnIdle { assertEquals(1, toggles) }
        } finally { f.close() }
    }

    @Test
    fun staleRevisionKeepsThePreviouslyDisplayedBody() {
        val f = Fixture()
        f.crypto.result = DecryptedBody("confirmed body", "e2ee.sender.unverified")
        var row by mutableStateOf(base)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalTouchUi provides true, LocalApp provides f.app) { MaterialTheme {
                    Box(Modifier.width(440.dp)) { Message(f.app, channel, row, emptyList(), emptyList(), emptyList(), false, false) }
                } }
            }
            ui.onNodeWithText("confirmed body").assertIsDisplayed()
            ui.runOnIdle {
                f.crypto.result = DecryptedBody(notice = "e2ee.revision.stale", revision = MessageRevision.STALE)
                row = JsonObject(row + ("revision" to JsonPrimitive(1)))
            }
            ui.onNodeWithText("confirmed body").assertIsDisplayed()
            ui.onNodeWithText("e2ee.revision.stale").assertIsDisplayed()
            ui.onNodeWithText("e2ee.sender.unverified").assertIsDisplayed()
        } finally { f.close() }
    }
}
