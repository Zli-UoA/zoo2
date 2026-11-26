package zoo.android.phone

import android.app.Activity
import android.app.Notification
import android.app.PictureInPictureParams
import android.app.NotificationManager
import android.app.Service
import android.content.pm.ServiceInfo
import android.content.Intent
import android.media.ToneGenerator
import androidx.compose.runtime.snapshots.Snapshot
import android.telecom.CallAudioState
import android.telecom.Connection
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respondError
import io.ktor.http.HttpStatusCode
import io.ktor.http.encodedPath
import kotlin.test.*
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.shadows.ShadowActivity
import org.robolectric.shadows.ShadowService
import org.robolectric.shadows.ShadowToast
import org.robolectric.shadows.ShadowToneGenerator
import org.robolectric.util.ReflectionHelpers
import zoo.Environment
import zoo.Platform
import zoo.Repository
import zoo.Session
import zoo.Row
import zoo.SyncedTable
import zoo.android.PushBinding
import zoo.obj
import zoo.phone.PhoneCall

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 35])
@OptIn(ExperimentalCoroutinesApi::class)
class TelecomIntegrationTest {
    private class Fixture(scope: CoroutineScope, holdDmStream: Boolean = false) : AutoCloseable {
        val context = RuntimeEnvironment.getApplication()
        val shadow = shadowOf(context)
        val notifications = context.getSystemService(NotificationManager::class.java)
        val routes = mutableListOf<String>()
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "phone-test"
            override fun now() = "2026-11-21T00:00:00Z"
            override fun navigate(path: String) { routes.add(path) }
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val http = HttpClient(MockEngine { request ->
            if (holdDmStream && request.url.encodedPath.endsWith("/call/stream")) awaitCancellation()
            respondError(HttpStatusCode.ServiceUnavailable)
        })
        val config = zoo.Config("https://zoo.test", "/api/shapes", "alice", false)
        val session = Session(Repository(config, platform, http, scope, synchronize = false),
            scope, Environment(visible = false))
        var microphoneAllowed = true
        val host = AndroidPhoneHost(context, session, http, prepare = { check(microphoneAllowed) })

        fun ring(inviteId: String = "invite"): ZooConnection {
            host.controller.invites(listOf(obj(
                "id" to inviteId, "channel_id" to "target-dm", "invitee_id" to "alice",
                "inviter_name" to "Bob", "status" to "pending", "expires_at" to "2026-11-21T00:00:30Z",
            )), "alice", 1795219200000)
            return (host.connection(true, null) as ZooConnection).also { it.onShowIncomingCallUi() }
        }

        fun activePictureInPictureIntent(): Intent {
            val call = PhoneCall("active-call", "dm", "target-dm", "Bob", "active")
            setCall(call)
            return Intent(context, PhonePictureInPictureActivity::class.java)
                .putExtra(PHONE_SESSION, host.token).putExtra(PHONE_CALL, call.id)
        }

        fun setCall(call: PhoneCall) {
            ReflectionHelpers.callInstanceMethod<Unit>(host.controller, "setCall",
                ReflectionHelpers.ClassParameter.from(PhoneCall::class.java, call))
            Snapshot.sendApplyNotifications()
        }

        override fun close() {
            host.close()
            session.close()
            http.close()
        }
    }

    @Test fun pictureInPictureRejectsUnconnectedAndMismatchedCalls() = runTest {
        Fixture(backgroundScope).use { f ->
            f.host.controller.receiveDmCalls(true)
            runCurrent()
            f.ring()
            val id = assertNotNull(f.host.controller.call).id
            assertNull(f.host.incomingCaller("other-call"))
            assertNull(f.host.incomingCaller(id)) // The authenticated invite and user have not synced.
            PhonePictureInPictureActivity.open(f.context, f.host, id)
            assertNull(f.shadow.nextStartedActivity)

            for ((token, call) in listOf(f.host.token to id, "stale-session" to id, f.host.token to "other-call")) {
                val intent = Intent(f.context, PhonePictureInPictureActivity::class.java)
                    .putExtra(PHONE_SESSION, token).putExtra(PHONE_CALL, call)
                val activity = Robolectric.buildActivity(PhonePictureInPictureActivity::class.java, intent).create().get()
                assertTrue(activity.isFinishing)
            }
        }
    }

