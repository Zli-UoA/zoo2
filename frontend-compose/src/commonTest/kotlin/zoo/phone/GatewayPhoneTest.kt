package zoo.phone

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import zoo.*

@OptIn(ExperimentalCoroutinesApi::class)
class GatewayPhoneTest {
    @Test fun aTimedOutOldPostDoesNotFailTheNextAttempt() = runTest {
        lateinit var receive: suspend (String, Row) -> Unit
        val callbacks = mutableListOf<(String, Row) -> Unit>()
        val events = mutableListOf<String>()
        var calls = 0
        val phone = GatewayPhone(backgroundScope, request = { _, _, body ->
            if (body?.text("action") == "call" && ++calls == 1) awaitCancellation()
            obj()
        }, stream = { _, callback -> receive = callback; awaitCancellation() }, media = { callback ->
            callbacks.add(callback)
            object : PhoneMedia {
                override suspend fun command(method: String, payload: Row) {}
                override fun close() {}
            }
        }, events = { event, _ -> events.add(event) })
        phone.command("phone.register", obj("session_id" to "session", "peer_id" to "peer")); runCurrent()
        phone.command("phone.call", obj("number" to "101"))
        callbacks[0]("signal", obj("signal" to obj("type" to "offer", "sdp" to "old"))); runCurrent()
        receive("ended", obj())
        phone.command("phone.call", obj("number" to "102"))
        callbacks[1]("signal", obj("signal" to obj("type" to "offer", "sdp" to "new"))); runCurrent()
        advanceTimeBy(15_001); runCurrent()
        assertFalse("failed" in events)
        callbacks[1]("connected", obj()); receive("accepted", obj())
        assertEquals("connected", events.last())
        phone.close()
    }

    @Test fun resumingAnOldStartDoesNotClearTheNextInvitationsCandidates() = runTest {
        lateinit var receive: suspend (String, Row) -> Unit
        val suspended = CompletableDeferred<Unit>()
        val delivered = mutableListOf<String>()
        var engines = 0
        val phone = GatewayPhone(backgroundScope, request = { _, _, _ -> obj() },
            stream = { _, callback -> receive = callback; awaitCancellation() }, media = {
                val first = ++engines == 1
                object : PhoneMedia {
                    override suspend fun command(method: String, payload: Row) {
                        if (method == "rtc.event") {
                            val type = (payload["data"] as? kotlinx.serialization.json.JsonObject)
                                ?.get("signal") as? kotlinx.serialization.json.JsonObject
                            if (first) suspended.await() else delivered.add(type?.text("type").orEmpty())
                        }
                    }
                    override fun close() {}
                }
            }, events = { _, _ -> })
        phone.command("phone.register", obj("session_id" to "session", "peer_id" to "peer")); runCurrent()
        receive("incoming", obj("signal" to obj("type" to "offer")))
        val old = launch { phone.command("phone.answer", obj()) }; runCurrent()
        receive("ended", obj())
        receive("incoming", obj("signal" to obj("type" to "offer")))
        receive("signal", obj("signal" to obj("type" to "candidate")))
        suspended.complete(Unit); old.join()
        phone.command("phone.answer", obj())
        assertEquals(listOf("offer", "candidate"), delivered)
        phone.close()
    }
}
