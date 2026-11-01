package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

@OptIn(ExperimentalCoroutinesApi::class)
class SlackImportTest {
    private fun job(id: String, status: String, vararg extra: Pair<String, Any?>): Row =
        obj("id" to id, "status" to status, "file_name" to "$id.zip", *extra)

    @Test
    fun mappingsStartSkippedAndFollowPreviousTargets() {
        val general =
            obj(
                "slack_channel_id" to "C1",
                "suggested_name" to "slack-general",
                "is_private" to true,
            )
        val random =
            obj(
                "slack_channel_id" to "C2",
                "existing_channel" to obj("id" to "channel-random", "name" to "random"),
            )
        assertEquals(
            listOf(
                obj("slack_channel_id" to "C1", "action" to "skip"),
                obj("slack_channel_id" to "C2", "action" to "skip"),
            ),
            slackImportDefaultMappings(listOf(general, random)),
        )
        val targets = listOf(obj("id" to "channel-other"))
        assertEquals(
            obj(
                "slack_channel_id" to "C1",
                "action" to "create",
                "name" to "slack-general",
                "is_private" to true,
            ),
            slackImportMappingFor(general, "create", targets),
        )
        assertEquals(
            "channel-random",
            slackImportMappingFor(random, "existing", targets).text("channel_id"),
        )
        assertEquals(
            "channel-other",
            slackImportMappingFor(general, "existing", targets).text("channel_id"),
        )
    }

    @Test
    fun retryableFilesCountOnlyCompletedJobs() {
        assertEquals(
            5,
            slackImportRetryableFiles(
                job("a", "completed", "pending_files" to 3, "failed_files" to 2)
            ),
        )
        assertEquals(0, slackImportRetryableFiles(job("b", "importing", "pending_files" to 3)))
    }

    @Test
    fun savedExportsAndUnfinishedImportsStayOperable() {
        val (cards, history) =
            splitSlackImports(
                listOf(
                    job("run-b", "importing", "source_id" to "export"),
                    job("run-a", "failed", "source_id" to "export"),
                    job("export", "ready"),
                    job("run-z", "completed", "source_id" to "export"),
                    job("old", "cancelled"),
                )
            )
        assertEquals(listOf("export", "run-b", "run-a"), cards.map { it.text("id") })
        assertEquals(listOf("run-z", "old"), history.map { it.text("id") })
        val (latest, rest) =
            splitSlackImports(
                listOf(
                    job("export", "ready"),
                    job("run-b", "completed", "source_id" to "export"),
                    job("run-a", "completed", "source_id" to "export"),
                )
            )
        assertEquals(listOf("export", "run-b"), latest.map { it.text("id") })
        assertEquals(listOf("run-a"), rest.map { it.text("id") })
    }

    @Test
    fun startAddsAnImportAndKeepsTheExport() = runTest {
        var startedOnServer = false
        val requests = mutableListOf<Pair<String, String>>()
        val state =
            SlackImportController(backgroundScope) { path, method, _ ->
                requests += method to path
                when (path) {
                    "/api/admin/slack-imports" ->
                        obj(
                            "imports" to
                                JsonArray(
                                    listOfNotNull(
                                        job("run", "importing", "source_id" to "new").takeIf {
                                            startedOnServer
                                        },
                                        job("new", "ready"),
                                        job("old", "cancelled"),
                                    )
                                )
                        )
                    else -> {
                        startedOnServer = true
                        job("run", "importing", "source_id" to "new")
                    }
                }
            }
        state.load()
        runCurrent()
        assertEquals(listOf("new"), state.cards.map { it.text("id") })
        assertEquals(listOf("old"), state.history.map { it.text("id") })
        assertFalse(state.busy, "a saved ZIP does not block the next upload")
        state.start("new", listOf(obj("slack_channel_id" to "C1", "action" to "create")))
        runCurrent()
        assertTrue("POST" to "/api/admin/slack-imports/new/start" in requests)
        assertEquals("GET" to "/api/admin/slack-imports", requests.last())
        assertEquals(listOf("new", "run"), state.cards.map { it.text("id") })
        assertEquals(1, state.started)
        assertTrue(state.running)
        state.close()
    }

    @Test
    fun failuresKeepTheServerMessageAndForbiddenStatus() = runTest {
        var status = 409
        val state =
            SlackImportController(backgroundScope) { _, _, _ ->
                throw ApiException(status, obj("error" to "Channel name 'random' is already taken"))
            }
        state.load()
        runCurrent()
        assertEquals(
            SlackImportFailure(409, "Channel name 'random' is already taken"),
            state.failure,
        )
        assertNull(state.imports)
        status = 403
        state.fetchFiles("x")
        runCurrent()
        assertEquals(403, state.failure?.status)
        assertFalse(state.pending)
        state.close()
    }

    @Test
    fun aRejectedActionReloadsTheCurrentState() = runTest {
        val requests = mutableListOf<String>()
        val state =
            SlackImportController(backgroundScope) { path, method, _ ->
                requests += "$method $path"
                if (method == "POST")
                    throw ApiException(409, obj("error" to "The export is not waiting for channel mappings"))
                obj("imports" to JsonArray(listOf(job("new", "importing"))))
            }
        state.start("new", listOf(obj("slack_channel_id" to "C1", "action" to "skip")))
        runCurrent()
        assertEquals(
            listOf("POST /api/admin/slack-imports/new/start", "GET /api/admin/slack-imports"),
            requests,
        )
        assertEquals("importing", state.cards.single().text("status"))
        assertEquals(409, state.failure?.status)
        state.close()
    }

    @Test
    fun uploadCreatesThePutsAndAsksForAnalysis() = runTest {
        val requests = mutableListOf<String>()
        val puts = mutableListOf<String>()
        val state =
            SlackImportController(backgroundScope) { path, method, body ->
                requests += "$method $path"
                when (path) {
                    "/api/admin/slack-imports" ->
                        if (method == "POST") {
                            assertEquals("export.zip", body?.jsonObject?.text("file_name"))
                            obj(
                                "import" to job("new", "uploading"),
                                "upload_url" to "https://storage.example/put",
                            )
                        } else obj("imports" to JsonArray(emptyList()))
                    else -> job("new", "analyzing")
                }
            }
        state.upload("export.zip", 10) { url -> puts += url }
        assertEquals(listOf("https://storage.example/put"), puts)
        assertEquals(
            listOf("POST /api/admin/slack-imports", "POST /api/admin/slack-imports/new/uploaded"),
            requests,
        )
        assertEquals("analyzing", state.cards.single().text("status"))
        assertNull(state.uploading)
        state.close()
    }
}
