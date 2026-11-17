package zoo.android

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import zoo.*

@OptIn(ExperimentalCoroutinesApi::class)
class NotificationActionsTest {
    private class Fixture {
        val shown = mutableMapOf("notice" to obj("id" to "notice", "channel_id" to "channel"))
        val calls = mutableListOf<Triple<String, String, Row>>()
        val errors = mutableListOf<String>()
        private var sequence = 0
        var response: suspend (String, String, Row) -> Unit = { _, _, _ -> }
        val actions = NotificationActions(shown,
            request = { path, method, body -> calls += Triple(path, method, body); response(path, method, body) },
            uuid = { "reply-${++sequence}" }, canReply = { true }, replyParent = { "root" },
            showError = { _, key -> errors += key }, dismiss = { shown.remove(it) })
    }

    @Test fun successfulReplyDoesNotAcceptAnotherReplyAfterReadFailure() = runTest {
        val f = Fixture()
        f.response = { _, method, _ -> if (method == "PATCH") error("read failed") }
        f.actions.receive("notice", "reply", "reply A")
        assertTrue(f.actions.hasReplied("notice"), "The notification must hide its reply action")
        assertEquals(listOf("notifications.readError"), f.errors)
        f.actions.receive("notice", "reply", "reply B from stale PendingIntent")
        assertEquals(2, f.calls.size, "A stale action must neither resend nor mark B as sent")
        assertTrue("notice" in f.shown)
        f.response = { _, _, _ -> }
        f.actions.receive("notice", "read", null)
        assertTrue(f.shown.isEmpty())
        assertEquals(1, f.calls.count { it.second == "POST" })
    }

    @Test fun asyncFailuresDoNotReviveDismissedOrReadNotifications() = runTest {
        for (action in listOf("reply", "read")) {
            val f = Fixture()
            val gate = CompletableDeferred<Unit>()
            f.response = { _, _, _ -> gate.await(); error("network failed") }
            val job = launch { f.actions.receive("notice", action, "text") }
            runCurrent()
            f.shown.clear() // Synchronization has marked it read, or the user dismissed it.
            gate.complete(Unit)
            job.join()
            assertTrue(f.errors.isEmpty())
        }
    }

    @Test fun timeoutUpdatesTheNotificationAndReleasesTheBusyState() = runTest {
        val f = Fixture()
        f.response = { _, _, _ -> awaitCancellation() }
        f.actions.receive("notice", "reply", "text", timeoutMillis = 10)
        assertEquals(listOf("common.writeError"), f.errors)
        f.response = { _, _, _ -> }
        f.actions.receive("notice", "reply", "text")
        assertTrue(f.shown.isEmpty())
        assertEquals(listOf("reply-1", "reply-1"), f.calls.filter { it.second == "POST" }.map { it.third.text("id") })
    }

    @Test fun uncertainRepliesKeepIdsPerBodyWithoutLosingANewlyEnteredReply() = runTest {
        val f = Fixture()
        val saved = mutableMapOf<String, String>()
        var loseResponse = true
        f.response = { _, method, body ->
            if (method == "POST") {
                val id = body.text("id")
                val content = body.text("content")
                val existing = saved[id]
                // Plain-channel API: matching retries return the saved row; changed bodies conflict.
                if (existing != null && existing != content) throw ApiException(409)
                saved.putIfAbsent(id, content)
                if (loseResponse) awaitCancellation() // Saved by the server, but no response received.
            }
        }
        for (body in listOf("reply A", "reply B", "reply A")) {
            f.actions.receive("notice", "reply", body, timeoutMillis = 10)
            assertFalse(f.actions.hasReplied("notice"))
            assertTrue("notice" in f.shown)
        }
        loseResponse = false
        f.actions.receive("notice", "reply", "reply B")
        assertEquals(mapOf("reply-1" to "reply A", "reply-2" to "reply B"), saved)
        assertEquals(listOf("reply-1", "reply-2", "reply-1", "reply-2"),
            f.calls.filter { it.second == "POST" }.map { it.third.text("id") })
        assertTrue(f.actions.hasReplied("notice"))
        assertTrue(f.shown.isEmpty())
        assertEquals(List(3) { "common.writeError" }, f.errors)
    }

    @Test fun timeoutAfterSendingOnlyOffersReadRetry() = runTest {
        val f = Fixture()
        f.response = { _, method, _ -> if (method == "PATCH") awaitCancellation() }
        f.actions.receive("notice", "reply", "text", timeoutMillis = 10)
        assertTrue(f.actions.hasReplied("notice"))
        assertEquals(listOf("notifications.readError"), f.errors)
    }

    @Test fun sessionCancellationPropagatesWithoutPostingAnError() = runTest {
        val f = Fixture()
        f.response = { _, _, _ -> awaitCancellation() }
        val job = launch { f.actions.receive("notice", "reply", "text") }
        runCurrent()
        job.cancelAndJoin()
        assertTrue(job.isCancelled)
        assertTrue(f.errors.isEmpty())
    }

    @Test fun timeoutDoesNotReviveANotificationRemovedWhileWaiting() = runTest {
        val f = Fixture()
        f.response = { _, _, _ -> awaitCancellation() }
        val job = launch { f.actions.receive("notice", "reply", "text", timeoutMillis = 10) }
        runCurrent()
        f.shown.clear()
        job.join()
        assertTrue(f.errors.isEmpty())
    }
}