    @Test fun expandedPictureInPictureCanReenterWhenTheActivityIsReused() = runTest {
        Fixture(backgroundScope).use { f ->
            val intent = f.activePictureInPictureIntent()
            val activity = Robolectric.buildActivity(PhonePictureInPictureActivity::class.java, intent)
                .create().start().resume().postResume()
            try {
                val screen = activity.get()
                assertTrue(screen.isInPictureInPictureMode)

                activity.newIntent(Intent(intent))
                // Robolectric's shadow does not expose a public exit transition.
                ReflectionHelpers.setField(shadowOf(screen), "isInPictureInPictureMode", false)
                activity.postResume()
                assertFalse(screen.isInPictureInPictureMode)

                activity.newIntent(Intent(intent).putExtra(PHONE_SESSION, "stale-session")).postResume()
                assertFalse(screen.isInPictureInPictureMode)
                activity.newIntent(Intent(intent)).postResume()
                assertTrue(screen.isInPictureInPictureMode)
            } finally {
                activity.pause().stop().destroy()
            }
        }
    }

    @Implements(Activity::class)
    class PictureInPictureDenied : ShadowActivity() {
        @Implementation
        override fun enterPictureInPictureMode(params: PictureInPictureParams): Boolean = false
    }

    @Test
    @Config(shadows = [PictureInPictureDenied::class])
    fun pictureInPictureDenialExplainsHowToEnableItWithoutEndingTheCall() = runTest {
        Fixture(backgroundScope).use { f ->
            val activity = Robolectric.buildActivity(PhonePictureInPictureActivity::class.java, f.activePictureInPictureIntent())
                .create().start().resume().postResume()
            try {
                assertTrue(activity.get().isFinishing)
                assertEquals(phoneLabel(f.context, "androidPhone.pipUnavailable", "Check Picture-in-picture in Android settings."),
                    ShadowToast.getTextOfLatestToast())
                assertEquals("active", f.host.controller.call?.status)
            } finally {
                activity.pause().stop().destroy()
            }
        }
    }

    @Test fun enablingDmWaitingDoesNotStartAServiceOrPostANotification() = runTest {
        Fixture(backgroundScope).use {
            it.host.controller.receiveDmCalls(true)
            runCurrent()
            assertTrue(it.host.controller.dmWaiting)
            assertNull(it.shadow.nextStartedService)
            assertTrue(it.notifications.activeNotifications.isEmpty())
        }
    }

    @Test fun incomingNotificationOpensDmAndAnswerPromotesOneServiceUntilHangup() = runTest {
        Fixture(backgroundScope).use {
            it.host.controller.receiveDmCalls(true)
            runCurrent()
            val call = it.ring()
            assertNull(it.shadow.nextStartedService)
            val incoming = it.notifications.activeNotifications.single().notification
            assertEquals(2, incoming.actions.size)
            var presented = 0
            assertTrue(it.host.open(Intent("zoo.phone.OPEN")) { presented++ })
            assertEquals(0, presented)
            assertTrue(it.host.open(shadowOf(incoming.contentIntent).savedIntent) { presented++ })
            assertEquals(1, presented)
            val answer = incoming.actions.single { action ->
                shadowOf(action.actionIntent).savedIntent.action == "zoo.phone.ANSWER"
            }
            assertTrue(it.host.open(shadowOf(answer.actionIntent).savedIntent))
            assertEquals(listOf("/channels/target-dm", "/channels/target-dm"), it.routes)
            runCurrent()
            val start = assertNotNull(it.shadow.nextStartedService)
            val service = Robolectric.buildService(PhoneService::class.java).create().get()
            assertTrue(it.host.controller.busy)
            service.onStartCommand(start, 0, 1)
            assertTrue(ShadowToneGenerator.getPlayedTones().isEmpty())
            assertNotNull(shadowOf(service).lastForegroundNotification)
            assertTrue(it.notifications.activeNotifications.none { n -> n.id == 604 })

            val receiver = PhoneActionReceiver()
            val end = Intent("zoo.phone.END").putExtra(PHONE_SESSION, it.host.token)
                .putExtra(PHONE_CALL, call.id)
            receiver.onReceive(it.context, Intent(end).putExtra(PHONE_SESSION, "old-session"))
            receiver.onReceive(it.context, Intent(end).putExtra(PHONE_CALL, "old-call"))
            assertEquals(call.id, it.host.controller.call?.id)
            receiver.onReceive(it.context, end)
            assertNull(it.host.controller.call)
            assertEquals(start.component, it.shadow.nextStoppedService?.component)
            service.onDestroy()
        }
    }

