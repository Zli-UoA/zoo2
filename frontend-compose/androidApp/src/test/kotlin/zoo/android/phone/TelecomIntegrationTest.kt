package zoo.android.phone

import android.telecom.CallAudioState
import android.telecom.Connection
import kotlin.test.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 35])
class TelecomIntegrationTest {
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
}
