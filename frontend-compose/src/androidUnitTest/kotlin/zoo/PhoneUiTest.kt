package zoo

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.TextLayoutResult
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
import zoo.phone.*

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w320dp-h1100dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PhoneUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private class Fixture(prepare: suspend () -> Unit = {}) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "phone-test"
            override fun now() = "2030-01-01T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val http = HttpClient(MockEngine { error("Unexpected network request") })
        val app = Session(Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform, http, scope, synchronize = false), scope, Environment(touch = true))
        val controller = PhoneController(scope, request = { _, _, _ -> obj("enabled" to true, "lines" to kotlinx.serialization.json.JsonArray(emptyList())) },
            stream = { _, _, _ -> awaitCancellation() }, media = { _, _ -> error("Unexpected media capture") },
            telecom = object : PhoneTelecom {
                override suspend fun outgoing(call: PhoneCall) {}
                override suspend fun startAudio(id: String) = Unit
                override fun incoming(call: PhoneCall) {}
                override fun ended(id: String) {}
                override fun speaker(id: String, enabled: Boolean) {}
            }, prepare = prepare, waiting = {}, uuid = { "call" })
        fun close() { controller.close(); app.close(); http.close(); scope.cancel() }
    }
    @Test fun settingsExposeOptInWaitingAndPersonalRegistrationAtPhoneWidth() {
        val f = Fixture()
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides f.app, LocalTouchUi provides true) {
                    MaterialTheme { Column(Modifier.width(320.dp).verticalScroll(rememberScrollState())) { PhoneSettings(f.app, f.controller, emptyList()) } }
                }
            }
            ui.onNodeWithText("androidPhone.receiveDm").performClick()
            ui.runOnIdle { assertTrue(f.controller.dmWaiting) }
            ui.onNodeWithText("sip.addLine").performClick()
            ui.onNodeWithText("sip.name").performScrollTo().assertIsDisplayed()
            ui.onNodeWithText("sip.shared").assertDoesNotExist()
            ui.onNodeWithText("sip.save").performScrollTo().assertIsNotEnabled()
        } finally { f.close() }
    }
    @Test fun failureRemainsVisibleAfterTheCallHasEndedAndCanBeDismissed() {
        val f = Fixture()
        try {
            ui.runOnIdle {
                f.controller.callDm("dm", "Bob")
                f.controller.nativeFailed("call")
            }
            ui.setContent {
                CompositionLocalProvider(LocalApp provides f.app, LocalTouchUi provides true) {
                    MaterialTheme { PhoneCallBanner(f.app, f.controller) }
                }
            }
            ui.onNodeWithText("androidPhone.callError").assertIsDisplayed()
            ui.onNodeWithText("common.close").performClick()
            ui.onNodeWithText("androidPhone.callError").assertDoesNotExist()
        } finally { f.close() }
    }
    @Test fun waitingPermissionFailureUsesTheThemeErrorColorBesideTheWaitingControl() {
        val f = Fixture(prepare = { error("Microphone permission denied") })
        val colors = zooColorScheme(dark = true)
        try {
            ui.setContent {
                CompositionLocalProvider(LocalApp provides f.app, LocalTouchUi provides true) {
                    MaterialTheme(colorScheme = colors) {
                        Column(Modifier.width(320.dp).verticalScroll(rememberScrollState())) {
                            PhoneSettings(f.app, f.controller, emptyList())
                            PhoneCallBanner(f.app, f.controller)
                        }
                    }
                }
            }
            val waiting = ui.onNodeWithText("androidPhone.receiveDm").performClick()
            ui.onAllNodesWithText("androidPhone.callError").assertCountEquals(1)
            val message = ui.onNodeWithText("androidPhone.callError").assertIsDisplayed()
            assertTrue(message.fetchSemanticsNode().boundsInRoot.top >= waiting.fetchSemanticsNode().boundsInRoot.bottom)
            val layouts = mutableListOf<TextLayoutResult>()
            message.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
            assertEquals(colors.error, layouts.single().layoutInput.style.color)
            ui.onNodeWithText("common.close").assertDoesNotExist()
        } finally { f.close() }
    }
    @Test fun minimizedCallCanBeExpandedWithoutInterruptingTheCall() {
        val f = Fixture()
        try {
            ui.runOnIdle { f.controller.callDm("dm", "Bob") }
            ui.setContent {
                CompositionLocalProvider(LocalApp provides f.app, LocalTouchUi provides true) {
                    MaterialTheme { Box(Modifier.width(320.dp)) { PhoneCallBanner(f.app, f.controller) } }
                }
            }
            ui.onNodeWithText("androidPhone.minimizeCall").performClick()
            ui.onNodeWithContentDescription("androidPhone.expandCall").assertIsDisplayed()
            ui.runOnIdle { assertEquals("call", f.controller.call?.id) }
            ui.onNodeWithContentDescription("androidPhone.expandCall").performClick()
            ui.onNodeWithText("sip.hangup").performClick()
            ui.runOnIdle { assertNull(f.controller.call) }
            ui.onNodeWithContentDescription("androidPhone.expandCall").assertDoesNotExist()
        } finally { f.close() }
    }

    @Test fun incomingBannerOffersAnswerAndEndWithoutOpeningTheMicrophone() {
        val f = Fixture()
        try {
            ui.runOnIdle { f.controller.receiveDmCalls(true) }
            ui.waitUntil { f.controller.dmWaiting }
            ui.runOnIdle { f.controller.invites(listOf(obj("id" to "invite", "channel_id" to "dm", "invitee_id" to "alice", "inviter_name" to "Bob", "status" to "pending", "expires_at" to "2030-01-01T00:00:30Z")), "alice", 1893456000000) }
            ui.setContent {
                CompositionLocalProvider(LocalApp provides f.app, LocalTouchUi provides true) {
                    MaterialTheme { Box(Modifier.width(320.dp)) { PhoneCallBanner(f.app, f.controller) } }
                }
            }
            ui.onNodeWithText("Bob").assertIsDisplayed()
            ui.onNodeWithText("sip.answer").assertIsEnabled().assertHeightIsAtLeast(48.dp)
            ui.onNodeWithText("sip.hangup").performClick()
            ui.runOnIdle { assertNull(f.controller.call) }
        } finally { f.close() }
    }
}
