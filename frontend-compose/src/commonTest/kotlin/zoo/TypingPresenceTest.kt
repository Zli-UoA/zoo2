@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

class TypingPresenceTest {
    private class Server(var typing: List<Row> = emptyList()) {
        val writes = mutableListOf<Boolean>()
        var reads = 0
        val paths = mutableListOf<String>()
        suspend fun request(path: String, method: String, body: JsonElement?): JsonElement {
            paths += "$method $path"
            if (method == "POST") {
                writes += body!!.jsonObject["active"]!!.jsonPrimitive.boolean
                return buildJsonObject { put("ok", true) }
            }
            reads++
            return JsonArray(typing)
        }
    }

    @Test fun typingSendsAtMostEveryTwoSecondsAndStopsAfterThreeQuietSeconds() = runTest {
        val server = Server()
        val presence = TypingPresence(server::request, "session", testTimeSource)
        val typed = MutableSharedFlow<Boolean>()
        val job = backgroundScope.launch { presence.run("channel", null, typed) }
        runCurrent()
        typed.emit(true); runCurrent()
        assertEquals(listOf(true), server.writes)
        advanceTimeBy(1_000); typed.emit(true); runCurrent()
        assertEquals(listOf(true), server.writes, "a second keystroke within 2s does not resend")
        advanceTimeBy(1_500); typed.emit(true); runCurrent()
        assertEquals(listOf(true, true), server.writes)
        advanceTimeBy(3_100); runCurrent()
        assertEquals(listOf(true, true, false), server.writes, "3s without input stops")
        job.cancelAndJoin()
        assertEquals(listOf(true, true, false), server.writes, "nothing more to stop")
    }

    @Test fun clearingTheDraftStopsAtOnce() = runTest {
        val server = Server()
        val presence = TypingPresence(server::request, "session", testTimeSource)
        val typed = MutableSharedFlow<Boolean>()
        backgroundScope.launch { presence.run("channel", "thread", typed) }
        runCurrent()
        typed.emit(true); runCurrent()
        typed.emit(false); runCurrent()
        assertEquals(listOf(true, false), server.writes)
        assertTrue(server.paths.all { it.endsWith("/api/channels/channel/typing") || it.endsWith("?parent_id=thread") })
    }

    @Test fun leavingWhileTypingSendsTheStop() = runTest {
        val server = Server()
        val presence = TypingPresence(server::request, "session", testTimeSource)
        val typed = MutableSharedFlow<Boolean>()
        val job = launch { presence.run("channel", null, typed) }
        runCurrent()
        typed.emit(true); runCurrent()
        job.cancelAndJoin()
        assertEquals(listOf(true, false), server.writes)
        assertEquals(emptyList(), presence.users.value)
    }

    @Test fun pollingShowsOthersAndBacksOffWhenNobodyTypes() = runTest {
        val server = Server(listOf(buildJsonObject { put("id", "u2"); put("display_name", "Hanako") }))
        val presence = TypingPresence(server::request, "session", testTimeSource)
        backgroundScope.launch { presence.run("channel", null, emptyFlow()) }
        runCurrent()
        assertEquals(listOf("Hanako"), TypingPresence.names(presence.users.value) { it })
        advanceTimeBy(1_500); runCurrent()
        assertEquals(2, server.reads, "someone typing keeps the 1.5s interval")
        server.typing = emptyList()
        advanceTimeBy(1_500); runCurrent()
        assertEquals(3, server.reads)
        assertEquals(emptyList(), presence.users.value)
        advanceTimeBy(2_999); runCurrent()
        assertEquals(3, server.reads, "nobody typing doubles the interval")
        advanceTimeBy(1); runCurrent()
        assertEquals(4, server.reads)
        advanceTimeBy(20_000); runCurrent()
        assertTrue(server.reads <= 4 + 20_000 / 5_000 + 1, "the interval stops growing at 5s")
    }

    @Test fun startingToTypeWakesTheIdlePoll() = runTest {
        val server = Server()
        val presence = TypingPresence(server::request, "session", testTimeSource)
        val typed = MutableSharedFlow<Boolean>()
        backgroundScope.launch { presence.run("channel", null, typed) }
        runCurrent()
        advanceTimeBy(20_000); runCurrent()
        val before = server.reads
        typed.emit(true); runCurrent()
        assertEquals(before + 1, server.reads)
    }

    @Test fun namesFallBackToTheUserList() {
        val rows = listOf(buildJsonObject { put("id", "u1"); put("display_name", "") })
        assertEquals(listOf("Taro"), TypingPresence.names(rows) { if (it == "u1") "Taro" else it })
    }
}
