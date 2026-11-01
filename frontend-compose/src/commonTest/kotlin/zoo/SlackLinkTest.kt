package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

@OptIn(ExperimentalCoroutinesApi::class)
class SlackLinkTest {
    private val notLinked = obj("configured" to true, "link" to null)
    private val linked =
        obj(
            "configured" to true,
            "link" to obj("team_id" to "T1", "slack_user_id" to "U1", "slack_user_name" to "alice"),
        )

    @Test
    fun readsTheResultSlackRedirectsBackWith() {
        assertEquals(
            SlackLinkResult.Authorized("c", "s"),
            readSlackLinkResult(mapOf("code" to "c", "state" to "s")),
        )
        assertEquals(
            SlackLinkResult.Failed("team_mismatch"),
            readSlackLinkResult(mapOf("slack" to "error", "reason" to "team_mismatch")),
        )
        assertEquals(SlackLinkResult.Failed(""), readSlackLinkResult(mapOf("slack" to "error")))
        assertNull(readSlackLinkResult(mapOf("tab" to "slack")))
        assertEquals("invalid_state", slackLinkErrorReason(400))
        assertEquals("team_mismatch", slackLinkErrorReason(403))
        assertEquals("already_linked", slackLinkErrorReason(409))
        assertEquals("failed", slackLinkErrorReason(502))
    }

    @Test
    fun aLateInitialReadDoesNotOverwriteTheCompletedLink() = runTest {
        val initial = CompletableDeferred<JsonElement>()
        val state =
            SlackLinkController(backgroundScope) { path, _, _ ->
                when (path) {
                    "/api/slack/link" -> initial.await()
                    else -> linked
                }
            }
        state.load()
        runCurrent()
        state.handle(SlackLinkResult.Authorized("c", "s"))
        runCurrent()
        assertEquals("U1", (state.state?.get("link") as JsonObject).text("slack_user_id"))
        assertEquals(1, state.linked)
        initial.complete(notLinked)
        runCurrent()
        assertNotNull(state.state?.get("link") as? JsonObject, "the stale read must be discarded")
        state.close()
    }

    @Test
    fun aFailedCompletionReloadsSoTheUserCanRetry() = runTest {
        val state =
            SlackLinkController(backgroundScope) { path, _, _ ->
                when (path) {
                    "/api/slack/link" -> notLinked
                    else -> throw ApiException(400, obj("error" to "invalid_state"))
                }
            }
        state.handle(SlackLinkResult.Authorized("c", "expired"))
        runCurrent()
        assertEquals("slackLink.errors.invalid_state", state.error)
        assertEquals(notLinked, state.state)
        assertFalse(state.pending)
        state.close()
    }

    @Test
    fun unlinkFailureKeepsTheLinkAndUnknownReasonsFallBack() = runTest {
        var fail = true
        val state =
            SlackLinkController(backgroundScope) { _, method, _ ->
                if (method == "DELETE") {
                    if (fail) throw ApiException(502, obj("error" to "revocation failed"))
                    notLinked
                } else linked
            }
        state.load()
        runCurrent()
        var closed = false
        state.unlink { closed = true }
        runCurrent()
        assertEquals("slackLink.unlinkFailed", state.error)
        assertFalse(closed)
        assertNotNull(state.state?.get("link") as? JsonObject)
        fail = false
        state.unlink { closed = true }
        runCurrent()
        assertTrue(closed)
        assertEquals(notLinked, state.state)
        state.handle(SlackLinkResult.Failed("weird"))
        assertEquals("slackLink.errors.failed", state.error)
        state.close()
    }

    @Test
    fun ownImportStartsAnImportAndShowsServerReasons() = runTest {
        val run = obj("id" to "run", "status" to "importing", "source_id" to "export")
        var imports = listOf<JsonElement>()
        var reject = true
        val requests = mutableListOf<String>()
        val state =
            SlackOwnImportController(backgroundScope) { path, method, _ ->
                requests += "$method $path"
                if (method == "POST") {
                    if (reject)
                        throw ApiException(
                            403,
                            obj("error" to "You can only import Slack channels you created (C2)"),
                        )
                    imports = listOf(run)
                    run
                } else
                    obj(
                        "export" to obj("id" to "export", "file_name" to "export.zip"),
                        "channels" to JsonArray(listOf(obj("slack_channel_id" to "C1"))),
                        "imports" to JsonArray(imports),
                    )
            }
        state.load()
        runCurrent()
        assertFalse(state.running)
        val mappings = listOf(obj("slack_channel_id" to "C1", "action" to "create"))
        state.start(mappings)
        runCurrent()
        // 各ユーザーの取り込みでは、403 も「管理者だけ」ではなくサーバーの理由を出す
        assertEquals(403, state.failure?.status)
        assertEquals("You can only import Slack channels you created (C2)", state.failure?.message)
        assertEquals(0, state.started)
        reject = false
        state.start(mappings)
        runCurrent()
        assertNull(state.failure)
        assertEquals(1, state.started)
        assertEquals("run", state.latest?.text("id"))
        assertTrue(state.running)
        assertEquals("GET /api/slack/imports", requests.last())
        state.close()
    }
}