    @Test fun hangupBeforeServiceDeliveryAcknowledgesStartWithoutMicrophone() = runTest {
        Fixture(backgroundScope).use {
            it.host.controller.receiveDmCalls(true)
            runCurrent()
            val call = it.ring()
            call.onAnswer()
            runCurrent()
            val start = assertNotNull(it.shadow.nextStartedService)
            call.onDisconnect()
            assertNull(it.host.controller.call)
            assertNull(it.shadow.nextStoppedService)

            val service = Robolectric.buildService(PhoneService::class.java).create().get()
            service.onStartCommand(start, 0, 1)
            assertNotNull(shadowOf(service).lastForegroundNotification)
            if (android.os.Build.VERSION.SDK_INT >= 29) {
                assertEquals(android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL,
                    service.foregroundServiceType)
            }
            assertTrue(shadowOf(service).isStoppedBySelf)
            assertNull(it.host.controller.call)
            service.onDestroy()
        }
    }

    @Implements(Service::class)
    class MicrophoneDeniedServiceShadow : ShadowService() {
        @Implementation
        override fun startForeground(id: Int, notification: Notification, type: Int) {
            if (type and ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE != 0) {
                throw SecurityException("Microphone access denied")
            }
            super.startForeground(id, notification, type)
        }
    }

