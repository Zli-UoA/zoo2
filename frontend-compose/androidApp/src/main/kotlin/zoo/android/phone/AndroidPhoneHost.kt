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
        val address = if (call.kind == "sip") Uri.fromParts("tel", call.peer, null) else Uri.fromParts("sip", "dm-${call.target}@zoo.invalid", null)
        try { manager.placeCall(address, options); withTimeout(10_000) { ready.await() } }
        catch (_: SecurityException) { throw IllegalStateException("Telecom permission unavailable") }
        finally { if (connectionReady === ready) connectionReady = null }
    }
    override fun incoming(call: PhoneCall) { check(available); manager.addNewIncomingCall(handle, extras(call)) }
    fun connection(incoming: Boolean, address: Uri?): Connection {
        val call = controller.call ?: return Connection.createFailedConnection(DisconnectCause(DisconnectCause.CANCELED))
        if (connection != null || incoming != (call.status == "incoming")) {
            return Connection.createFailedConnection(DisconnectCause(DisconnectCause.CANCELED))
        }
        val native = ZooConnection(this, call.id, incoming)
        native.setCallerDisplayName(call.label.ifBlank { "Zoo" }, TelecomManager.PRESENTATION_ALLOWED)
        address?.let { native.setAddress(it, TelecomManager.PRESENTATION_ALLOWED) }
        connection = native; connectionReady?.complete(Unit)
        return native
    }
    fun failed() { connectionReady?.completeExceptionally(IllegalStateException("Telecom rejected the call")); controller.call?.id?.let(controller::nativeFailed) }
    private fun showIncoming(id: String) {
        val call = controller.call?.takeIf { it.id == id && it.status == "incoming" } ?: return
        if (audioCallId == id) return
        try {
            showIncomingCall(context, this, call)
        } catch (_: RuntimeException) {
            controller.nativeFailed(id)
        }
    }
    override fun ended(id: String) {
        connection?.takeIf { it.id == id }?.let { it.setDisconnected(DisconnectCause(DisconnectCause.LOCAL)); it.destroy(); connection = null }
        connectionReady?.cancel(); connectionReady = null; pendingMute = null
        dismissIncomingCall(context)
        if (audioCallId == id) {
            audioCallId = null; audioReady?.cancel(); audioReady = null; service = null
            context.stopService(Intent(context, PhoneService::class.java))
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
        invites?.cancel(); invites = null; inviteTable?.close(); inviteTable = null
        if (enabled) {
            val table = SyncedTable("call_invites", app.config, app.platform, http, app.scope, persist = false)
            inviteTable = table
            invites = app.scope.launch {
                combine(table.synchronizedRows.filterNotNull(), app.repo.channels.replicatedRows, app.repo.users.replicatedRows) { rows, channels, users ->
                    val dmIds = channels.filter { it.flag("is_dm") }.map { it.text("id") }.toSet()
                    rows.filter { it.text("channel_id") in dmIds }.map { row ->
                        val user = users.find { it.text("id") == row.text("inviter_id") }
                        kotlinx.serialization.json.JsonObject(row + ("inviter_name" to kotlinx.serialization.json.JsonPrimitive(user?.text("display_name").orEmpty().ifBlank { "Zoo" })))
                    }
                }.collect { rows -> controller.invites(rows, app.config.userId, kotlin.time.Clock.System.now().toEpochMilliseconds()) }
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
        dismissIncomingCall(context)
        audioReady?.complete(Unit)
    }

    fun audioFailed(id: String) {
        if (audioCall(id) != null) controller.nativeFailed(id)
    }

    fun open(intent: Intent): Boolean {
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
        dismissIncomingCall(context)
        context.stopService(Intent(context, PhoneService::class.java))
    }
    companion object { var current: AndroidPhoneHost? = null; private set }
}
