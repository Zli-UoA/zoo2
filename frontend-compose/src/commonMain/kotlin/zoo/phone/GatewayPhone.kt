package zoo.phone

import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import io.ktor.http.encodeURLPathPart
import zoo.*

/** Authenticated gateway control; the WebView receives only WebRTC SDP/ICE. */
internal class GatewayPhone(
    private val parent: CoroutineScope,
    private val request: suspend (String, String, Row?) -> JsonElement,
    private val stream: suspend (String, suspend (String, Row) -> Unit) -> Unit,
    private val media: ((String, Row) -> Unit) -> PhoneMedia,
    private val events: (String, Row) -> Unit,
) : PhoneMedia {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private var connection: Row? = null
    private var subscription: Job? = null
    private var queue: Job? = null
    private var closed = false
    private var offer: Row? = null
    private val early = mutableListOf<Row>()
    private class Attempt(val number: String, val incoming: Row?) {
        var engine: PhoneMedia? = null
        var submitted = false
        var descriptionSent = false
        var accepted = false
        var connected = false
        var notified = false
        val candidates = mutableListOf<Row>()
    }
    private var current: Attempt? = null
    private var ending: CompletableDeferred<Unit>? = null
    private fun path() = "/api/phone/sessions/${connection?.text("session_id")?.encodeURLPathPart() ?: error("Phone not registered")}/command"
    private suspend fun post(body: Row) { withTimeout(15_000) { request(path(), "POST", body) } }
    private fun active(attempt: Attempt) {
        if (current === attempt && attempt.accepted && attempt.connected && !attempt.notified && !closed) {
            attempt.notified = true; events("connected", obj())
        }
    }
    private fun enqueue(block: suspend () -> Unit): Job {
        val previous = queue
        val job = scope.launch {
            try { previous?.join(); if (!closed) block() }
            catch (timeout: TimeoutCancellationException) { if (!closed) events("failed", obj()) }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { if (!closed) events("failed", obj()) }
        }
        queue = job; return job
    }
    private fun signal(attempt: Attempt, payload: Row) {
        val signal = payload["signal"] as? JsonObject ?: return
        when (signal.text("type")) {
            "offer", "answer" -> enqueue {
                if (current !== attempt) return@enqueue
                attempt.submitted = true
                post(obj("action" to if (signal.text("type") == "offer") "call" else "accept", "number" to attempt.number.takeIf { it.isNotEmpty() }, "signal" to signal))
                attempt.descriptionSent = true
                for (candidate in attempt.candidates.toList()) {
                    if (current !== attempt) break
                    post(obj("action" to "candidate", "signal" to candidate))
                }
                attempt.candidates.clear()
            }
            "candidate" -> {
                if (!attempt.descriptionSent) {
                    check(attempt.candidates.size < 128) { "Too many phone candidates" }
                    attempt.candidates.add(signal)
                } else enqueue { if (current === attempt) post(obj("action" to "candidate", "signal" to signal)) }
            }
        }
    }
    private suspend fun received(event: String, payload: Row) {
        if (closed) return
        when (event) {
            "registered" -> events("registered", obj())
            "incoming" -> {
                if (current == null && offer == null) {
                    offer = payload["signal"] as? JsonObject ?: error("Invalid phone invitation")
                    events("incoming", obj())
                }
            }
            "signal" -> {
                val attempt = current
                if (attempt?.engine != null) attempt.engine?.command("rtc.event", obj("event" to "signal", "data" to payload))
                else if (offer != null) { check(early.size < 128); early.add(payload) }
            }
            "accepted" -> current?.let { it.accepted = true; offer = null; active(it) }
            "ended" -> {
                current?.engine?.close(); current = null; offer = null; early.clear()
                ending?.complete(Unit); ending = null
                events("ended", obj()); events("registered", obj())
            }
            "disconnected" -> events("failed", obj())
        }
    }
    private suspend fun start(number: String) {
        check(!closed && current == null && ending == null)
        val info = connection ?: error("Phone not registered")
        val attempt = Attempt(number, offer); current = attempt
        val engine = media { event, payload ->
            if (current === attempt && !closed) when (event) {
                "signal" -> signal(attempt, payload)
                "connected" -> { attempt.connected = true; active(attempt) }
                "failed" -> events("failed", obj())
            }
        }
        attempt.engine = engine
        try {
            engine.command("rtc.start", obj("session_id" to info.text("session_id"), "ice_servers" to info["ice_servers"], "relay" to (info["relay_only"]?.jsonPrimitive?.booleanOrNull != false),
                "room_state" to obj("participants" to JsonArray(listOf(obj("session_id" to info.text("peer_id")))), "offerer_session_id" to if (number.isNotEmpty()) info.text("session_id") else info.text("peer_id"))))
            if (current !== attempt || closed) return
            attempt.incoming?.let { engine.command("rtc.event", obj("event" to "signal", "data" to obj("from_session_id" to info.text("peer_id"), "signal" to it))) }
            for (pending in early.toList()) {
                if (current !== attempt) break
                engine.command("rtc.event", obj("event" to "signal", "data" to pending))
            }
            early.clear()
        } catch (failure: Exception) { if (current === attempt) { current = null; engine.close() }; throw failure }
    }
    override suspend fun command(method: String, payload: Row) {
        check(!closed)
        when (method) {
            "phone.register" -> {
                check(connection == null && payload.text("session_id").isNotEmpty() && payload.text("peer_id").isNotEmpty())
                connection = payload
                subscription = scope.launch {
                    try {
                        stream(payload.text("session_id"), ::received)
                        if (!closed) events("failed", obj())
                    } catch (timeout: TimeoutCancellationException) { if (!closed) events("failed", obj()) }
                    catch (cancel: CancellationException) { throw cancel }
                    catch (_: Exception) { if (!closed) events("failed", obj()) }
                }
            }
            "phone.call" -> start(payload.text("number"))
            "phone.answer" -> { check(offer != null); start("") }
            "phone.end" -> {
                val attempt = current; val pending = offer
                if (attempt == null && pending == null) return
                events("ending", obj())
                current = null; offer = null; early.clear(); attempt?.engine?.close()
                val done = CompletableDeferred<Unit>(); ending = done
                enqueue {
                    if (attempt?.submitted == true) post(obj("action" to "hangup"))
                    else if (pending != null) post(obj("action" to "decline"))
                    else { ending = null; done.complete(Unit); events("ended", obj()); events("registered", obj()) }
                }.join()
                try { withTimeout(5_000) { done.await() } }
                catch (timeout: TimeoutCancellationException) { if (!closed) events("failed", obj()); throw timeout }
            }
            "phone.dtmf" -> enqueue { post(obj("action" to "dtmf", "tone" to payload.text("tone"))) }.join()
            "mute" -> current?.engine?.command("mute", payload)
            else -> error("Unknown phone operation")
        }
    }
    override fun close() {
        if (closed) return
        closed = true; subscription?.cancel(); current?.engine?.close(); current = null; offer = null; early.clear()
        ending?.cancel(); ending = null
        val id = connection?.text("session_id")
        if (id != null) parent.launch { runCatching { withTimeout(5_000) { request("/api/phone/sessions/${id.encodeURLPathPart()}/command", "POST", obj("action" to "disconnect")) } } }
        connection = null; lifetime.cancel()
    }
}