    @Test
    @Config(sdk = [35], shadows = [MicrophoneDeniedServiceShadow::class])
    fun nativeMicrophonePromotionFailureAcknowledgesStartAndEndsCall() = runTest {
        Fixture(backgroundScope).use {
            it.host.controller.receiveDmCalls(true)
            runCurrent()
            it.ring().onAnswer()
            runCurrent()
            val start = assertNotNull(it.shadow.nextStartedService)
            val service = Robolectric.buildService(PhoneService::class.java).create().get()
            service.onStartCommand(start, 0, 1)
            assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL, service.foregroundServiceType)
            assertNotNull(shadowOf(service).lastForegroundNotification)
            assertTrue(shadowOf(service).isStoppedBySelf)
            assertNull(it.host.controller.call)
            assertTrue(it.host.controller.error)
            service.onDestroy()
        }
    }

    @Test fun notificationAnswerWithDeniedMicrophoneDoesNotStartService() = runTest {
        Fixture(backgroundScope).use {
            it.host.controller.receiveDmCalls(true)
            runCurrent()
            it.ring()
            it.microphoneAllowed = false
            val incoming = it.notifications.activeNotifications.single().notification
            val answer = incoming.actions.single { action ->
                shadowOf(action.actionIntent).savedIntent.action == "zoo.phone.ANSWER"
            }
            assertTrue(it.host.open(shadowOf(answer.actionIntent).savedIntent))
            runCurrent()
            assertNull(it.shadow.nextStartedService)
            assertNull(it.host.controller.call)
            assertTrue(it.host.controller.error)
        }
    }

    @Test fun pushAnswerWaitsForTheMatchingAuthenticatedInviteAndTelecomConnection() = runTest {
        Fixture(backgroundScope).use {
            val store = PushBinding(it.context)
            val binding = store.bind("https://zoo.test", "alice")
            store.setDmCallsEnabled("alice", true)
            val id = "5f3d960a-49bd-4ebf-beae-4090a7e96950"
            val answer = Intent(DM_PUSH_ANSWER_ACTION).putExtra("invite", id)
                .putExtra("binding", binding).putExtra("expiresAt", System.currentTimeMillis() / 1000L + 30)
            assertTrue(it.host.open(answer))
            runCurrent()
            assertNull(it.shadow.nextStartedService)
            assertTrue(it.routes.isEmpty())

            it.ring(id)
            runCurrent()
            assertEquals(listOf("/channels/target-dm"), it.routes)
            assertNotNull(it.shadow.nextStartedService)
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
    @Implements(ToneGenerator::class)
    class RingbackShadow : ShadowToneGenerator() {
        @Implementation fun release() { releases++ }

        companion object {
            var releases = 0
        }
    }

    private fun kotlinx.coroutines.test.TestScope.startOutgoingService(f: Fixture, kind: String = "dm"): PhoneService {
        f.host.controller.callDm("target-dm", "Bob")
        runCurrent()
        val native = f.host.connection(incoming = false, address = null)
        assertEquals(Connection.STATE_DIALING, native.state)
        assertFalse(native.isRingbackRequested)
        runCurrent()
        val start = assertNotNull(f.shadow.nextStartedService)
        if (kind != "dm") f.setCall(assertNotNull(f.host.controller.call).copy(kind = kind))
        return Robolectric.buildService(PhoneService::class.java).create().get().also {
            it.onStartCommand(start, 0, 1)
        }
    }

    @Test
    @Config(shadows = [RingbackShadow::class])
    fun outgoingDmRingbackIsReleasedWhenTheCallConnects() = runTest {
        Fixture(backgroundScope).use { f ->
            RingbackShadow.releases = 0
            val service = startOutgoingService(f)
            try {
                assertEquals(ToneGenerator.TONE_SUP_RINGTONE, ShadowToneGenerator.getPlayedTones().single().type())
                assertEquals(0, RingbackShadow.releases)
                f.setCall(assertNotNull(f.host.controller.call).copy(status = "active"))
                runCurrent()
                assertEquals(1, RingbackShadow.releases)
            } finally {
                f.host.controller.hangup()
                service.onDestroy()
                runCurrent()
            }
        }
    }

    @Test
    @Config(shadows = [RingbackShadow::class])
    fun outgoingDmRingbackIsReleasedWhenDialingEnds() = runTest {
        Fixture(backgroundScope).use { f ->
            RingbackShadow.releases = 0
            val service = startOutgoingService(f)
            try {
                assertEquals(0, RingbackShadow.releases)
                f.host.controller.hangup()
                assertEquals(1, RingbackShadow.releases)
            } finally {
                service.onDestroy()
                runCurrent()
            }
        }
    }

    @Test
    @Config(shadows = [RingbackShadow::class])
    fun ringbackStopsWhenTheLastOutgoingInviteDisappears() = runTest {
        Fixture(backgroundScope).use { f ->
            RingbackShadow.releases = 0
            val service = startOutgoingService(f)
            try {
                val invite = obj("channel_id" to "target-dm", "inviter_id" to "alice",
                    "status" to "pending", "expires_at" to "2099-01-01T00:00:00Z")
                f.host.updateRingbackInvites(emptyList())
                f.host.updateRingbackInvites(listOf(invite))
                assertEquals(0, RingbackShadow.releases)
                f.host.updateRingbackInvites(emptyList())
                assertEquals(1, RingbackShadow.releases)
                assertEquals("dialing", f.host.controller.call?.status)
            } finally {
                f.host.controller.hangup()
                service.onDestroy()
                runCurrent()
            }
        }
    }

    @Test
    @Config(shadows = [RingbackShadow::class])
    fun ringbackTimesOutEvenWithoutInviteSync() = runTest {
        Fixture(backgroundScope).use { f ->
            RingbackShadow.releases = 0
            val service = startOutgoingService(f)
            try {
                advanceTimeBy(30_000)
                runCurrent()
                assertEquals(1, RingbackShadow.releases)
            } finally {
                f.host.controller.hangup()
                service.onDestroy()
                runCurrent()
            }
        }
    }

    @Test fun enablingWaitingReevaluatesTheLatestSnapshotOnTheRetainedRingbackSubscription() = runTest {
        Fixture(backgroundScope, holdDmStream = true).use { f ->
            val service = startOutgoingService(f)
            try {
                f.host.controller.receiveDmCalls(true)
                runCurrent()
                val table = assertNotNull(ReflectionHelpers.getField<SyncedTable?>(f.host, "inviteTable"))
                f.host.controller.receiveDmCalls(false)
                runCurrent()
                Snapshot.sendApplyNotifications()
                runCurrent()
                assertSame(table, ReflectionHelpers.getField<SyncedTable>(f.host, "inviteTable"))

                val channels = ReflectionHelpers.getField<MutableStateFlow<List<Row>>>(f.session.repo.channels, "serverRows")
                val invites = ReflectionHelpers.getField<MutableStateFlow<List<Row>?>>(table, "successfulSnapshot")
                channels.value = listOf(obj("id" to "other-dm", "is_dm" to true))
                invites.value = listOf(obj("id" to "pending", "channel_id" to "other-dm",
                    "inviter_id" to "bob", "invitee_id" to "alice", "status" to "pending",
                    "expires_at" to "2099-01-01T00:00:00Z"))
                runCurrent()
                val seen = ReflectionHelpers.getField<Set<String>>(f.host.controller, "seenInvites")
                assertFalse("pending" in seen)

                f.host.controller.receiveDmCalls(true)
                runCurrent()
                Snapshot.sendApplyNotifications()
                runCurrent()
                assertSame(table, ReflectionHelpers.getField<SyncedTable>(f.host, "inviteTable"))
                assertTrue("pending" in seen)
            } finally {
                f.host.controller.hangup()
                service.onDestroy()
                runCurrent()
            }
        }
    }

    @Test fun outgoingSipDoesNotPlayLocalRingback() = runTest {
        Fixture(backgroundScope).use { f ->
            val service = startOutgoingService(f, kind = "sip")
            try {
                assertTrue(ShadowToneGenerator.getPlayedTones().isEmpty())
            } finally {
                f.host.controller.hangup()
                service.onDestroy()
                runCurrent()
            }
        }
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
