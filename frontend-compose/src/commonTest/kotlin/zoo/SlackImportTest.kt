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
    fun defaultMappingsFollowPreviousAndDeletedTargets() {
        val mappings =
            slackImportDefaultMappings(
                listOf(
                    obj(
                        "slack_channel_id" to "C1",
                        "suggested_name" to "slack-general",
                        "is_private" to false,
                    ),
                    obj(
                        "slack_channel_id" to "C2",
                        "existing_channel" to obj("id" to "channel-random", "name" to "random"),
                    ),
                    obj("slack_channel_id" to "C3", "deleted_channel" to "old-design"),
                )
            )
        assertEquals(
            listOf(
                obj(
                    "slack_channel_id" to "C1",
                    "action" to "create",
                    "name" to "slack-general",
                    "is_private" to false,
                ),
                obj(
                    "slack_channel_id" to "C2",
                    "action" to "existing",
                    "channel_id" to "channel-random",
                ),
                obj("slack_channel_id" to "C3", "action" to "skip"),
            ),
            mappings,
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
    fun everyUnfinishedImportStaysOperable() {
        val (cards, history) =
            splitSlackImports(
                listOf(
                    job("b", "awaiting_mapping"),
                    job("a", "awaiting_mapping"),
                    job("z", "completed"),
                )
            )
        assertEquals(listOf("b", "a"), cards.map { it.text("id") })
        assertEquals(listOf("z"), history.map { it.text("id") })
        val (latest, rest) =
            splitSlackImports(
                listOf(job("c", "cancelled"), job("b", "completed"), job("a", "completed"))
            )
        assertEquals(listOf("b"), latest.map { it.text("id") })
        assertEquals(listOf("c", "a"), rest.map { it.text("id") })
    }

    @Test
    fun loadKeepsActiveAndHistoryAndStartReplacesTheJob() = runTest {
        val requests = mutableListOf<Pair<String, String>>()
        val state =
            SlackImportController(backgroundScope) { path, method, _ ->
                requests += method to path
                when (path) {
                    "/api/admin/slack-imports" ->
                        obj(
                            "imports" to
                                JsonArray(
                                    listOf(
                                        job("new", "awaiting_mapping"),
                                        job("old", "cancelled"),
                                    )
                                )
                        )
                    else -> job("new", "importing")
                }
            }
        state.load()
        runCurrent()
        assertEquals(listOf("new"), state.cards.map { it.text("id") })
        assertEquals(listOf("old"), state.history.map { it.text("id") })
        assertTrue(state.busy, "a new ZIP waits until the mapping is decided")
        state.start("new", listOf(obj("slack_channel_id" to "C1", "action" to "skip")))
        runCurrent()
        assertEquals("POST" to "/api/admin/slack-imports/new/start", requests.last())
        assertEquals("importing", state.cards.single().text("status"))
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
