package zoo.phone

import androidx.compose.runtime.*
import io.ktor.http.encodeURLPathPart
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import kotlin.time.TimeMark
import kotlin.time.TimeSource
import zoo.*

/** No credentials are stored in UI state, saved state, notification extras or caches. */
data class PhoneCall(val id: String, val kind: String, val target: String, val label: String, val status: String, val peer: String = "", val muted: Boolean = false, val speaker: Boolean = false, val peerUserId: String? = null)
data class SipRegistration(val lineId: String? = null, val status: String = "offline")
interface PhoneMedia {
    suspend fun command(method: String, payload: Row = obj())
    fun close()
}
interface PhoneTelecom {
    suspend fun outgoing(call: PhoneCall)
    fun incoming(call: PhoneCall)
    suspend fun startAudio(id: String)
    fun ended(id: String)
    fun speaker(id: String, enabled: Boolean)
}
private val SFU_OPS = setOf("join", "cf/publish", "cf/pull", "cf/answer")
class PhoneController(
    parent: CoroutineScope,
    private val request: suspend (String, String, Row?) -> JsonElement,
    private val stream: suspend (String, String, suspend (String, Row) -> Unit) -> Unit,
    private val media: (String, (String, Row) -> Unit) -> PhoneMedia,
    private val phoneStream: suspend (String, suspend (String, Row) -> Unit) -> Unit = { _, _ -> error("Phone stream unavailable") },
    private val telecom: PhoneTelecom,
    private val prepare: suspend () -> Unit,
    private val waiting: (Boolean) -> Unit,
    private val uuid: () -> String,
    private val heartbeatClock: TimeSource = TimeSource.Monotonic,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    var call by mutableStateOf<PhoneCall?>(null); private set
    var remoteAudioLevel by mutableFloatStateOf(0f); private set
    var registration by mutableStateOf(SipRegistration()); private set
    var lines by mutableStateOf<List<Row>>(emptyList()); private set
    var enabled by mutableStateOf(false); private set
    var loading by mutableStateOf(false); private set
    var loaded by mutableStateOf(false); private set
    var loadError by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    var sipConnectionError by mutableStateOf<String?>(null); private set
    var dmWaitingError by mutableStateOf(false); private set
    var writeError by mutableStateOf(false); private set
    var writing by mutableStateOf(false); private set
    var busy by mutableStateOf(false); private set
    var dmWaiting by mutableStateOf(false); private set
    private var read: Job? = null
    private var readAgain = false
    private var sipEpoch = 0
    private var callEpoch = 0
    private var sip: PhoneMedia? = null
    private var dm: PhoneMedia? = null
    private var sipRevision = ""
    private var sipJob: Job? = null
    private var callJob: Job? = null
    private var heartbeat: Job? = null
    private var sessionId: String? = null
    private var invite: Row? = null
    private var answeringInvite: String? = null
    private var setupExpiry: Job? = null
    private var inviteExpiry: Job? = null
    private val seenInvites = linkedSetOf<String>()
    private fun data(value: JsonElement) = value.jsonObject["data"]?.jsonObject ?: error("Invalid phone response")
    private suspend fun api(path: String, method: String = "GET", body: Row? = null) = withTimeout(15_000) { request(path, method, body) }
    private fun path(id: String) = "/api/sip/lines/${id.encodeURLPathPart()}"

    fun refresh(force: Boolean = false) {
        if (!lifetime.isActive) return
        if (read?.isActive == true) { if (force) readAgain = true; return }
        loading = true; loadError = false
        read = scope.launch {
            try {
                do {
                    readAgain = false
                    val result = api("/api/sip/lines").jsonObject
                    ensureActive()
                    enabled = result.flag("enabled"); lines = result["lines"]?.rows().orEmpty(); loaded = true
                    registration.lineId?.let { id -> if (lines.none { it.text("id") == id && it.flag("can_connect") }) disconnectSip() }
                } while (readAgain)
            } catch (_: TimeoutCancellationException) { loadError = true }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { loadError = true }
            finally { loading = false; read = null }
        }
    }
    fun save(id: String?, body: Row, done: () -> Unit) {
        if (writing || !lifetime.isActive) return
        writing = true; writeError = false
        scope.launch {
            try {
                api(id?.let(::path) ?: "/api/sip/lines", "POST", body); ensureActive()
                if (registration.lineId == id) disconnectSip()
                done(); refresh(true)
            } catch (_: TimeoutCancellationException) { writeError = true }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { writeError = true }
            finally { writing = false }
        }
    }
    fun remove(id: String, done: () -> Unit) {
        if (writing || !lifetime.isActive) return
        writing = true; writeError = false
        scope.launch {
            try { api(path(id), "DELETE"); ensureActive(); if (registration.lineId == id) disconnectSip(); done(); refresh(true) }
            catch (_: TimeoutCancellationException) { writeError = true }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { writeError = true }
            finally { writing = false }
        }
    }
    fun connectSip(id: String) {
        if (!lifetime.isActive || registration.status != "offline" || lines.none { it.text("id") == id && it.flag("can_connect") }) return
        sipConnectionError = null; registration = SipRegistration(id, "connecting")
        val generation = ++sipEpoch
        sipJob = scope.launch {
            try {
                prepare(); ensureActive()
                val credentials = data(api("${path(id)}/connect", "POST"))
                if (generation != sipEpoch) return@launch
                val ready = CompletableDeferred<Unit>()
                val engine = GatewayPhone(scope, request = { path, method, body -> api(path, method, body) }, stream = phoneStream, media = { events -> media("sip", events) }) { event, payload ->
                    if (generation == sipEpoch && lifetime.isActive) when (event) {
                        "registered" -> { registration = SipRegistration(id, "ready"); ready.complete(Unit) }
                        "incoming" -> incomingSip(id)
                        "ending" -> { registration = SipRegistration(id, "connecting") }
                        "connected" -> if (call?.kind == "sip") call = call?.copy(status = "active")
                        "remote_audio_level" -> updateRemoteAudio("sip", payload)
                        "ended" -> if (call?.kind == "sip") finishCall()
                        "failed" -> sipConnectionFailed(id)
                    }
                }
                sip = engine; sipRevision = credentials.text("revision")
                withTimeout(15_000) { engine.command("phone.register", credentials); ready.await() }
            } catch (timeout: TimeoutCancellationException) { if (generation == sipEpoch) sipConnectionFailed(id) }
            catch (cancel: CancellationException) { if (generation == sipEpoch) disconnectSip(); throw cancel }
            catch (_: Exception) { if (generation == sipEpoch) sipConnectionFailed(id) }
        }
    }
    private fun sipConnectionFailed(id: String) {
        val wasInCall = call?.kind == "sip"
        disconnectSip()
        if (wasInCall) error = true else sipConnectionError = id
    }

    fun disconnectSip() {
        sipEpoch++; sipJob?.cancel(); sipJob = null
        if (call?.kind == "sip") finishCall()
        val previous = sip; sip = null; previous?.close(); sipRevision = ""; registration = SipRegistration()
    }
    private suspend fun authorizeSip() {
        val id = registration.lineId ?: error("Not registered")
        val current = data(api("${path(id)}/authorize", "POST"))
        if (current.text("revision") != sipRevision) error("Line changed")
    }
    fun dialSip(number: String) {
        if (!Regex("\\+?[0-9]{1,32}").matches(number) || registration.status != "ready" || call != null || busy) return
        val line = lines.find { it.text("id") == registration.lineId } ?: return
        val next = PhoneCall(uuid(), "sip", line.text("id"), line.text("name"), "dialing", number)
        call = next; val epoch = ++callEpoch
        operation(epoch) {
            authorizeSip(); if (epoch != callEpoch) return@operation
            telecom.outgoing(next); telecom.startAudio(next.id)
            if (epoch == callEpoch) sip?.command("phone.call", obj("number" to number))
        }
    }
    private fun incomingSip(id: String) {
        if (call != null) { scope.launch { runCatching { sip?.command("phone.end") } }; return }
        val line = lines.find { it.text("id") == id }
        val next = PhoneCall(uuid(), "sip", id, line?.text("name").orEmpty(), "incoming")
        ++callEpoch; call = next
        try { telecom.incoming(next) } catch (_: Exception) { hangup(); error = true }
    }
    fun receiveDmCalls(value: Boolean) {
        if (!lifetime.isActive || value == dmWaiting || busy) return
        dmWaitingError = false
        scope.launch {
            try { if (value) prepare(); ensureActive(); dmWaiting = value; waiting(value) }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { dmWaiting = false; runCatching { waiting(false) }; dmWaitingError = true }
        }
    }
    fun restoreDmWaiting() {
        if (!lifetime.isActive || dmWaiting) return
        dmWaiting = true
        waiting(true)
    }

    val ringingInviteId: String? get() = invite?.text("id")

    /** Only the session-wide network subscription supplies committed, private invite rows. */
    fun invites(rows: List<Row>, userId: String, now: Long, preferredInviteId: String? = null) {
        if (!dmWaiting || !lifetime.isActive) return
        val current = invite
        if (current != null && call?.status == "incoming" && answeringInvite != current.text("id") && rows.none { it.text("id") == current.text("id") && it.text("status") == "pending" }) finishCall()
        if (preferredInviteId != null && call != null) return
        val pending = rows.firstOrNull { (preferredInviteId == null || it.text("id") == preferredInviteId) && it.text("invitee_id") == userId && it.text("status") == "pending" && it.text("id") !in seenInvites && (timestampMillis(it.text("expires_at")) ?: 0L) > now } ?: return
        seenInvites.add(pending.text("id")); if (seenInvites.size > 1000) seenInvites.remove(seenInvites.first())
        if (call != null) { scope.launch { runCatching { api("/api/call-invites/${pending.text("id").encodeURLPathPart()}/decline", "POST") } }; return }
        val next = PhoneCall(uuid(), "dm", pending.text("channel_id"), pending.text("inviter_name").ifBlank { "Zoo" }, "incoming",
            peerUserId = pending.text("inviter_id").ifBlank { null })
        ++callEpoch; call = next; invite = pending
        try { telecom.incoming(next) } catch (_: Exception) { hangup(); error = true }
        val epoch = callEpoch
        inviteExpiry = scope.launch { delay(((timestampMillis(pending.text("expires_at")) ?: now) - now).coerceIn(0, 30_000)); if (epoch == callEpoch && call?.status == "incoming" && answeringInvite == null) finishCall() }
    }
    fun callDm(channelId: String, label: String, peerUserId: String? = null) {
        if (!lifetime.isActive || call != null || busy) return
        val next = PhoneCall(uuid(), "dm", channelId, label, "dialing", peerUserId = peerUserId)
        call = next; val epoch = ++callEpoch
        operation(epoch) {
            prepare(); if (epoch != callEpoch) return@operation
            telecom.outgoing(next); telecom.startAudio(next.id)
            if (epoch == callEpoch) joinDm(next, epoch)
        }
    }
    fun answer() {
        val next = call?.takeIf { it.status == "incoming" } ?: return
        if (busy) return
        if (next.kind == "dm") answeringInvite = invite?.text("id")
        val epoch = callEpoch
        operation(epoch) {
            prepare()
            if (epoch != callEpoch) return@operation
            if (next.kind == "sip") authorizeSip()
            if (epoch != callEpoch) return@operation
            telecom.startAudio(next.id)
            if (epoch != callEpoch) return@operation
            if (next.kind == "sip") {
                call = call?.copy(status = "connecting")
                sip?.command("phone.answer")
            } else {
                val pending = invite ?: error("Invite ended")
                val accepted = api("/api/call-invites/${pending.text("id").encodeURLPathPart()}/accept", "POST").jsonObject
                if (!accepted.flag("ok") || accepted.text("channel_id") != next.target) error("Invalid invitation")
                if (epoch == callEpoch) {
                    inviteExpiry?.cancel(); invite = null; answeringInvite = null
                    call = call?.copy(status = "connecting"); joinDm(next, epoch)
                }
            }
        }
    }
    private fun joinDm(next: PhoneCall, epoch: Int) {
        val session = uuid(); sessionId = session
        setupExpiry = scope.launch { delay(45_000); if (epoch == callEpoch && call?.status != "active") hangup() }
        callJob = scope.launch {
            val early = mutableListOf<Pair<String, Row>>()
            try {
                stream(next.target, session) { event, value ->
                    if (epoch != callEpoch) return@stream
                    if (event != "room_state" && dm == null) {
                        check(early.size < 128) { "Too many early call events" }
                        early.add(event to value); return@stream
                    }
                    if (event == "room_state" && dm == null) {
                        val presenceStarted = heartbeatClock.markNow()
                        val engine = media("dm") { type, payload ->
                            if (epoch == callEpoch) when (type) {
                                "connected" -> { setupExpiry?.cancel(); call = call?.copy(status = "active") }
                                "remote_audio_level" -> updateRemoteAudio("dm", payload)
                                "ended" -> hangup()
                                "failed" -> { hangup(); error = true }
                                "signal" -> signal(next.target, session, epoch, payload)
                                "sfu_request" -> sfuRequest(next.target, session, epoch, payload)
                            }
                        }
                        dm = engine
                        engine.command("rtc.start", obj("session_id" to session, "ice_servers" to (value["ice_servers"] ?: JsonArray(emptyList())),
                            "peer_user_id" to next.peerUserId))
                        heartbeat = scope.launch { maintainDmPresence(next.target, session, epoch, presenceStarted) }
                    }
                    dm?.command("rtc.event", obj("event" to event, "data" to value))
                    for ((name, pending) in early) dm?.command("rtc.event", obj("event" to name, "data" to pending))
                    early.clear()
                }
                if (epoch == callEpoch) { hangup(); error = true }
            } catch (timeout: TimeoutCancellationException) { if (epoch == callEpoch) { hangup(); error = true } }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { if (epoch == callEpoch) { hangup(); error = true } }
        }
    }
    private suspend fun maintainDmPresence(channel: String, session: String, epoch: Int, presenceStarted: TimeMark) {
        val path = "/api/channels/${channel.encodeURLPathPart()}/call/heartbeat"
        val body = obj("session_id" to session)
        var lastRenewal = presenceStarted
        try {
            while (epoch == callEpoch) {
                val elapsed = lastRenewal.elapsedNow().inWholeMilliseconds
                // Count response latency against the 30-second lease and reserve one second for delivery.
                withTimeout(29_000 - elapsed) {
                    delay(10_000 - elapsed)
                    for (attempt in 1..3) {
                        val sent = heartbeatClock.markNow()
                        try {
                            withTimeout(if (attempt == 1) 15_000 else 5_000) { request(path, "POST", body) }
                            lastRenewal = sent
                            break
                        } catch (timeout: TimeoutCancellationException) {
                            if (attempt == 3) throw timeout
                        } catch (cancel: CancellationException) {
                            throw cancel
                        } catch (failure: Exception) {
                            val status = (failure as? ApiException)?.status
                            val permanent = status != null && status < 500 && status != 408 && status != 429
                            if (permanent || attempt == 3) throw failure
                        }
                        delay(1_000)
                    }
                }
            }
        } catch (_: TimeoutCancellationException) {
            if (epoch == callEpoch) { hangup(); error = true }
        } catch (cancel: CancellationException) {
            throw cancel
        } catch (_: Exception) {
            if (epoch == callEpoch) { hangup(); error = true }
        }
    }
    private fun signal(channel: String, session: String, epoch: Int, payload: Row) {
        scope.launch {
            try {
                var attempt = 0
                while (epoch == callEpoch) {
                    try { api("/api/channels/${channel.encodeURLPathPart()}/call/signal", "POST", obj("session_id" to session, "target_session_id" to payload.text("target_session_id"), "signal" to payload["signal"])); return@launch }
                    catch (timeout: TimeoutCancellationException) { if (++attempt >= 3) throw timeout; delay(200L * attempt) }
                    catch (cancel: CancellationException) { throw cancel }
                    catch (failure: Exception) { if (++attempt >= 3 || failure is ApiException && failure.status < 500) throw failure; delay(200L * attempt) }
                }
            } catch (timeout: TimeoutCancellationException) { if (epoch == callEpoch) { hangup(); error = true } }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { if (epoch == callEpoch) { hangup(); error = true } }
        }
    }
    /** The WebView engine cannot reach the API itself; only these SFU calls are relayed (docs/calls-sfu.md). */
    private fun sfuRequest(channel: String, session: String, epoch: Int, payload: Row) {
        val id = payload["id"] as? JsonPrimitive ?: return
        val op = payload.text("op")
        scope.launch {
            val reply = try {
                check(op in SFU_OPS) { "Unknown SFU request" }
                val body = buildJsonObject {
                    (payload["body"] as? JsonObject)?.forEach { (key, value) -> if (key != "session_id") put(key, value) }
                    put("session_id", session)
                }
                obj("id" to id, "ok" to true, "data" to api("/api/channels/${channel.encodeURLPathPart()}/call/sfu/$op", "POST", body))
            }
            // api() の 15 秒の制限は CancellationException の派生なので、先に失敗の返信にする（エンジンの待機を終わらせる）。
            catch (_: TimeoutCancellationException) { obj("id" to id, "ok" to false) }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { obj("id" to id, "ok" to false) }
            if (epoch == callEpoch) runCatching { dm?.command("rtc.sfu.reply", reply) }
        }
    }
    private fun operation(epoch: Int, fatal: Boolean = true, block: suspend () -> Unit) {
        if (busy || !lifetime.isActive) return
        busy = true; error = false
        scope.launch {
            try { if (epoch == callEpoch) block() }
            catch (timeout: TimeoutCancellationException) { if (epoch == callEpoch) { if (fatal) hangup(); error = true } }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { if (epoch == callEpoch) { val sipCall = call?.kind == "sip"; if (fatal) { hangup(); if (sipCall) disconnectSip() }; error = true } }
            finally { if (epoch == callEpoch) busy = false }
        }
    }
    fun hangup() {
        val previous = call ?: return
        val pending = invite; val session = sessionId
        if (previous.kind == "sip") { val engine = sip; scope.launch { runCatching { engine?.command("phone.end") } } }
        else scope.launch {
            runCatching {
                if (pending != null) api("/api/call-invites/${pending.text("id").encodeURLPathPart()}/decline", "POST")
                else if (session != null) api("/api/channels/${previous.target.encodeURLPathPart()}/call/leave", "POST", obj("session_id" to session))
            }
        }
        finishCall()
    }
    private fun updateRemoteAudio(kind: String, payload: Row) {
        if (call?.let { it.kind == kind && it.status == "active" } != true) return
        val value = (payload["level"] as? JsonPrimitive)?.floatOrNull ?: return
        if (value.isFinite()) remoteAudioLevel = value.coerceIn(0f, 1f)
    }
    private fun finishCall() {
        val previous = call
        remoteAudioLevel = 0f
        ++callEpoch; inviteExpiry?.cancel(); inviteExpiry = null; invite = null; answeringInvite = null; setupExpiry?.cancel(); setupExpiry = null
        callJob?.cancel(); callJob = null; heartbeat?.cancel(); heartbeat = null
        dm?.close(); dm = null; sessionId = null; call = null; busy = false
        if (previous != null) telecom.ended(previous.id)
    }
    fun mute(value: Boolean = !(call?.muted ?: false)) {
        val current = call?.takeIf { it.status == "active" } ?: return
        if (current.muted == value || busy) return
        val epoch = callEpoch
        operation(epoch, fatal = false) {
            (if (current.kind == "sip") sip else dm)?.command("mute", obj("muted" to value))
            if (epoch == callEpoch) call = call?.copy(muted = value)
            if (current.kind == "dm") api("/api/channels/${current.target.encodeURLPathPart()}/call/state", "POST", obj("session_id" to sessionId, "is_muted" to value))
        }
    }
    fun speaker() { val current = call ?: return; telecom.speaker(current.id, !current.speaker) }
    fun audioRoute(speaker: Boolean) { call = call?.copy(speaker = speaker) }
    fun dtmf(tone: String) { if (call?.kind != "sip" || call?.status != "active" || !Regex("[0-9*#]").matches(tone)) return; operation(callEpoch, fatal = false) { sip?.command("phone.dtmf", obj("tone" to tone)) } }
    fun clearError() { error = false }
    fun nativeFailed(id: String) { if (call?.id == id) { hangup(); error = true } }
    fun stop() { hangup(); disconnectSip(); dmWaiting = false; waiting(false) }
    fun close() { stop(); read?.cancel(); lifetime.cancel(); lines = emptyList(); loaded = false }
}
