package zoo.android.phone

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.net.Uri
import android.os.*
import android.telecom.*
import androidx.compose.runtime.snapshotFlow
import io.ktor.client.HttpClient
import io.ktor.http.encodeURLPathPart
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import java.util.UUID
import zoo.*
import zoo.android.PushBinding
import zoo.phone.*

/** One owner per authenticated session, shared by all screens, notifications and Telecom. */
internal class AndroidPhoneHost(
    private val context: Context,
    val app: Session,
    private val http: HttpClient,
    prepare: suspend () -> Unit,
) : PhoneTelecom, ConnectionActions {
    val token = UUID.randomUUID().toString()
    private val manager = context.getSystemService(TelecomManager::class.java)
    private val handle = PhoneAccountHandle(ComponentName(context, ZooConnectionService::class.java), "zoo.calls")
    private var available = false
    private var connection: ZooConnection? = null
    private var connectionReady: CompletableDeferred<Unit>? = null
    private var inviteTable: SyncedTable? = null
    private var invites: Job? = null
    private val pushBinding = PushBinding(context)
    private var incomingTag: String? = null
    private var currentInviteRows = emptyList<Row>()
    private data class PushedInvite(val invite: DmCallPushAnswer, val answer: Boolean)
    private val pushedInvite = MutableStateFlow<PushedInvite?>(null)
    private var pushedInviteExpiry: Job? = null
    private var audioReady: CompletableDeferred<Unit>? = null
    private var audioCallId: String? = null
    private var service: PhoneService? = null
    internal var pendingMute: Boolean? = null
    val controller: PhoneController = PhoneController(
        app.scope,
        request = { path, method, body -> app.api.request(path, method, body) },
        stream = app.api::streamCall,
        phoneStream = app.api::streamPhone,
        media = { kind, events -> PhoneWebView(context, app.config.apiBaseUrl, { mayCapture(kind) }, events) },
        telecom = this,
        prepare = { check(available); prepare() },
        waiting = ::waiting,
        uuid = { UUID.randomUUID().toString() },
    )
    private val state: Job
    init {
        current?.close(); current = this
        available = runCatching { manager.registerPhoneAccount(PhoneAccount.builder(handle, "Zoo").setCapabilities(PhoneAccount.CAPABILITY_SELF_MANAGED).setSupportedUriSchemes(listOf(PhoneAccount.SCHEME_TEL, PhoneAccount.SCHEME_SIP)).build()) }.isSuccess
        state = app.scope.launch {
            snapshotFlow { controller.call?.let { Triple(it.id, it.status, controller.busy) } }.collect {
                val call = controller.call
                if (call != null && call.status == "active" && connection?.id == call.id && !controller.busy) {
                    connection?.setRingbackRequested(false); connection?.setActive()
                    pendingMute?.let { mute -> pendingMute = null; controller.mute(mute) }
                }
                service?.refresh()
            }
        }
        if (available && pushBinding.dmCallsEnabled(app.config.userId)) controller.restoreDmWaiting()
    }
    private fun mayCapture(kind: String) =
        service != null && audioCallId == controller.call?.id && controller.call?.kind == kind &&
            context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
    override fun valid(id: String) = matches(token, id)
    override fun answer(id: String) { if (valid(id)) controller.answer() }
    override fun end(id: String) { if (valid(id)) controller.hangup() }
    override fun tone(id: String, tone: String) { if (valid(id)) controller.dtmf(tone) }
    override fun incomingUi(id: String) = showIncoming(id)
    override fun audio(id: String, speaker: Boolean, muted: Boolean?) {
        if (!valid(id)) return
        controller.audioRoute(speaker)
        if (muted == null) return
        if (controller.call?.status == "active" && !controller.busy) controller.mute(muted) else pendingMute = muted
    }
    fun matches(session: String?, id: String?) = current === this && session == token && id != null && controller.call?.id == id
    private fun extras(call: PhoneCall) = Bundle().apply { putString(PHONE_SESSION, token); putString(PHONE_CALL, call.id) }
    override suspend fun outgoing(call: PhoneCall) {
        check(available)
        check(controller.call?.id == call.id)
        val ready = CompletableDeferred<Unit>(); connectionReady = ready
        val options = Bundle().apply { putParcelable(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, handle); putBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS, extras(call)) }
        val address = if (call.kind == "sip") Uri.fromParts("tel", call.peer, null) else Uri.fromParts("sip", "dm@zoo.invalid", null)
        try { manager.placeCall(address, options); withTimeout(10_000) { ready.await() } }
        catch (_: SecurityException) { throw IllegalStateException("Telecom permission unavailable") }
        finally { if (connectionReady === ready) connectionReady = null }
    }
    override fun incoming(call: PhoneCall) {
        check(available)
        incomingTag = controller.ringingInviteId?.let { dmNotificationTag(app.config.userId, it) }
        manager.addNewIncomingCall(handle, extras(call))
    }
    fun connection(incoming: Boolean, address: Uri?): Connection {
        val call = controller.call ?: return Connection.createFailedConnection(DisconnectCause(DisconnectCause.CANCELED))
        if (connection != null || incoming != (call.status == "incoming")) {
            return Connection.createFailedConnection(DisconnectCause(DisconnectCause.CANCELED))
        }
        val native = ZooConnection(this, call.id, incoming)
        native.setCallerDisplayName(call.label.ifBlank { "Zoo" }, TelecomManager.PRESENTATION_ALLOWED)
        address?.let { native.setAddress(it, TelecomManager.PRESENTATION_ALLOWED) }
        connection = native; connectionReady?.complete(Unit)
        if (incoming && pushedInvite.value?.answer == true) app.scope.launch {
            yield() // Telecom must receive the Connection before the call is answered.
            openPushedInviteIfReady()
        }
        return native
    }
    fun failed() { connectionReady?.completeExceptionally(IllegalStateException("Telecom rejected the call")); controller.call?.id?.let(controller::nativeFailed) }
    fun incomingCaller(id: String): Row? {
        if (!valid(id) || controller.call?.status != "incoming") return null
        val inviteId = controller.ringingInviteId ?: return null
        val callerId = currentInviteRows.firstOrNull { it.text("id") == inviteId }?.text("inviter_id") ?: return null
        return app.repo.users.replicatedRows.value.firstOrNull { it.text("id") == callerId }
    }

    private fun showIncoming(id: String) {
        val call = controller.call?.takeIf { it.id == id && it.status == "incoming" } ?: return
        if (audioCallId == id) return
        try {
            showIncomingCall(context, this, call, incomingTag)
        } catch (_: RuntimeException) {
            controller.nativeFailed(id)
        }
    }
    private fun clearIncomingNotification() {
        synchronized(PushBinding.lock) {
            incomingTag?.substringAfterLast(':')?.let { pushBinding.rememberEndedDmInvite(app.config.userId, it) }
            dismissIncomingCall(context, incomingTag)
            incomingTag = null
        }
    }

    override fun ended(id: String) {
        connection?.takeIf { it.id == id }?.let { it.setDisconnected(DisconnectCause(DisconnectCause.LOCAL)); it.destroy(); connection = null }
        connectionReady?.cancel(); connectionReady = null; pendingMute = null
        clearIncomingNotification()
        if (audioCallId == id) {
            audioCallId = null
            audioReady?.cancel()
            audioReady = null
            // A queued foreground-service start must be acknowledged by onStartCommand.
            if (service != null) context.stopService(Intent(context, PhoneService::class.java))
            service = null
        }
    }
    @Suppress("DEPRECATION")
    override fun speaker(id: String, enabled: Boolean) {
        val native = connection?.takeIf { it.id == id } ?: return
        val supported = native.callAudioState?.supportedRouteMask ?: 0
        native.setAudioRoute(when {
            enabled -> CallAudioState.ROUTE_SPEAKER
            supported and CallAudioState.ROUTE_BLUETOOTH != 0 -> CallAudioState.ROUTE_BLUETOOTH
            else -> CallAudioState.ROUTE_WIRED_OR_EARPIECE
        })
    }
    private fun waiting(enabled: Boolean) {
        if (current === this) pushBinding.setDmCallsEnabled(app.config.userId, enabled)
        if (!enabled) {
            clearPushedInvite()
            currentInviteRows = emptyList()
        }
        invites?.cancel(); invites = null; inviteTable?.close(); inviteTable = null
        if (enabled) {
            val table = SyncedTable("call_invites", app.config, app.platform, http, app.scope, persist = false)
            inviteTable = table
            invites = app.scope.launch {
                val dmRows = combine(table.synchronizedRows.filterNotNull(), app.repo.channels.replicatedRows, app.repo.users.replicatedRows) { rows, channels, users ->
                    val dmIds = channels.filter { it.flag("is_dm") }.map { it.text("id") }.toSet()
                    rows.filter { it.text("channel_id") in dmIds }.map { row ->
                        val user = users.find { it.text("id") == row.text("inviter_id") }
                        kotlinx.serialization.json.JsonObject(row + ("inviter_name" to kotlinx.serialization.json.JsonPrimitive(user?.text("display_name").orEmpty().ifBlank { "Zoo" })))
                    }
                }
                combine(dmRows, pushedInvite) { rows, selection -> rows to selection }
                    .collect { (rows, selection) ->
                        currentInviteRows = rows
                        val now = kotlin.time.Clock.System.now().toEpochMilliseconds()
                        val target = selection?.invite?.takeIf {
                            it.remainingMillis() > 0 && !pushBinding.dmInviteEnded(it.id)
                        }?.id
                        if (target != null && selection?.answer == false) {
                            val row = rows.firstOrNull {
                                it.text("id") == target && it.text("invitee_id") == app.config.userId &&
                                    it.text("status") == "pending" && (timestampMillis(it.text("expires_at")) ?: 0L) > now
                            }
                            if (row != null) {
                                clearPushedInvite()
                                app.navigate("/channels/${row.text("channel_id").encodeURLPathPart()}")
                            }
                        }
                        controller.invites(rows, app.config.userId, now,
                            preferredInviteId = target?.takeIf { selection?.answer == true })
                    }
            }
        }
    }
    override suspend fun startAudio(id: String) {
        check(valid(id) && connection?.id == id && audioCallId == null)
        val ready = CompletableDeferred<Unit>()
        audioCallId = id
        audioReady = ready
        context.startForegroundService(Intent(context, PhoneService::class.java)
            .putExtra(PHONE_SESSION, token).putExtra(PHONE_CALL, id))
        withTimeout(10_000) { ready.await() }
    }

    fun audioCall(id: String?) = controller.call?.takeIf { it.id == id && id == audioCallId }
    fun audioStarted(instance: PhoneService, id: String) {
        if (audioCall(id) == null) return
        service = instance
        clearIncomingNotification()
        audioReady?.complete(Unit)
    }

    fun audioFailed(id: String) {
        if (audioCall(id) != null) controller.nativeFailed(id)
    }

    private fun clearPushedInvite() {
        pushedInviteExpiry?.cancel()
        pushedInviteExpiry = null
        pushedInvite.value = null
    }

    private fun openPushedInviteIfReady() {
        val selection = pushedInvite.value?.takeIf { it.answer } ?: return
        if (selection.invite.remainingMillis() <= 0 || pushBinding.dmInviteEnded(selection.invite.id)) {
            clearPushedInvite()
            return
        }
        val call = controller.call ?: return
        if (controller.ringingInviteId != selection.invite.id || connection?.id != call.id) return
        clearPushedInvite()
        app.navigate("/channels/${call.target.encodeURLPathPart()}")
        controller.answer()
    }

    fun open(intent: Intent): Boolean {
        if (intent.action == DM_PUSH_ANSWER_ACTION || intent.action == DM_PUSH_OPEN_ACTION) {
            val invite = DmCallPushAnswer.fromIntent(intent) ?: return true
            val remaining = invite.remainingMillis()
            if (available && pushBinding.matches(invite.binding, app.config.userId) &&
                pushBinding.dmCallsEnabled(app.config.userId) && !pushBinding.dmInviteEnded(invite.id) &&
                remaining > 0
            ) {
                pushedInviteExpiry?.cancel()
                pushedInvite.value = PushedInvite(invite, intent.action == DM_PUSH_ANSWER_ACTION)
                pushedInviteExpiry = app.scope.launch {
                    delay(remaining)
                    pushedInvite.value = null
                    pushedInviteExpiry = null
                }
                controller.restoreDmWaiting()
                openPushedInviteIfReady()
            }
            return true
        }
        if (intent.action != "zoo.phone.OPEN" && intent.action != "zoo.phone.ANSWER") return false
        if (!matches(intent.getStringExtra(PHONE_SESSION), intent.getStringExtra(PHONE_CALL))) return true
        val call = controller.call ?: return true
        if (intent.action == "zoo.phone.ANSWER") controller.answer()
        app.navigate(if (call.kind == "dm") "/channels/${call.target.encodeURLPathPart()}" else "/settings?tab=sip")
        return true
    }
    fun serviceDestroyed(instance: PhoneService) {
        if (service !== instance) return
        service = null
        audioCallId?.let(::audioFailed)
    }
    fun close() {
        if (current !== this) return
        current = null; state.cancel(); invites?.cancel(); inviteTable?.close(); controller.close()
        connection?.setDisconnected(DisconnectCause(DisconnectCause.LOCAL)); connection?.destroy(); connection = null
        runCatching { manager.unregisterPhoneAccount(handle) }
        clearIncomingNotification()
    }
    companion object { var current: AndroidPhoneHost? = null; private set }
}
