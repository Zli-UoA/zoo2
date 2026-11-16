package zoo.phone

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*
import zoo.*

@OptIn(ExperimentalCoroutinesApi::class)
class PhoneControllerTest {
    private class Fixture(val scope: CoroutineScope) {
        val requests = mutableListOf<String>()
        val commands = mutableListOf<String>()
        val events = mutableMapOf<String, (String, Row) -> Unit>()
        val closed = mutableListOf<String>()
        val native = mutableListOf<String>()
        var number = 0
        var revision = "v1"
        var authorize = true
        var permission = true
        var heartbeatTimeout = false
        var signalTimeouts = 0
        var streamTimeout = false
        var phoneStreamTimeout = false
        var pendingAccept: CompletableDeferred<JsonElement>? = null
        var beforeRead: CompletableDeferred<Unit>? = null
        var streamEvent: (suspend (String, Row) -> Unit)? = null
        var phoneEvent: (suspend (String, Row) -> Unit)? = null
        var pendingPhoneCall: CompletableDeferred<Unit>? = null
        val phoneActions = mutableListOf<String>()
        val line = obj("id" to "home", "name" to "Home", "can_connect" to true, "can_manage" to true)
        val controller = PhoneController(scope, request = { path, method, body ->
            requests.add("$method $path")
            when {
                path == "/api/sip/lines" -> { beforeRead?.await(); obj("enabled" to true, "lines" to JsonArray(listOf(line))) }
                path.endsWith("/connect") -> obj("data" to obj("revision" to revision, "session_id" to "phone-session", "peer_id" to "gateway-peer", "ice_servers" to JsonArray(emptyList())))
                path.endsWith("/authorize") -> { check(authorize); obj("data" to obj("revision" to revision)) }
                path.endsWith("/accept") -> pendingAccept?.await() ?: obj("data" to obj("channel_id" to "dm"))
                path.endsWith("/call/heartbeat") && heartbeatTimeout -> { delay(20_000); obj() }
                path.endsWith("/call/signal") && signalTimeouts > 0 -> { signalTimeouts--; delay(20_000); obj() }
                path.endsWith("/command") -> {
                    val action = body?.text("action").orEmpty(); phoneActions.add(action)
                    if (action == "call") pendingPhoneCall?.await()
                    if (action in listOf("call", "accept")) phoneEvent?.invoke("accepted", obj())
                    if (action in listOf("hangup", "decline")) phoneEvent?.invoke("ended", obj())
                    obj("data" to obj())
                }
                else -> obj("data" to obj())
            }
        }, stream = { _, _, callback -> streamEvent = callback; if (streamTimeout) withTimeout(1) { awaitCancellation() } else awaitCancellation() }, phoneStream = { _, callback -> phoneEvent = callback; callback("registered", obj()); if (phoneStreamTimeout) withTimeout(1) { awaitCancellation() } else awaitCancellation() }, media = { kind, callback ->
            events[kind] = callback
            object : PhoneMedia {
                override suspend fun command(method: String, payload: Row) {
                    commands.add(method)
                    if (kind == "sip" && method == "rtc.start" && payload["room_state"]?.jsonObject?.text("offerer_session_id") == payload.text("session_id")) callback("signal", obj("signal" to obj("type" to "offer", "sdp" to "v=0")))
                    if (kind == "sip" && method == "rtc.event" && payload["data"]?.jsonObject?.get("signal")?.jsonObject?.text("type") == "offer") callback("signal", obj("signal" to obj("type" to "answer", "sdp" to "v=0")))
                }
                override fun close() { closed.add(kind) }
            }
        }, telecom = object : PhoneTelecom {
            override suspend fun outgoing(call: PhoneCall) { native.add("outgoing:${call.kind}") }
            override fun incoming(call: PhoneCall) { native.add("incoming:${call.kind}") }
            override fun ended(id: String) { native.add("end:$id") }
            override fun speaker(id: String, enabled: Boolean) { native.add("speaker:$enabled") }
        }, prepare = { check(permission) }, waiting = { native.add("waiting:$it") }, uuid = { "id-${++number}" })
        fun invitation() = obj("id" to "invite", "channel_id" to "dm", "invitee_id" to "alice", "inviter_name" to "Bob", "status" to "pending", "expires_at" to "2030-01-01T00:00:30Z")
    }
    @Test fun registrationDoesNotPlaceANativeCallAndExplicitDialRechecksMembership() = runTest {
        val f = Fixture(backgroundScope); f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
        assertEquals("ready", f.controller.registration.status); assertFalse(f.controller.lines.toString().contains("dummy"))
        assertTrue(f.native.isEmpty()); assertTrue(f.events.isEmpty()); f.controller.dialSip("101"); runCurrent()
        assertEquals(listOf("outgoing:sip"), f.native); assertTrue(f.requests.any { it.endsWith("/authorize") }); assertEquals("rtc.start", f.commands.last())
        f.events.getValue("sip")("connected", obj()); assertEquals("active", f.controller.call?.status)
        f.controller.hangup(); runCurrent(); assertNull(f.controller.call); assertEquals("ready", f.controller.registration.status)
        f.controller.close()
    }
    @Test fun sipIncomingWaitsForExplicitAnswerAndChecksAuthorizationAgain() = runTest {
        for (allowed in listOf(true, false)) {
            val f = Fixture(backgroundScope); f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
            f.phoneEvent!!("incoming", obj("signal" to obj("type" to "offer", "sdp" to "v=0"))); runCurrent()
            assertEquals("incoming:sip", f.native.last()); assertEquals("incoming", f.controller.call?.status)
            assertFalse("accept" in f.phoneActions)
            f.authorize = allowed; f.controller.answer(); runCurrent()
            if (allowed) { assertTrue("accept" in f.phoneActions); assertEquals("connecting", f.controller.call?.status) }
            else { assertFalse("accept" in f.phoneActions); assertNull(f.controller.call); assertTrue(f.controller.error) }
            f.controller.close()
        }
    }
    @Test fun rejectedAuthorizationOrChangedSettingsNeverReachTelecom() = runTest {
        for (changed in listOf(false, true)) {
            val f = Fixture(backgroundScope); f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
            if (changed) f.revision = "v2" else f.authorize = false
            f.controller.dialSip("101"); runCurrent()
            assertFalse("outgoing:sip" in f.native); assertFalse("call" in f.phoneActions); assertTrue(f.controller.error); assertNull(f.controller.call)
            f.controller.close()
        }
    }
    @Test fun permissionsAreRequiredBeforeCreatingAMediaEngine() = runTest {
        val f = Fixture(backgroundScope); f.permission = false; f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
        assertTrue(f.events.isEmpty()); assertEquals("offline", f.controller.registration.status); assertTrue(f.controller.error)
        f.controller.close()
    }
    @Test fun inviteRemovalDuringAcceptDoesNotCancelTheAcceptedCall() = runTest {
        val f = Fixture(backgroundScope); f.controller.receiveDmCalls(true); runCurrent()
        f.controller.invites(listOf(f.invitation()), "alice", 1893456000000)
        assertEquals("incoming:dm", f.native.last()); assertTrue(f.commands.isEmpty())
        f.pendingAccept = CompletableDeferred(); f.controller.answer(); runCurrent()
        f.controller.invites(emptyList(), "alice", 1893456000100)
        assertNotNull(f.controller.call)
        f.pendingAccept!!.complete(obj("data" to obj("channel_id" to "dm"))); runCurrent()
        assertEquals("dialing", f.controller.call?.status); assertNotNull(f.streamEvent)
        f.streamEvent!!("room_state", obj("ice_servers" to JsonArray(emptyList()))); runCurrent()
        assertTrue("rtc.start" in f.commands)
        f.events.getValue("dm")("connected", obj()); assertEquals("active", f.controller.call?.status)
        f.controller.close(); runCurrent(); assertNull(f.controller.call); assertTrue("dm" in f.closed)
    }
    @Test fun expiredForeignAndDuplicateInvitesNeverCreateAdditionalNativeCalls() = runTest {
        val f = Fixture(backgroundScope); f.controller.receiveDmCalls(true); runCurrent()
        f.controller.invites(listOf(f.invitation()), "bob", 1893456000000); assertNull(f.controller.call)
        f.controller.invites(listOf(f.invitation()), "alice", 1893456040000); assertNull(f.controller.call)
        repeat(2) { f.controller.invites(listOf(f.invitation()), "alice", 1893456000000) }
        assertEquals(1, f.native.count { it == "incoming:dm" })
        advanceTimeBy(30_001); runCurrent(); assertNull(f.controller.call)
        f.controller.close()
    }
    @Test fun endingTheSessionDropsLateDirectoryAndMediaCallbacks() = runTest {
        val f = Fixture(backgroundScope); f.beforeRead = CompletableDeferred(); f.controller.refresh(); runCurrent()
        f.controller.close(); f.beforeRead!!.complete(Unit); runCurrent()
        assertTrue(f.controller.lines.isEmpty()); assertFalse(f.controller.loaded)
        f.controller.connectSip("home"); runCurrent(); assertTrue(f.events.isEmpty())
    }
    @Test fun dmOutgoingReservesTelecomBeforeJoiningAndTimesOutWhenUnanswered() = runTest {
        val f = Fixture(backgroundScope); f.controller.callDm("dm", "Bob"); runCurrent()
        assertEquals(listOf("outgoing:dm"), f.native); assertNotNull(f.streamEvent); assertTrue(f.commands.isEmpty())
        advanceTimeBy(45_001); runCurrent(); assertNull(f.controller.call)
        assertTrue(f.requests.any { it.endsWith("/call/leave") }); f.controller.close()
    }
    @Test fun earlySignalsAreBufferedUntilTheInitialRoomState() = runTest {
        val f = Fixture(backgroundScope); f.controller.callDm("dm", "Bob"); runCurrent()
        f.streamEvent!!("signal", obj("from_session_id" to "remote", "signal" to obj("type" to "offer"))); runCurrent()
        assertTrue(f.commands.isEmpty())
        f.streamEvent!!("room_state", obj("participants" to JsonArray(emptyList()))); runCurrent()
        assertEquals(listOf("rtc.start", "rtc.event", "rtc.event"), f.commands)
        f.controller.close()
    }
    @Test fun directoryTimeoutIsVisibleAndCanBeRetried() = runTest {
        val f = Fixture(backgroundScope); f.beforeRead = CompletableDeferred(); f.controller.refresh(); runCurrent()
        advanceTimeBy(15_001); runCurrent(); assertTrue(f.controller.loadError); assertFalse(f.controller.loading)
        f.beforeRead = null; f.controller.refresh(); runCurrent()
        assertFalse(f.controller.loadError); assertTrue(f.controller.loaded); f.controller.close()
    }
    @Test fun oldQueuedHangupCompletesBeforeAnotherTelephoneCallCanStart() = runTest {
        val f = Fixture(backgroundScope); f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
        f.pendingPhoneCall = CompletableDeferred(); f.controller.dialSip("101"); runCurrent()
        f.controller.hangup(); runCurrent(); assertEquals("connecting", f.controller.registration.status)
        f.controller.dialSip("102"); runCurrent(); assertEquals(1, f.native.count { it == "outgoing:sip" })
        f.pendingPhoneCall!!.complete(Unit); runCurrent(); assertEquals("ready", f.controller.registration.status)
        f.controller.dialSip("102"); runCurrent(); assertEquals(listOf("call", "hangup", "call"), f.phoneActions)
        f.controller.close()
    }
    @Test fun telephoneCandidatesReceivedBeforeAnswerAreRetainedWithoutCapture() = runTest {
        val f = Fixture(backgroundScope); f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
        f.phoneEvent!!("incoming", obj("signal" to obj("type" to "offer", "sdp" to "v=0")))
        f.phoneEvent!!("signal", obj("from_session_id" to "gateway-peer", "signal" to obj("type" to "candidate", "candidate" to obj("candidate" to "dummy"))))
        assertTrue(f.commands.isEmpty()); f.controller.answer(); runCurrent()
        assertEquals(listOf("rtc.start", "rtc.event", "rtc.event"), f.commands)
        f.controller.close()
    }
    @Test fun simultaneousSipAndDmCallsAreNotCreated() = runTest {
        val f = Fixture(backgroundScope); f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
        f.controller.callDm("dm", "Bob"); runCurrent(); f.controller.dialSip("101"); runCurrent()
        assertEquals(1, f.native.count { it.startsWith("outgoing:") }); assertFalse("call" in f.phoneActions)
        f.controller.close()
    }
    @Test fun lateAcceptCannotEraseTheNextDmInvitation() = runTest {
        val f=Fixture(backgroundScope); f.controller.receiveDmCalls(true); runCurrent()
        f.controller.invites(listOf(f.invitation()),"alice",1893456000000)
        val first=CompletableDeferred<JsonElement>(); f.pendingAccept=first
        f.controller.answer(); runCurrent(); f.controller.hangup(); runCurrent()
        val second=JsonObject(f.invitation() + ("id" to JsonPrimitive("next-invite")))
        f.controller.invites(listOf(second),"alice",1893456000000)
        first.complete(obj("data" to obj("channel_id" to "dm"))); runCurrent()
        assertEquals("incoming",f.controller.call?.status)
        f.pendingAccept=null; f.controller.answer(); runCurrent()
        assertTrue(f.requests.any {it.endsWith("/next-invite/accept")})
        assertEquals("dialing",f.controller.call?.status); f.controller.close()
    }
    @Test fun heartbeatTimeoutClosesMediaAndShowsAnError() = runTest {
        val f=Fixture(backgroundScope); f.controller.callDm("dm","Bob"); runCurrent()
        f.streamEvent!!("room_state",obj()); f.events.getValue("dm")("connected",obj())
        f.heartbeatTimeout=true; advanceTimeBy(25_001); runCurrent()
        assertNull(f.controller.call); assertTrue(f.controller.error); assertTrue("dm" in f.closed)
        f.controller.close()
    }
    @Test fun streamTimeoutEndsTheCallInsteadOfLeavingADialingCall() = runTest {
        val f=Fixture(backgroundScope); f.streamTimeout=true; f.controller.callDm("dm","Bob"); runCurrent()
        advanceTimeBy(2); runCurrent(); assertNull(f.controller.call); assertTrue(f.controller.error)
        f.controller.close()
    }
    @Test fun timedOutSignalsAreRetriedWithoutEndingTheActiveCall() = runTest {
        val f=Fixture(backgroundScope); f.controller.callDm("dm","Bob"); runCurrent()
        f.streamEvent!!("room_state",obj()); f.events.getValue("dm")("connected",obj())
        f.signalTimeouts=1
        f.events.getValue("dm")("signal",obj("target_session_id" to "other","signal" to obj("type" to "offer","sdp" to "v=0")))
        runCurrent(); advanceTimeBy(15_201); runCurrent()
        assertEquals(2,f.requests.count {it.endsWith("/call/signal")})
        assertEquals("active",f.controller.call?.status); assertFalse(f.controller.error)
        f.controller.close()
    }

    @Test fun telephoneStreamTimeoutDisconnectsRegistrationAndShowsAnError() = runTest {
        val f=Fixture(backgroundScope); f.phoneStreamTimeout=true
        f.controller.refresh(); runCurrent(); f.controller.connectSip("home"); runCurrent()
        advanceTimeBy(2); runCurrent()
        assertEquals("offline",f.controller.registration.status); assertTrue(f.controller.error)
        f.controller.close()
    }

}
