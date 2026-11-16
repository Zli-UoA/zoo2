package zoo.android.phone

import android.telecom.*
import android.telecom.Connection.*
import android.os.Bundle

internal const val PHONE_SESSION = "zoo.phone.session"
internal const val PHONE_CALL = "zoo.phone.call"

/** Telecom binds this service under BIND_TELECOM_CONNECTION_SERVICE; it never accepts credentials. */
class ZooConnectionService : ConnectionService() {
    override fun onCreateOutgoingConnection(account: PhoneAccountHandle?, request: ConnectionRequest): Connection = create(request, false)
    override fun onCreateIncomingConnection(account: PhoneAccountHandle?, request: ConnectionRequest): Connection = create(request, true)
    private fun create(request: ConnectionRequest, incoming: Boolean): Connection {
        val host = AndroidPhoneHost.current
        val extras = request.extras ?: Bundle.EMPTY
        val nested = extras.getBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS) ?: extras
        if (host == null || !host.matches(nested.getString(PHONE_SESSION), nested.getString(PHONE_CALL))) return Connection.createFailedConnection(DisconnectCause(DisconnectCause.ERROR))
        return host.connection(incoming)
    }
    override fun onCreateOutgoingConnectionFailed(account: PhoneAccountHandle?, request: ConnectionRequest) { failed(request) }
    override fun onCreateIncomingConnectionFailed(account: PhoneAccountHandle?, request: ConnectionRequest) { failed(request) }
    private fun failed(request: ConnectionRequest) {
        val extras = request.extras ?: return
        val values = extras.getBundle(TelecomManager.EXTRA_OUTGOING_CALL_EXTRAS) ?: extras
        AndroidPhoneHost.current?.takeIf { it.matches(values.getString(PHONE_SESSION), values.getString(PHONE_CALL)) }?.failed()
    }
}

internal interface ConnectionActions {
    fun valid(id: String): Boolean
    fun answer(id: String)
    fun end(id: String)
    fun tone(id: String, tone: String)
    fun incomingUi(id: String)
    fun audio(id: String, speaker: Boolean, muted: Boolean)
}

internal class ZooConnection(private val actions: ConnectionActions, val id: String, incoming: Boolean) : Connection() {
    init {
        connectionProperties = PROPERTY_SELF_MANAGED
        connectionCapabilities = CAPABILITY_MUTE
        audioModeIsVoip = true
        if (incoming) setRinging() else { setDialing(); setRingbackRequested(true) }
    }
    override fun onAnswer() { if (actions.valid(id)) actions.answer(id) }
    override fun onAnswer(videoState: Int) = onAnswer()
    override fun onReject() { if (actions.valid(id)) actions.end(id) }
    override fun onDisconnect() = onReject()
    override fun onAbort() = onReject()
    override fun onPlayDtmfTone(c: Char) { if (actions.valid(id)) actions.tone(id, c.toString()) }
    override fun onShowIncomingCallUi() { if (actions.valid(id)) actions.incomingUi(id) }
    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun onCallAudioStateChanged(state: CallAudioState) {
        if (actions.valid(id)) actions.audio(id, state.route == CallAudioState.ROUTE_SPEAKER, state.isMuted)
    }
}
