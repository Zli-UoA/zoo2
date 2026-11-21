package zoo.android.phone

import android.app.NotificationManager
import android.telecom.CallAudioState
import android.telecom.Connection
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respondError
import io.ktor.http.HttpStatusCode
import kotlin.test.*
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import zoo.Environment
import zoo.Platform
import zoo.Repository
import zoo.Session

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 35])
class TelecomIntegrationTest {
    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun enablingDmWaitingDoesNotStartAServiceOrPostANotification() = runTest {
        val context = RuntimeEnvironment.getApplication()
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "phone-test"
            override fun now() = "2026-11-21T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val http = HttpClient(MockEngine { respondError(HttpStatusCode.ServiceUnavailable) })
        val config = zoo.Config("https://zoo.test", "/api/shapes", "alice", false)
        val session = Session(Repository(config, platform, http, backgroundScope, synchronize = false),
            backgroundScope, Environment(visible = false))
        val host = AndroidPhoneHost(context, session, http, prepare = {})
        try {
            host.controller.receiveDmCalls(true)
            runCurrent()
            assertTrue(host.controller.dmWaiting)
            assertNull(shadowOf(context).nextStartedService)
            assertTrue(context.getSystemService(NotificationManager::class.java).activeNotifications.isEmpty())
        } finally {
            host.close()
            session.close()
            http.close()
        }
    }

    private class Actions : ConnectionActions {
        var current = "call"
        var appMuted = false
        val calls = mutableListOf<String>()
        override fun valid(id: String) = id == current
        override fun answer(id: String) { calls.add("answer:$id") }
        override fun end(id: String) { calls.add("end:$id") }
        override fun tone(id: String, tone: String) { calls.add("tone:$tone") }
        override fun incomingUi(id: String) { calls.add("incoming:$id") }
        override fun audio(id: String, speaker: Boolean, muted: Boolean?) { calls.add("audio:$speaker:$muted"); if (muted != null) appMuted = muted }
    }
    @Test fun telecomAnswerHeadsetDisconnectAndDtmfReachOnlyTheCurrentCall() {
        val actions = Actions(); val call = ZooConnection(actions, "call", true)
        assertEquals(Connection.STATE_RINGING, call.state)
        assertTrue(call.connectionProperties and Connection.PROPERTY_SELF_MANAGED != 0)
        assertTrue(call.connectionCapabilities and Connection.CAPABILITY_HOLD == 0)
        call.onShowIncomingCallUi(); call.onAnswer(); call.onPlayDtmfTone('#'); call.onDisconnect()
        assertEquals(listOf("incoming:call", "answer:call", "tone:#", "end:call"), actions.calls)
        actions.current = "next"
        call.onAnswer(); call.onReject(); call.onDisconnect(); call.onPlayDtmfTone('1')
        assertEquals(4, actions.calls.size)
    }
    @Suppress("DEPRECATION")
    @Test fun nativeSpeakerAndMuteChangesAreForwarded() {
        val actions = Actions(); val call = ZooConnection(actions, "call", false)
        assertEquals(Connection.STATE_DIALING, call.state)
        call.onCallAudioStateChanged(CallAudioState(true, CallAudioState.ROUTE_SPEAKER, CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE))
        assertEquals(listOf("audio:true:true"), actions.calls)
        actions.current = "other"
        call.onCallAudioStateChanged(CallAudioState(false, CallAudioState.ROUTE_EARPIECE, CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE))
        assertEquals(1, actions.calls.size)
    }
    @Test fun onlyPackagedAssetsAtTheAuthenticatedHttpsOriginCanLoad() {
        val origin = phoneOrigin("https://zli.chat/")
        assertEquals("https://zli.chat", origin)
        assertEquals("engine.js", phoneAsset(origin, "$origin/__zoo_phone__/engine.js"))
        for (url in listOf("https://zli.chat.evil/__zoo_phone__/engine.js", "$origin/api/users/me", "$origin/__zoo_phone__/%2e%2e/engine.js", "$origin/__zoo_phone__/engine.js?secret=x", "file:///engine.js", "http://zli.chat/__zoo_phone__/engine.js")) assertNull(phoneAsset(origin, url))
        for (url in listOf("http://zli.chat", "https://user@zli.chat", "https://zli.chat/path", "https://zli.chat?x=y")) assertFails { phoneOrigin(url) }
    }
    @Suppress("DEPRECATION")
    @Test fun routeNotificationsDoNotUndoAppMuteAndNextCallsResetNativeMute() {
        val actions=Actions(); val call=ZooConnection(actions,"call",false)
        actions.appMuted=true
        call.onCallAudioStateChanged(CallAudioState(false,CallAudioState.ROUTE_SPEAKER,(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE)))
        assertTrue(actions.appMuted)
        call.onCallAudioStateChanged(CallAudioState(true,CallAudioState.ROUTE_EARPIECE,(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE)))
        call.onCallAudioStateChanged(CallAudioState(false,CallAudioState.ROUTE_EARPIECE,(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE)))
        assertFalse(actions.appMuted)
        call.onCallAudioStateChanged(CallAudioState(true,CallAudioState.ROUTE_EARPIECE,(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE)))
        actions.current="next"; actions.appMuted=true
        ZooConnection(actions,"next",false).onCallAudioStateChanged(CallAudioState(false,CallAudioState.ROUTE_SPEAKER,(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE)))
        assertTrue(actions.appMuted)
    }
    @Test fun rejectedCallStyleFallsBackToAnswerAndEndActions() {
        val context=org.robolectric.RuntimeEnvironment.getApplication()
        val intent=android.app.PendingIntent.getBroadcast(context,1,android.content.Intent("answer"),android.app.PendingIntent.FLAG_IMMUTABLE)
        val answer=android.app.Notification.Action.Builder(null as android.graphics.drawable.Icon?,"Answer",intent).build()
        val end=android.app.Notification.Action.Builder(null as android.graphics.drawable.Icon?,"End",intent).build()
        val builder=android.app.Notification.Builder(context,"calls").setSmallIcon(android.R.drawable.ic_menu_call)
        if (android.os.Build.VERSION.SDK_INT>=31) builder.setStyle(android.app.Notification.CallStyle.forIncomingCall(android.app.Person.Builder().setName("Caller").build(),intent,intent))
        else builder.setActions(answer,end)
        val posted=mutableListOf<android.app.Notification>(); var attempts=0
        postIncomingNotification(builder,answer,end) { notification ->
            attempts++
            if (android.os.Build.VERSION.SDK_INT>=31 && attempts==1) throw IllegalArgumentException("CallStyle requires FGS")
            posted.add(notification)
        }
        assertEquals(if (android.os.Build.VERSION.SDK_INT>=31) 2 else 1,attempts)
        assertEquals(listOf("Answer","End"),posted.single().actions.map {it.title.toString()})
        assertTrue(posted.single().flags and android.app.Notification.FLAG_INSISTENT != 0)
    }

}
