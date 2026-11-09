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
        assertEquals(
            "linked_other_account",
            slackLinkErrorReason(409, obj("error" to "linked_other_account")),
        )
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

    @Test
    fun aLinkToAWorkspaceThatIsNoLongerAllowedCannotImport() {
        assertTrue(slackLinkTeamAllowed(obj("team_id" to "T1")))
        assertTrue(slackLinkTeamAllowed(obj("team_id" to "T1", "team_allowed" to true)))
        assertFalse(slackLinkTeamAllowed(obj("team_id" to "T0", "team_allowed" to false)))
    }

    @Test
    fun aHiddenPreviousTargetCountsAsImported() {
        assertFalse(slackImportImported(obj("slack_channel_id" to "C1", "existing_channel" to null)))
        assertTrue(
            slackImportImported(
                obj("slack_channel_id" to "C1", "existing_channel" to obj("id" to "c", "name" to "general"))
            )
        )
        assertTrue(
            slackImportImported(
                obj("slack_channel_id" to "G1", "existing_channel" to null, "existing_hidden" to true)
            )
        )
    }

    @Test
    fun pollingWaitsForTheSlowReadInsteadOfDiscardingIt() = runTest {
        val responses = mutableListOf<CompletableDeferred<JsonElement>>()
        val state =
            SlackOwnImportController(backgroundScope) { _, _, _ ->
                CompletableDeferred<JsonElement>().also { responses += it }.await()
            }
        state.load()
        runCurrent()
        assertTrue(state.loading)
        // 応答が遅くても、読み込み中は次の読み込みを始めない (前の応答を捨てない)
        state.poll()
        runCurrent()
        assertEquals(1, responses.size)
        val value =
            obj(
                "export" to null,
                "channels" to JsonArray(emptyList()),
                "imports" to JsonArray(listOf(obj("id" to "run", "status" to "importing"))),
            )
        responses.single().complete(value)
        runCurrent()
        assertFalse(state.loading)
        assertEquals("run", state.latest?.text("id"))
        state.poll()
        runCurrent()
        assertEquals(2, responses.size)
        state.close()
    }

    @Test
    fun returningFromTheExternalBrowserReloadsTheLinkAndAllowsRetry() = runTest {
        var linkedNow = false
        val state =
            SlackLinkController(backgroundScope) { path, method, _ ->
                when {
                    path == "/api/slack/oauth/start" ->
                        obj("authorize_url" to "https://slack.com/oauth/v2/authorize?state=x")
                    method == "GET" -> if (linkedNow) linked else notLinked
                    else -> error("unexpected $method $path")
                }
            }
        state.load()
        runCurrent()
        assertEquals(notLinked, state.state)
        state.link { }
        runCurrent()
        // ブラウザーへ移動している間は待ち状態
        assertTrue(state.pending)
        // ブラウザーで連携を完了してアプリに戻ると、状態を読み直し、待ち状態も解除する
        linkedNow = true
        state.resume()
        runCurrent()
        assertFalse(state.pending)
        assertEquals(linked, state.state)
        state.close()
    }

    @Test
    fun pluralKeysFollowTheCount() {
        assertEquals("slackLink.importSummary_one", pluralKey("slackLink.importSummary", 1))
        assertEquals("slackLink.importSummary_other", pluralKey("slackLink.importSummary", 0))
        assertEquals("slackLink.importSummary_other", pluralKey("slackLink.importSummary", 3))
    }

    @Test
    fun resumingDoesNotCancelAnUnlinkThatIsStillRunning() = runTest {
        val unlink = CompletableDeferred<JsonElement>()
        val state =
            SlackLinkController(backgroundScope) { _, method, _ ->
                if (method == "DELETE") unlink.await() else linked
            }
        state.load()
        runCurrent()
        state.unlink { }
        runCurrent()
        assertTrue(state.pending)
        // 連携のためのブラウザー移動ではないので、戻ってきても解除の待ちは消さない
        state.resume()
        runCurrent()
        assertTrue(state.pending)
        unlink.complete(notLinked)
        runCurrent()
        assertFalse(state.pending)
        state.close()
    }

    @Test
    fun resumingDoesNotCancelTheCompletionOfAnAuthorizedLink() = runTest {
        val complete = CompletableDeferred<JsonElement>()
        val state =
            SlackLinkController(backgroundScope) { path, method, _ ->
                when {
                    path == "/api/slack/oauth/start" ->
                        obj("authorize_url" to "https://slack.com/oauth/v2/authorize?state=x")
                    path == "/api/slack/oauth/complete" -> complete.await()
                    method == "GET" -> notLinked
                    else -> error("unexpected $method $path")
                }
            }
        state.link { }
        runCurrent()
        // 認可の結果がアプリに渡り、連携の完了を待っている間は、アプリに戻っても待ちを解除しない
        state.handle(SlackLinkResult.Authorized("code", "state"))
        runCurrent()
        assertTrue(state.pending)
        state.resume()
        runCurrent()
        assertTrue(state.pending)
        complete.complete(linked)
        runCurrent()
        assertFalse(state.pending)
        assertEquals(linked, state.state)
        state.close()
    }
}
